use anyhow::Result;
use omni_engine::{AudioEngine, EngineCommand};
use crossbeam_channel::{unbounded, Sender, Receiver};
use eframe::egui;
use omni_shared::project::{Project};

#[derive(Clone)]
pub struct ClipData {
    pub notes: Vec<omni_shared::project::Note>, 
    pub color: egui::Color32,
    pub length: f64,
}

impl Default for ClipData {
    fn default() -> Self {
        Self {
            notes: Vec::new(),
            color: egui::Color32::from_gray(60),
            length: 4.0,
        }
    }
}

pub struct TrackData {
    pub name: String,
    pub mute: bool,
    pub volume: f32,
    pub pan: f32,
    pub clips: Vec<ClipData>,
    pub active_clip: Option<usize>,
    pub trigger_flash: f32,
}

impl Default for TrackData {
    fn default() -> Self {
        Self {
            name: "New Track".to_string(),
            mute: false,
            volume: 1.0,
            pan: 0.0,
            clips: vec![ClipData::default(); 8], // 8 Scenes
            active_clip: None,
            trigger_flash: 0.0,
        }
    }
}

// --- Custom Widgets ---
fn knob_ui(ui: &mut egui::Ui, value: &mut f32, range: std::ops::RangeInclusive<f32>) -> egui::Response {
    let desired_size = egui::vec2(30.0, 30.0);
    let (rect, mut response) = ui.allocate_exact_size(desired_size, egui::Sense::drag());

    if response.dragged() {
        // Standard DAW behavior: Drag up to increase, down to decrease.
        // Also allow horizontal: Right to increase.
        let delta = response.drag_delta().x + -response.drag_delta().y; 
        let speed = 0.005; // Slower speed for precision
        *value = (*value + delta * speed).clamp(*range.start(), *range.end());
        response.mark_changed();
    }

    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact_selectable(&response, true);
        let center = rect.center();
        let radius = rect.width() / 2.0;

        // Draw Background Arc
        ui.painter().circle(center, radius, visuals.bg_fill, visuals.bg_stroke);

        // Draw Active Arc
        let start_angle = -135.0f32.to_radians();
        let end_angle = 135.0f32.to_radians();
        let normalized = egui::remap_clamp(*value, *range.start()..=*range.end(), 0.0..=1.0);
        let current_angle = egui::lerp(start_angle..=end_angle, normalized);
        
        // Indicator Line
        let indicator_len = radius * 0.8;
        let indicator_pos = center + egui::Vec2::new(current_angle.sin(), -current_angle.cos()) * indicator_len;
        
        ui.painter().line_segment([center, indicator_pos], (2.0, visuals.fg_stroke.color));
    }
    
    response
}

// #[derive(serde::Deserialize, serde::Serialize)]
// #[serde(default)] // Disabled for now to fix channel initialization issues
pub struct OmniApp {
    is_playing: bool,
    master_volume: f32,
    messenger: Sender<EngineCommand>,
    _receiver: Option<Receiver<EngineCommand>>,
    engine: Option<AudioEngine>,
    tracks: Vec<TrackData>,
    bpm: f32,
    last_step: usize,
    plugin_params: Vec<omni_shared::ParamInfo>,
    selected_track: usize,
    selected_clip: usize,
    param_states: std::collections::HashMap<u32, f32>,
    
    // Piano Roll State
    piano_roll_scroll_x: f32,
    piano_roll_scroll_y: f32,
    piano_roll_zoom_x: f32,
    piano_roll_zoom_y: f32,
    
    // Playback State
    current_step: u32,
    
    // Interaction State
    drag_original_note: Option<omni_shared::project::Note>,
    drag_accumulated_delta: egui::Vec2,
}

impl OmniApp {
    fn new(tx: Sender<EngineCommand>, rx: Receiver<EngineCommand>) -> Self {
        let tracks = Vec::new();
        
        // Initialize Engine Immediately
        // Since we are inside new(), we need to handle Result.
        // But new() returns Self, panic or wrap?
        // OmniApp::new takes tx/rx for commands.
        // Actually, in main() we create tx/rx.
        // We need to create the engine here using the rx passed in?
        // Wait, main passes rx to new.
        // So we can init engine here.
        
        let engine = match AudioEngine::new(rx) {
            Ok(e) => Some(e),
            Err(e) => {
                eprintln!("Failed to init engine: {}", e);
                None
            }
        };

        Self {
            is_playing: false,
            master_volume: 0.1,
            messenger: tx,
            _receiver: None, // Taken by engine
            engine,
            tracks,
            bpm: 120.0,
            last_step: 0,
            plugin_params: Vec::new(),
            selected_track: 0,
            selected_clip: 0,
            param_states: std::collections::HashMap::new(),
            
            // Piano Roll State
            piano_roll_scroll_x: 0.0,
            piano_roll_scroll_y: 60.0 * 20.0, // Center roughly on C3
            piano_roll_zoom_x: 50.0, // Pixels per beat
            piano_roll_zoom_y: 20.0, // Pixels per note
            
            current_step: 0,
            
            drag_original_note: None,
            drag_accumulated_delta: egui::Vec2::ZERO,
        }
    }

    fn load_project(&mut self, path: String) {
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(shared_proj) = serde_json::from_str::<Project>(&content) {
                if let Some(ref _engine) = self.engine {
                     // 1. Reset Engine Graph
                     let _ = self.messenger.send(EngineCommand::ResetGraph);
                     
                     // 2. Clear local UI state
                     self.tracks.clear();
                     self.bpm = shared_proj.bpm;
                     let _ = self.messenger.send(EngineCommand::SetBpm(self.bpm));
                     self.selected_track = 0;
                     self.selected_clip = 0;
                     self.param_states.clear();
                     
                     // 3. Rebuild
                     for (t_idx, shared_track) in shared_proj.tracks.iter().enumerate() {
                         // A. UI Sync
                         let mut local_track = TrackData {
                             name: shared_track.name.clone(),
                             volume: shared_track.volume,
                             pan: shared_track.pan,
                             mute: shared_track.mute,
                             active_clip: shared_track.active_clip_index,
                             ..Default::default()
                         };
                         for (c_idx, shared_clip) in shared_track.clips.iter().enumerate() {
                             if c_idx < local_track.clips.len() {
                                 local_track.clips[c_idx].notes = shared_clip.notes.clone();
                                 local_track.clips[c_idx].length = shared_clip.length;
                             }
                         }
                         self.tracks.push(local_track);

                         // B. Engine Sync (Graph)
                         let plugin_path = if shared_track.plugin_path.is_empty() { None } else { Some(shared_track.plugin_path.clone()) };
                         let _ = self.messenger.send(EngineCommand::AddTrack { plugin_path });
                         
                         // C. Restore Parameters
                         for (&p_id, &val) in &shared_track.parameters {
                             let _ = self.messenger.send(EngineCommand::SetPluginParam { track_index: t_idx, id: p_id, value: val });
                         }

                         // D. Restore Clip State
                         if let Some(c_idx) = shared_track.active_clip_index {
                             let _ = self.messenger.send(EngineCommand::TriggerClip { track_index: t_idx, clip_index: c_idx });
                         }
                     }
                     
                     // 4. Force Update Engine's internal Project state
                     let _ = self.messenger.send(EngineCommand::LoadProject(path));
                }
            }
        }
    }
}

impl eframe::App for OmniApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let current_step = if let Some(ref engine) = self.engine {
            if self.is_playing {
                ctx.request_repaint(); // Keep UI responsive for playhead
                engine.get_current_step() as usize
            } else {
                0
            }
        } else {
            0
        };
        
        // Sync to struct for Piano Roll
        self.current_step = current_step as u32;

        // Handle trigger flashes
        if self.is_playing && current_step != self.last_step {
            for track in self.tracks.iter_mut() {
                if let Some(active_idx) = track.active_clip {
                    let clip = &track.clips[active_idx];
                    let step_start_beat = current_step as f64 * 0.25;
                    let step_end_beat = step_start_beat + 0.25;
                    
                    // Flash if any note starts in this step
                    if clip.notes.iter().any(|n| n.start >= step_start_beat && n.start < step_end_beat) && !track.mute {
                        track.trigger_flash = 1.0;
                    }
                }
            }
            self.last_step = current_step;
        }
        
        // Decay triggers
        for track in self.tracks.iter_mut() {
            track.trigger_flash = (track.trigger_flash - 0.1).max(0.0);
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Omni DAW");
            ui.add_space(20.0);

            ui.horizontal(|ui| {
                let label = if self.is_playing { "STOP" } else { "PLAY" };
                if ui.button(label).clicked() {
                    self.is_playing = !self.is_playing;
                    let cmd = if self.is_playing { EngineCommand::Play } else { EngineCommand::Stop };
                    let _ = self.messenger.send(cmd);
                }

                ui.add_space(20.0);
                if ui.button("💾 SAVE PROJECT").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("Omni Project", &["json"])
                        .set_file_name("project.json")
                        .save_file() 
                    {
                        if let Some(path_str) = path.to_str() {
                            let _ = self.messenger.send(EngineCommand::SaveProject(path_str.to_string()));
                        }
                    }
                }

                if ui.button("📂 LOAD PROJECT").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("Omni Project", &["json"])
                        .pick_file() 
                    {
                        if let Some(path_str) = path.to_str() {
                            self.load_project(path_str.to_string());
                        }
                    }
                }
            });

            ui.add_space(20.0);
            ui.horizontal(|ui| {
                ui.label("Master:");
                if ui.add(egui::Slider::new(&mut self.master_volume, 0.0..=1.0)).changed() {
                    let _ = self.messenger.send(EngineCommand::SetVolume(self.master_volume));
                }
                
                ui.add_space(20.0);
                ui.label("BPM:");
                if ui.add(egui::Slider::new(&mut self.bpm, 60.0..=200.0)).changed() {
                    let _ = self.messenger.send(EngineCommand::SetBpm(self.bpm));
                }

                ui.add_space(20.0);
                // Loop Length Control
                if let Some(track) = self.tracks.get_mut(self.selected_track) {
                    if let Some(clip_idx) = track.active_clip {
                        let clip = &mut track.clips[clip_idx];
                        ui.label("Len (Beats):");
                        if ui.add(egui::DragValue::new(&mut clip.length).speed(1.0).range(1.0..=128.0)).changed() {
                            let _ = self.messenger.send(EngineCommand::SetClipLength {
                                track_index: self.selected_track,
                                clip_index: clip_idx,
                                length: clip.length
                            });
                            let _ = self.messenger.send(EngineCommand::TriggerClip {
                                track_index: self.selected_track,
                                clip_index: clip_idx,
                            });
                        }
                    }
                }

                ui.add_space(20.0);
                if ui.button("+ Add Track").clicked() {
                    // Open File Dialog to choose plugin
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("CLAP Plugin", &["clap"])
                        .pick_file() 
                    {
                        if let Some(path_str) = path.to_str() {
                             eprintln!("[UI] Requesting Add Track: {}", path_str);
                             let _ = self.messenger.send(EngineCommand::AddTrack { plugin_path: Some(path_str.to_string()) });
                             
                             // Sync UI state
                             let name = path.file_stem()
                                .and_then(|s| s.to_str())
                                .unwrap_or("Plugin")
                                .to_string();

                             eprintln!("[UI] Track Added Successfully: {}", name);
                             self.tracks.push(TrackData { 
                                 name, 
                                 active_clip: None, 
                                 ..Default::default() 
                             });
                        } else {
                            eprintln!("[UI] Error: Path is not valid UTF-8");
                        }
                    } 
                }
                
                ui.label(format!("Count: {}", self.tracks.len()));
            });

            ui.add_space(20.0);
            ui.separator();
            ui.add_space(10.0);

            if !self.plugin_params.is_empty() {
                ui.heading("Device View: CLAP Plugin");
                ui.horizontal(|ui| {
                    if ui.button(egui::RichText::new("KILL PLUGIN (TEST)").color(egui::Color32::RED)).clicked() {
                        let _ = self.messenger.send(EngineCommand::SimulateCrash { track_index: 0 });
                    }
                });
                egui::ScrollArea::horizontal()
                    .id_salt("device_view_scroll")
                    .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        // Limit to first 16 params for UI safety in prototype
                        for param in self.plugin_params.iter().take(16) {
                            ui.push_id(param.id, |ui| {
                                ui.group(|ui| {
                                    ui.set_width(100.0);
                                    ui.vertical_centered(|ui| {
                                        ui.label(&param.name);
                                        // Simple detection for boolean params: Stepped + Min 0 + Max 1
                                        let is_stepped = (param.flags & 1) != 0;
                                        let is_bool = is_stepped && param.min_value == 0.0 && param.max_value == 1.0;

                                        // Get current value from local map, fallback to default
                                        let current_val = self.param_states.get(&param.id).copied().unwrap_or(param.default_value as f32);
                                        let mut val = current_val;

                                        if is_bool {
                                            let mut bool_val = val > 0.5;
                                            if ui.checkbox(&mut bool_val, "").changed() {
                                                val = if bool_val { 1.0 } else { 0.0 };
                                                self.param_states.insert(param.id, val); // Update local state
                                                let _ = self.messenger.send(EngineCommand::SetPluginParam { 
                                                    track_index: self.selected_track, // Use selected track!
                                                    id: param.id, 
                                                    value: val 
                                                });
                                            }
                                        } else {
                                            if ui.add(egui::Slider::new(&mut val, param.min_value as f32..=param.max_value as f32).show_value(false)).changed() {
                                                self.param_states.insert(param.id, val); // Update local state
                                                let _ = self.messenger.send(EngineCommand::SetPluginParam { 
                                                    track_index: self.selected_track, 
                                                    id: param.id, 
                                                    value: val 
                                                });
                                            }
                                        }
                                    });
                                });
                            });
                        }
                    });
                });
            }


            ui.add_space(20.0);
            ui.separator();
            ui.add_space(10.0);

            ui.heading("Session Matrix");
            ui.add_space(5.0);
            
            // MATRIX GRID (Cols = Tracks, Rows = Clips)
            egui::ScrollArea::horizontal()
                .id_salt("session_matrix_scroll")
                .show(ui, |ui| {
                ui.horizontal(|ui| {
                    // MASTER SCENE COLUMN
                    ui.vertical(|ui| {
                        ui.heading("Master");
                        ui.add_space(5.0);
                        
                        for scene_idx in 0..8 {
                            let btn_size = egui::vec2(60.0, 30.0); // Match Track Clip height (30.0)
                            let btn = egui::Button::new(format!("Scene {}", scene_idx + 1));
                            
                            // Scene Button
                            if ui.add_sized(btn_size, btn).clicked() {
                                // Trigger this clip index on ALL tracks
                                for (track_idx, track) in self.tracks.iter_mut().enumerate() {
                                    let _ = self.messenger.send(EngineCommand::TriggerClip { 
                                        track_index: track_idx, 
                                        clip_index: scene_idx 
                                    });
                                    // Update local UI state immediately to show feedback
                                    if scene_idx < track.clips.len() {
                                        track.active_clip = Some(scene_idx);
                                    }
                                }
                            }
                        }
                    });

                    ui.separator();

                    // TRACK COLUMNS
                    for (track_idx, track) in self.tracks.iter_mut().enumerate() {
                        ui.push_id(track_idx, |ui| {
                            ui.vertical(|ui| {
                                ui.set_width(90.0); // Fixed width for Compact Layout (appx 4 buttons * 22px)
                            // Track Header
                            ui.label(egui::RichText::new(&track.name).strong());
                            
                            // 1. Clips (Top of Strip) - Matches Master Scene buttons
                            for (clip_idx, clip) in track.clips.iter_mut().enumerate() {
                                let is_active = track.active_clip == Some(clip_idx);
                                let is_selected = self.selected_track == track_idx && self.selected_clip == clip_idx;
                                
                                let mut color = if is_active {
                                    clip.color
                                } else {
                                    egui::Color32::from_gray(40)
                                };
                                
                                if is_selected {
                                    color = egui::Color32::from_rgb(
                                        color.r().saturating_add(50), 
                                        color.g().saturating_add(50), 
                                        color.b().saturating_add(50)
                                    );
                                }

                                let icon = if is_active { "▶" } else { "⏵" };   
                                let btn = egui::Button::new(icon)
                                    .fill(color)
                                    .min_size(egui::vec2(ui.available_width(), 30.0));
                                
                                if ui.add(btn).clicked() {
                                    self.selected_track = track_idx;
                                    self.selected_clip = clip_idx;
                                    track.active_clip = Some(clip_idx);
                                    let _ = self.messenger.send(EngineCommand::TriggerClip { track_index: track_idx, clip_index: clip_idx });
                                }
                            }

                            ui.add_space(10.0);
                            ui.separator();
                            ui.add_space(5.0);

                            // 2. Track Controls (Bottom of Strip)
                            
                            // 2. Track Mixer Strip (Compact)
                            
                            // A. Header Row: Load | GUI | Mute | Stop
                            ui.horizontal(|ui| {
                                let btn_w = (ui.available_width() - 12.0) / 4.0; // 4 buttons, 3 spaces approx
                                let btn_size = egui::vec2(btn_w, 20.0);
                                
                                // Load
                                if ui.add_sized(btn_size, egui::Button::new("📂")).clicked() {
                                    if let Some(path) = rfd::FileDialog::new().add_filter("CLAP", &["clap"]).pick_file() {
                                        if let Some(path_str) = path.to_str() {
                                             let _ = self.messenger.send(EngineCommand::LoadPluginToTrack { track_index: track_idx, plugin_path: path_str.to_string() });
                                             track.name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Plugin").to_string();
                                        }
                                    }
                                }
                                
                                // GUI
                                if ui.add_sized(btn_size, egui::Button::new("GUI")).clicked() {
                                    let _ = self.messenger.send(EngineCommand::OpenPluginEditor { track_index: track_idx });
                                }
                                
                                // Mute
                                let mute_color = if track.mute { egui::Color32::RED } else { egui::Color32::from_gray(60) };
                                if ui.add_sized(btn_size, egui::Button::new("M").fill(mute_color)).clicked() {
                                    track.mute = !track.mute;
                                    let _ = self.messenger.send(EngineCommand::SetMute { track_index: track_idx, muted: track.mute });
                                }

                                // Stop
                                if ui.add_sized(btn_size, egui::Button::new("■")).clicked() {
                                    track.active_clip = None;
                                    let _ = self.messenger.send(EngineCommand::StopTrack { track_index: track_idx });
                                }
                            });

                            ui.add_space(10.0);

                            // B. Knobs Row: Volume | Pan
                            ui.horizontal(|ui| {
                                let col_w = (ui.available_width() - 4.0) / 2.0;
                                
                                // Vol Col
                                ui.vertical(|ui| {
                                    ui.set_width(col_w);
                                    ui.label(egui::RichText::new("Vol").small().weak());
                                    ui.horizontal(|ui| {
                                        ui.add_space((col_w - 30.0) / 2.0); // Center knob
                                        if knob_ui(ui, &mut track.volume, 0.0..=1.0).changed() {
                                            let _ = self.messenger.send(EngineCommand::SetTrackVolume { track_index: track_idx, volume: track.volume });
                                        }
                                    });
                                    let db = if track.volume > 0.0 { 20.0 * track.volume.log10() } else { -144.0 };
                                    ui.label(egui::RichText::new(format!("{:.1}dB", db)).small());
                                });

                                // Pan Col
                                ui.vertical(|ui| {
                                    ui.set_width(col_w);
                                    ui.label(egui::RichText::new("Pan").small().weak());
                                    ui.horizontal(|ui| {
                                        ui.add_space((col_w - 30.0) / 2.0); // Center knob
                                        if knob_ui(ui, &mut track.pan, -1.0..=1.0).changed() {
                                            let _ = self.messenger.send(EngineCommand::SetTrackPan { track_index: track_idx, pan: track.pan });
                                        }
                                    });
                                    ui.label(egui::RichText::new(format!("{:.2}", track.pan)).small());
                                });
                            });

                            ui.add_space(10.0);
                        }); // Close ui.vertical
                        }); // Close ui.push_id
                        
                        ui.add_space(4.0);
                        ui.separator();
                        ui.add_space(4.0);
                    }
                }); // Close ui.horizontal (Tracks Container)
            }); // Close ScrollArea

            ui.add_space(20.0);
            ui.separator();
            ui.add_space(10.0);


            // PIANO ROLL EDITOR
            if self.selected_track < self.tracks.len() {
                let track_name = self.tracks[self.selected_track].name.clone();
                if self.selected_clip < self.tracks[self.selected_track].clips.len() {
                    let clip = &mut self.tracks[self.selected_track].clips[self.selected_clip];
                    
                    ui.heading(format!("Piano Roll: {} - Clip {}", track_name, self.selected_clip));
                    ui.label("Controls: [LMB] Add Note | [RMB] Delete | [MMB/Mwheel] Pan | [Ctrl+Wheel] Zoom");

                    // 1. Allocate Canvas
                    let available_size = ui.available_size();
                    let inner_response = ui.allocate_ui(
                        available_size, // Take remaining space
                        |ui| ui.max_rect()
                    );
                    let mut rect = inner_response.inner;
                    let response = inner_response.response;
                    
                    // Force a minimum height if available size is small (e.g. initial layout)
                    if rect.height() < 200.0 {
                        rect = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), 300.0));
                    }
                    
                    let painter = ui.painter_at(rect);
                    
                    // 2. Input Handling (Navigation)
                    let (scroll_delta, modifiers) = ui.input(|i| (i.raw_scroll_delta, i.modifiers));
                    
                    // Zoom (Ctrl + Scroll)
                    if modifiers.ctrl {
                        if scroll_delta.y != 0.0 {
                            self.piano_roll_zoom_x = (self.piano_roll_zoom_x + scroll_delta.y * 0.1).clamp(10.0, 200.0);
                        }
                    } else {
                        // Pan (Scroll Wheel or Middle Drag)
                         if scroll_delta.x != 0.0 || scroll_delta.y != 0.0 {
                            self.piano_roll_scroll_x -= scroll_delta.x;
                            self.piano_roll_scroll_y -= scroll_delta.y; 
                        }
                    }
                    if response.dragged_by(egui::PointerButton::Middle) {
                         self.piano_roll_scroll_x -= response.drag_delta().x;
                         self.piano_roll_scroll_y -= response.drag_delta().y;
                    }

                    // Clip Canvas
                    let painter = painter.with_clip_rect(rect);
                    
                    // 3. Draw Background (Time Grid)
                    let beat_width = self.piano_roll_zoom_x;
                    let start_beat = (self.piano_roll_scroll_x / beat_width).max(0.0);
                    let end_beat = start_beat + (rect.width() / beat_width);
                    
                    // Draw Beats
                    for b in (start_beat as usize)..(end_beat as usize + 1) {
                        let x = rect.left() + (b as f32 * beat_width) - self.piano_roll_scroll_x;
                        if x >= rect.left() && x <= rect.right() {
                            let color = if b % 4 == 0 { egui::Color32::from_gray(80) } else { egui::Color32::from_gray(40) };
                            painter.line_segment([egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())], (1.0, color));
                        }
                    }
                    
                    // Visualize Loop End & Handle Interaction
                    let loop_x = rect.left() + (clip.length as f32 * beat_width) - self.piano_roll_scroll_x;
                    
                    // Interaction Layer for Loop Marker
                    // We want a hit area slightly wider than the line for easier grabbing
                    let marker_hit_width = 10.0;
                    if loop_x > rect.left() - marker_hit_width && loop_x < rect.right() + marker_hit_width {
                        let marker_rect = egui::Rect::from_min_size(
                            egui::pos2(loop_x - marker_hit_width/2.0, rect.top()), 
                            egui::vec2(marker_hit_width, rect.height())
                        );
                        
                        let marker_response = ui.allocate_rect(marker_rect, egui::Sense::drag());
                        
                        // Cursor
                        if marker_response.hovered() || marker_response.dragged() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                        }

                        // Drag Logic
                        if marker_response.dragged() {
                             let delta_beats = marker_response.drag_delta().x / beat_width;
                             clip.length = (clip.length + delta_beats as f64).max(1.0); // Min length 1 beat
                             
                             // Snap (Shift to disable)
                             if !ui.input(|i| i.modifiers.shift) {
                                 let snap = 1.0; // Snap to whole beats for loop length usually makes sense
                                 clip.length = (clip.length / snap).round() * snap;
                                 if clip.length < snap { clip.length = snap; }
                             }
                             
                             // Send update immediately for responsive feel
                             let _ = self.messenger.send(EngineCommand::SetClipLength {
                                 track_index: self.selected_track,
                                 clip_index: self.selected_clip,
                                 length: clip.length
                             });
                             // Auto-trigger to ensure we hear the change
                             let _ = self.messenger.send(EngineCommand::TriggerClip {
                                 track_index: self.selected_track,
                                 clip_index: self.selected_clip,
                             });
                        }
                    }

                    // Draw Loop Visuals (Background dimming + Line)
                    // Re-calculate loop_x based on potentially updated clip.length
                    let draw_loop_x = rect.left() + (clip.length as f32 * beat_width) - self.piano_roll_scroll_x;
                    
                    if draw_loop_x < rect.right() {
                        // Dimmed area outside loop
                        painter.rect_filled(
                            egui::Rect::from_min_size(
                                egui::pos2(draw_loop_x, rect.top()), 
                                egui::vec2(rect.right() - draw_loop_x, rect.height())
                            ),
                            0.0,
                            egui::Color32::from_rgba_premultiplied(0, 0, 0, 150)
                        );
                        // Loop Line
                        painter.line_segment(
                            [egui::pos2(draw_loop_x, rect.top()), egui::pos2(draw_loop_x, rect.bottom())],
                            (2.0, egui::Color32::YELLOW)
                        );
                        
                        // Label
                        painter.text(
                            egui::pos2(draw_loop_x + 5.0, rect.top() + 10.0),
                            egui::Align2::LEFT_TOP,
                            "LOOP END",
                            egui::FontId::proportional(10.0),
                            egui::Color32::YELLOW
                        );
                    }

                    // 4. Draw Background (Pitch Grid)
                    let note_height = self.piano_roll_zoom_y;
                    // Y=0 is MIDI 127. Y=max is MIDI 0.
                    
                    for note in 0..128 {
                        let y = rect.top() + ((127 - note) as f32 * note_height) - self.piano_roll_scroll_y;
                        
                        if y >= rect.top() - note_height && y <= rect.bottom() {
                             // Black keys background
                             let is_black = matches!(note % 12, 1 | 3 | 6 | 8 | 10);
                             if is_black {
                                 painter.rect_filled(
                                     egui::Rect::from_min_size(egui::pos2(rect.left(), y), egui::vec2(rect.width(), note_height)),
                                     0.0,
                                     egui::Color32::from_rgba_premultiplied(30, 30, 30, 100)
                                 );
                             }
                             painter.line_segment([egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)], (1.0, egui::Color32::from_gray(30)));
                             
                             // Label C notes
                             if note % 12 == 0 {
                                 painter.text(
                                    egui::pos2(rect.left() + 2.0, y + note_height/2.0),
                                    egui::Align2::LEFT_CENTER,
                                    format!("C{}", note / 12 - 2),
                                    egui::FontId::proportional(10.0),
                                    egui::Color32::GRAY
                                 );
                             }
                        }
                    }
                    
                    // 5. Draw Notes & Handle Interactions
                    // We need to collect actions to avoid borrowing conflicts
                    let mut note_actions = Vec::new(); // (ActionType, NoteIdx, NewNoteData)
                    // ActionType: 0 = Move, 1 = Resize, 2 = Delete
                    
                    for (idx, note) in clip.notes.iter_mut().enumerate() {
                        let x = rect.left() + (note.start as f32 * beat_width) - self.piano_roll_scroll_x;
                        let y = rect.top() + ((127 - note.key) as f32 * note_height) - self.piano_roll_scroll_y;
                        let w = note.duration as f32 * beat_width;
                        let h = note_height - 1.0;
                        
                        // Culling
                        if x + w > rect.left() && x < rect.right() && y + h > rect.top() && y < rect.bottom() {
                             // Note Rect
                             let note_rect = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, h));
                             
                             // Visuals
                             painter.rect(
                                 note_rect,
                                 2.0,
                                 egui::Color32::from_rgb(100, 200, 255),
                                 egui::Stroke::new(1.0, egui::Color32::WHITE),
                                 egui::StrokeKind::Middle
                             );

                             // Interaction
                             // We use a simplified interaction model: 
                             // Main body = Move
                             // Right edge (last 5px) = Resize
                             
                             let resize_handle_width = 5.0f32.min(w * 0.5);
                             let resize_rect = egui::Rect::from_min_size(
                                 egui::pos2(note_rect.right() - resize_handle_width, y),
                                 egui::vec2(resize_handle_width, h)
                             );
                             
                             let body_rect = egui::Rect::from_min_size(note_rect.min, egui::vec2(w - resize_handle_width, h));

                             // Check Resize First
                             let resize_response = ui.allocate_rect(resize_rect, egui::Sense::drag());
                             if resize_response.hovered() {
                                 ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                             }
                             
                             if resize_response.drag_started() {
                                 self.drag_original_note = Some(note.clone());
                                 self.drag_accumulated_delta = egui::Vec2::ZERO;
                             }
                             
                             if resize_response.dragged() {
                                 self.drag_accumulated_delta += resize_response.drag_delta();
                                 if let Some(orig) = &self.drag_original_note {
                                     let delta_beats = self.drag_accumulated_delta.x / beat_width;
                                     note.duration = (orig.duration + delta_beats as f64).max(0.125); 
                                     
                                     // Optional Snap
                                     if !ui.input(|i| i.modifiers.shift) {
                                         let snap = 0.25;
                                         note.duration = (note.duration / snap).round() * snap;
                                         if note.duration < snap { note.duration = snap; }
                                     }
                                 }
                             }
                             
                             if resize_response.drag_stopped() {
                                 if let Some(orig) = &self.drag_original_note {
                                     // Commit to Engine
                                     let _ = self.messenger.send(EngineCommand::ToggleNote {
                                         track_index: self.selected_track,
                                         clip_index: self.selected_clip,
                                         start: orig.start,
                                         duration: orig.duration,
                                         note: orig.key,
                                     });
                                     let _ = self.messenger.send(EngineCommand::ToggleNote {
                                         track_index: self.selected_track,
                                         clip_index: self.selected_clip,
                                         start: note.start,
                                         duration: note.duration,
                                         note: note.key,
                                     });
                                 }
                                 self.drag_original_note = None;
                             }

                             // Check Move Body
                             if !resize_response.dragged() && !resize_response.hovered() {
                                 let body_response = ui.allocate_rect(body_rect, egui::Sense::click_and_drag());
                                 if body_response.hovered() {
                                     ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                                 }
                                 
                                 if body_response.drag_started() {
                                     self.drag_original_note = Some(note.clone());
                                     self.drag_accumulated_delta = egui::Vec2::ZERO;
                                 }
                                 
                                 if body_response.dragged() {
                                     ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                                     self.drag_accumulated_delta += body_response.drag_delta();
                                     
                                     if let Some(orig) = &self.drag_original_note {
                                         let delta_beats = self.drag_accumulated_delta.x / beat_width;
                                         let delta_keys = -(self.drag_accumulated_delta.y / note_height); // Y inverted
                                         
                                         note.start = (orig.start + delta_beats as f64).max(0.0);
                                         
                                         // Snap Beat
                                         if !ui.input(|i| i.modifiers.shift) {
                                             let snap = 0.25;
                                             note.start = (note.start / snap).round() * snap;
                                         }
                                         
                                         // Snap Key (Integral)
                                         let new_key = (orig.key as f32 + delta_keys).clamp(0.0, 127.0) as u8;
                                         if new_key != note.key {
                                             note.key = new_key;
                                         }
                                     }
                                 }
                                 
                                 if body_response.drag_stopped() {
                                     if let Some(orig) = &self.drag_original_note {
                                         // Update Engine
                                         let _ = self.messenger.send(EngineCommand::ToggleNote {
                                             track_index: self.selected_track,
                                             clip_index: self.selected_clip,
                                             start: orig.start,
                                             duration: orig.duration,
                                             note: orig.key,
                                         });
                                         let _ = self.messenger.send(EngineCommand::ToggleNote {
                                             track_index: self.selected_track,
                                             clip_index: self.selected_clip,
                                             start: note.start,
                                             duration: note.duration,
                                             note: note.key,
                                         });
                                     }
                                     self.drag_original_note = None;
                                 }
                                 
                                 if body_response.secondary_clicked() {
                                     // Delete
                                     note_actions.push((2, idx, note.clone()));
                                 }
                             }
                        }
                    }
                    
                    // Apply One-Shot Actions (Deletes)
                    // We handle Move/Resize in-place above for immediate feedback, 
                    // but Deletes change the Vec structure so must be deferred.
                    for (action, idx, note) in note_actions {
                        if action == 2 {
                             clip.notes.remove(idx);
                             let _ = self.messenger.send(EngineCommand::ToggleNote {
                                 track_index: self.selected_track,
                                 clip_index: self.selected_clip,
                                 start: note.start,
                                 duration: note.duration,
                                 note: note.key,
                             });
                        }
                    }

                    // 6. Interaction: Add Note (Background Click)
                    let pointer_pos = ui.input(|i| i.pointer.interact_pos());
                    if let Some(pos) = pointer_pos {
                        if rect.contains(pos) {
                             if ui.input(|i| i.pointer.primary_clicked()) {
                                 // Check if we clicked on a note? No, we handled that above with allocate_rect!
                                 // allocate_rect consumes the click if it hits a note.
                                 // So if we are here, we clicked BACKGROUND.
                                 
                                let local_x = pos.x - rect.left() + self.piano_roll_scroll_x;
                                let local_y = pos.y - rect.top() + self.piano_roll_scroll_y;
                                
                                let start_exact = local_x as f64 / beat_width as f64;
                                let snap = 0.25;
                                let start_snapped = (start_exact / snap).floor() * snap;
                                
                                let note_idx_raw = 127.0 - (local_y / note_height).floor();
                                let note_idx = note_idx_raw.clamp(0.0, 127.0) as u8;
                                
                                 clip.notes.push(omni_shared::project::Note {
                                     start: start_snapped,
                                     duration: 0.25, 
                                     key: note_idx,
                                     velocity: 100,
                                     selected: false,
                                 });
                                 let _ = self.messenger.send(EngineCommand::ToggleNote {
                                     track_index: self.selected_track,
                                     clip_index: self.selected_clip,
                                     start: start_snapped,
                                     duration: 0.25,
                                     note: note_idx,
                                 });
                             }
                        }
                    }
                    
                    // 7. Draw Playhead
                    if self.is_playing {
                        // Current step is 1/4 steps. 
                        // So beat = step * 0.25
                        // Ideally we want smoother interpolation but step is what we have from engine event.
                        let playhead_beat = self.current_step as f32 * 0.25;
                        let x = rect.left() + (playhead_beat * beat_width) - self.piano_roll_scroll_x;
                        
                        if x >= rect.left() && x <= rect.right() {
                            painter.line_segment(
                                [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                                (2.0, egui::Color32::from_rgb(255, 50, 50))
                            );
                            
                            // Head
                            painter.circle_filled(
                                egui::pos2(x, rect.top() + 5.0),
                                5.0,
                                egui::Color32::from_rgb(255, 50, 50)
                            );
                        }
                    }
                }
            }

            if self.engine.is_none() && self.is_playing {
                ui.colored_label(egui::Color32::RED, "Engine failed to initialize");
            }
        });
    }
}

fn main() -> Result<()> {
    let (tx, rx) = unbounded();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([700.0, 400.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Omni",
        options,
        Box::new(|_cc| {
            Ok(Box::new(OmniApp::new(tx, rx)))
        }),
    ).map_err(|e| anyhow::anyhow!("Eframe error: {}", e))?;

    Ok(())
}
