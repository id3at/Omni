use anyhow::{Result, anyhow};
use omni_engine::{AudioEngine, EngineCommand};
use crossbeam_channel::{unbounded, Sender, Receiver};
use eframe::egui;
use omni_shared::project::{Project, Track as SharedTrack, Clip as SharedClip};
use std::collections::HashMap;

#[derive(Clone)]
pub struct ClipData {
    pub events: Vec<Vec<u8>>, // Step -> List of Notes
    pub color: egui::Color32,
}

impl Default for ClipData {
    fn default() -> Self {
        Self {
            events: vec![vec![]; 16],
            color: egui::Color32::from_gray(60),
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
    receiver: Option<Receiver<EngineCommand>>,
    engine: Option<AudioEngine>,
    tracks: Vec<TrackData>,
    bpm: f32,
    last_step: usize,
    plugin_params: Vec<omni_shared::ParamInfo>,
    selected_track: usize,
    selected_clip: usize,
    param_states: std::collections::HashMap<u32, f32>,
    selected_note: u8,
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
            receiver: None, // Taken by engine
            engine,
            tracks,
            bpm: 120.0,
            last_step: 0,
            plugin_params: Vec::new(),
            selected_track: 0,
            selected_clip: 0,
            param_states: std::collections::HashMap::new(),
            selected_note: 60, // C3
        }
    }

    fn load_project(&mut self, path: String) {
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(shared_proj) = serde_json::from_str::<Project>(&content) {
                if let Some(ref engine) = self.engine {
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
                                 local_track.clips[c_idx].events = shared_clip.notes.clone();
                             }
                         }
                         self.tracks.push(local_track);

                         // B. Engine Sync (Graph)
                         let plugin_path = if shared_track.plugin_path.is_empty() { None } else { Some(shared_track.plugin_path.as_str()) };
                         let _ = engine.add_track(plugin_path);
                         
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

        // Handle trigger flashes
        if self.is_playing && current_step != self.last_step {
            for track in self.tracks.iter_mut() {
                if let Some(active_idx) = track.active_clip {
                    if !track.clips[active_idx].events[current_step].is_empty() && !track.mute {
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
                if ui.button("+ Add Track").clicked() {
                    // Open File Dialog to choose plugin
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("CLAP Plugin", &["clap"])
                        .pick_file() 
                    {
                        if let Some(ref engine) = self.engine {
                            if let Some(path_str) = path.to_str() {
                                 eprintln!("[UI] Requesting Add Track: {}", path_str);
                                 if let Err(e) = engine.add_track(Some(path_str)) {
                                     eprintln!("Failed to add track: {}", e);
                                 } else {
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
                                 }
                            } else {
                                eprintln!("[UI] Error: Path is not valid UTF-8");
                            }
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
                    .id_source("device_view_scroll")
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
                .id_source("session_matrix_scroll")
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
                                    if let Some(ref engine) = self.engine {
                                        if let Some(path) = rfd::FileDialog::new().add_filter("CLAP", &["clap"]).pick_file() {
                                            if let Some(path_str) = path.to_str() {
                                                 let _ = engine.load_plugin_to_track(track_idx, path_str);
                                                 track.name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Plugin").to_string();
                                            }
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

            // PATTERN EDITOR (For Selected Clip)
            if self.selected_track < self.tracks.len() {
                let track_name = self.tracks[self.selected_track].name.clone();
                if self.selected_clip < self.tracks[self.selected_track].clips.len() {
                    let clip = &mut self.tracks[self.selected_track].clips[self.selected_clip];
                    
                    ui.heading(format!("Editor: {} - Clip {}", track_name, self.selected_clip));

                     // Note Selector
                    ui.horizontal(|ui| {
                        ui.label("Paint Note:");
                        ui.add(egui::DragValue::new(&mut self.selected_note).speed(1).range(0..=127));
                        ui.label(format!("(MIDI {})", self.selected_note));
                    });
                    
                    // Render Steps
                    ui.horizontal(|ui| {
                        for step in 0..16 {
                            let is_current = self.is_playing && step == current_step;
                            // Check if selected note is present (or any note?)
                            // For simple view, show filled if ANY note is present.
                            // If we want to show specific note presence, we check contains.
                            let has_notes = !clip.events[step].is_empty();
                            let has_selected_note = clip.events[step].contains(&self.selected_note);
                            
                            let base_color = if step % 4 == 0 { egui::Color32::from_gray(60) } else { egui::Color32::from_gray(40) };
                            
                            // Color logic: 
                            // Blue = Has Selected Note
                            // Dim Blue = Has Other Notes
                            // Grey = Empty
                            let fill_color = if has_selected_note { 
                                egui::Color32::from_rgb(100, 200, 255) 
                            } else if has_notes {
                                egui::Color32::from_rgb(60, 100, 140)
                            } else { 
                                base_color 
                            };
                            
                            let btn_size = egui::vec2(30.0, 40.0);
                            let btn = egui::Button::new("")
                                .fill(fill_color)
                                .stroke(if is_current { egui::Stroke::new(2.0, egui::Color32::WHITE) } else { egui::Stroke::NONE });
                            
                            if ui.add_sized(btn_size, btn).clicked() {
                                // Toggle selected note
                                if has_selected_note {
                                    clip.events[step].retain(|&n| n != self.selected_note);
                                } else {
                                    clip.events[step].push(self.selected_note);
                                }
                                
                                // Send update to engine
                                let _ = self.messenger.send(EngineCommand::ToggleNote { 
                                    track_index: self.selected_track, 
                                    clip_index: self.selected_clip, // Pass explict clip index!
                                    step, 
                                    note: self.selected_note 
                                });
                            }
                        }
                    });
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
