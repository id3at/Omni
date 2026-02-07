use anyhow::Result;
use omni_engine::{AudioEngine, EngineCommand};
use crossbeam_channel::{unbounded, Sender, Receiver};
use eframe::egui;
use omni_shared::project::{Project, StepSequencerData};
mod sequencer_ui;
mod arrangement_ui;
mod project_io;
pub mod ui; // New UI module

use project_io::{load_project_file, save_project_file};
use arrangement_ui::ArrangementUI;
use std::collections::HashMap;

#[derive(Clone)]
pub struct ClipData {
    pub notes: Vec<omni_shared::project::Note>, 
    pub color: egui::Color32,
    pub length: f64,
    pub use_sequencer: bool,
    pub step_sequencer: StepSequencerData,
}

impl Default for ClipData {
    fn default() -> Self {
        Self {
            notes: Vec::new(),
            color: egui::Color32::from_gray(60),
            length: 4.0,
            use_sequencer: false,
            step_sequencer: StepSequencerData::default(),
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
    /// Valid MIDI notes for this track's plugin (None = all 128 notes valid)
    pub valid_notes: Option<Vec<i16>>,
    pub arrangement: omni_shared::project::TrackArrangement,
    pub parameters: HashMap<u32, f32>,
    pub plugin_path: String,
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
            valid_notes: None,
            arrangement: omni_shared::project::TrackArrangement::default(),
            parameters: HashMap::new(),
            plugin_path: String::new(),
        }
    }
}

pub struct OmniApp {
    is_playing: bool,
    is_recording: bool, // Recording Session to Arrangement
    master_volume: f32,
    messenger: Sender<EngineCommand>,
    _receiver: Option<Receiver<EngineCommand>>,
    engine: Option<AudioEngine>,
    tracks: Vec<TrackData>,
    bpm: f32,
    last_step: usize,
    
    // Plugin Params (Transient for selected track)
    plugin_params: Vec<omni_shared::ParamInfo>,
    pending_params_rx: Option<(usize, Receiver<Vec<omni_shared::ParamInfo>>)>,
    
    selected_track: usize,
    last_selected_track: usize, // To detect changes
    
    selected_clip: usize,
    current_step: u32,
    global_sample_pos: u64, // Added for polyrhythmic calculations
    
    // Piano Roll State (Refactored)
    piano_roll_state: ui::piano_roll::PianoRollState,
    selected_sequencer_lane: usize, // 0=Pitch, 1=Vel, etc.

    // Pending note names receiver (for async plugin query)
    // NoteNameInfo is in omni_shared root, not project
    pending_note_names_rx: Option<(usize, Receiver<(String, Vec<omni_shared::NoteNameInfo>)>)>,

    // Learning State
    is_learning: bool,
    last_touched_generation: u32,
    pending_last_touched_rx: Option<Receiver<Option<(u32, f32, u32)>>>,
    
    // Deferred Actions (RefCell to mutate from inside UI closures)
    deferred_track_remove: std::cell::RefCell<Option<usize>>,
    
    // Arrangement Logic
    arrangement_ui: ArrangementUI,
    show_arrangement_view: bool,
}

impl OmniApp {
    fn new(tx: Sender<EngineCommand>, rx: Receiver<EngineCommand>) -> Self {
        let tracks = Vec::new();
        
        let (drop_tx, drop_rx) = crossbeam_channel::unbounded::<Box<dyn omni_engine::nodes::AudioNode>>();
        
        std::thread::Builder::new()
            .name("Omni-GC-Thread".to_string())
            .spawn(move || {
                eprintln!("[GC] Thread started.");
                for node in drop_rx {
                    drop(node); 
                }
                eprintln!("[GC] Thread stopped.");
            })
            .expect("Failed to spawn GC thread");
        
        let engine = match AudioEngine::new(rx, drop_tx) {
            Ok(e) => Some(e),
            Err(e) => {
                eprintln!("Failed to init engine: {}", e);
                None
            }
        };

        Self {
            is_playing: false,
            is_recording: false,
            master_volume: 0.1,
            messenger: tx,
            _receiver: None, // Taken by engine
            engine,
            tracks,
            bpm: 120.0,
            last_step: 0,
            
            plugin_params: Vec::new(),
            pending_params_rx: None,
            
            selected_track: 0,
            last_selected_track: 9999, // Force initial update
            
            selected_clip: 0,
            current_step: 0,
            global_sample_pos: 0,
            
            piano_roll_state: ui::piano_roll::PianoRollState::default(),
            selected_sequencer_lane: 0,
            
            pending_note_names_rx: None,
            is_learning: false,
            last_touched_generation: 0,
            pending_last_touched_rx: None,
            deferred_track_remove: std::cell::RefCell::new(None),
            
            arrangement_ui: ArrangementUI::new(),
            show_arrangement_view: false,
        }
    }

    fn load_project(&mut self, path: String) {
        if let Some(ref engine) = self.engine {
            if let Ok((shared_proj, nodes)) = load_project_file(&path, engine.get_sample_rate() as f64) {
                let _ = self.messenger.send(EngineCommand::LoadProjectState(shared_proj.clone(), nodes));
                    
                self.tracks.clear();
                self.bpm = shared_proj.bpm;
                let _ = self.messenger.send(EngineCommand::SetBpm(self.bpm));
                self.selected_track = 0;
                self.last_selected_track = 9999; // Force refresh
                self.selected_clip = 0;
                    
                for (t_idx, shared_track) in shared_proj.tracks.iter().enumerate() {
                    let mut local_track = TrackData {
                        name: shared_track.name.clone(),
                        volume: shared_track.volume,
                        pan: shared_track.pan,
                        mute: shared_track.mute,
                        active_clip: shared_track.active_clip_index,
                        arrangement: shared_track.arrangement.clone(),
                        valid_notes: None,
                        parameters: shared_track.parameters.clone(),
                        plugin_path: shared_track.plugin_path.clone(),
                        ..Default::default()
                    };
                        
                    for (c_idx, shared_clip) in shared_track.clips.iter().enumerate() {
                        if c_idx < local_track.clips.len() {
                            local_track.clips[c_idx].notes = shared_clip.notes.clone();
                            local_track.clips[c_idx].length = shared_clip.length;
                            local_track.clips[c_idx].use_sequencer = shared_clip.use_sequencer;
                            local_track.clips[c_idx].step_sequencer = shared_clip.step_sequencer.clone();
                        }
                    }
                    self.tracks.push(local_track);

                    if !shared_track.plugin_path.is_empty() {
                        let (tx, rx) = crossbeam_channel::bounded(1);
                        self.pending_note_names_rx = Some((t_idx, rx));
                        let _ = self.messenger.send(EngineCommand::GetNoteNames { track_index: t_idx, response_tx: tx });
                    }
                }
                eprintln!("[UI] Loaded project from: {}", path);
            }
        }
    }
}

impl eframe::App for OmniApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // --- SELECTION CHANGE DETECTION ---
        if self.selected_track != self.last_selected_track {
            self.last_selected_track = self.selected_track;
            // 1. Fetch Params
            let (tx, rx) = unbounded();
            let _ = self.messenger.send(EngineCommand::GetPluginParams { 
                track_index: self.selected_track, 
                response_tx: tx 
            });
            self.pending_params_rx = Some((self.selected_track, rx));
            
            // Note names handled separately elsewhere, but could trigger here too if logic requires (currently valid_notes is persistent in TrackData).
        }
        
        // --- POLL PARAMS ---
        if let Some((track_idx, ref rx)) = self.pending_params_rx {
             if let Ok(params) = rx.try_recv() {
                 eprintln!("[App] Received Params for Track {}: {} params", track_idx, params.len());
                 if track_idx == self.selected_track {
                     self.plugin_params = params;
                 }
                 self.pending_params_rx = None;
             }
        }
        
        // --- DEFERRED ACTIONS ---
        if let Some(track_idx) = self.deferred_track_remove.borrow_mut().take() {
             if track_idx < self.tracks.len() {
                 let _ = self.messenger.send(EngineCommand::RemoveTrack { track_index: track_idx });
                 self.tracks.remove(track_idx);
                 if self.selected_track >= self.tracks.len() && !self.tracks.is_empty() {
                     self.selected_track = self.tracks.len() - 1;
                 }
                 eprintln!("[UI] Deleted Track {}", track_idx);
             }
        }

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
        
        // Update playback state
        if let Some(ref engine) = self.engine {
            self.current_step = engine.get_current_step();
            self.global_sample_pos = engine.get_sample_position() as u64;
        } else {
            self.current_step = 0;
            self.global_sample_pos = 0;
        }

        // Handle trigger flashes
        if self.is_playing && (self.current_step as usize) != self.last_step {
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

        // Poll for parameter learning
        let mut newly_touched_param = None;
        if self.is_learning {
            // 1. Send Request if idle
            if self.pending_last_touched_rx.is_none() {
                let (tx, rx) = crossbeam_channel::bounded(1);
                let _ = self.messenger.send(EngineCommand::GetLastTouchedParam { 
                    track_index: self.selected_track, 
                    response_tx: tx 
                });
                self.pending_last_touched_rx = Some(rx);
            }
            
            // 2. Check Response
            if let Some(ref rx) = self.pending_last_touched_rx {
                 match rx.try_recv() {
                     Ok(Some((id, _val, gen))) => {
                         if gen > self.last_touched_generation {
                             newly_touched_param = Some(id);
                             self.last_touched_generation = gen;
                             eprintln!("[App] Learned Param: ID={}, Gen={}", id, gen);
                         }
                         self.pending_last_touched_rx = None;
                     }
                     Ok(None) => {
                         // No touch data yet
                         self.pending_last_touched_rx = None;
                     }
                     Err(crossbeam_channel::TryRecvError::Empty) => {} // Wait
                     Err(_) => { self.pending_last_touched_rx = None; }
                 }
            }
        }

        // Poll for pending note names response
        if let Some((track_idx, ref rx)) = self.pending_note_names_rx {
            match rx.try_recv() {
                Ok((clap_id, names)) => {
                    eprintln!("[OmniApp Debug] Received response for track {}: clap_id='{}', names.len={}", track_idx, clap_id, names.len());
                    
                    if track_idx < self.tracks.len() {
                        let mut valid_keys = Vec::new();
                        
                        if !names.is_empty() {
                         valid_keys = names.iter()
                            .map(|n| n.key)
                            .filter(|&k| k >= 0)
                            .collect();
                    } else if clap_id == "com.tomic.drum-synth" {
                        eprintln!("[OmniApp] Applying hardcoded map for TOMiC Drum Synth");
                        valid_keys = vec![36, 37, 38, 39, 40, 41, 42, 43];
                    }

                    if !valid_keys.is_empty() {
                        self.tracks[track_idx].valid_notes = Some(valid_keys);
                        eprintln!("[OmniApp] Track {} has {} valid notes", track_idx, self.tracks[track_idx].valid_notes.as_ref().unwrap().len());
                    } else {
                        self.tracks[track_idx].valid_notes = None;
                    }
                }
                self.pending_note_names_rx = None;
            }
            Err(crossbeam_channel::TryRecvError::Empty) => {}
            Err(e) => {
                 eprintln!("[OmniApp Debug] Channel error: {}", e);
                 self.pending_note_names_rx = None;
            }
        }
    }



        // 1. TOP PANEL: Transport & Global Controls
        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.set_height(crate::ui::theme::PANEL_TOP_HEIGHT);
                
                let (play_rect, play_resp) = ui.allocate_exact_size(egui::vec2(crate::ui::theme::BUTTON_WIDTH_SMALL, crate::ui::theme::BUTTON_HEIGHT_SMALL), egui::Sense::click());
                if play_resp.hovered() {
                    ui.painter().rect_filled(play_rect, 2.0, crate::ui::theme::THEME.bg_light);
                }
                
                let icon_color = if self.is_playing { crate::ui::theme::THEME.accent_primary } else { crate::ui::theme::THEME.text_secondary };
                let center = play_rect.center();
                
                if self.is_playing {
                    // Stop Square
                    let size = 10.0;
                    ui.painter().rect_filled(egui::Rect::from_center_size(center, egui::vec2(size, size)), 1.0, icon_color);
                } else {
                    // Play Triangle
                    let size = 10.0;
                    let points = vec![
                        center + egui::vec2(-size * 0.4, -size * 0.5),
                        center + egui::vec2(-size * 0.4, size * 0.5),
                        center + egui::vec2(size * 0.6, 0.0),
                    ];
                    ui.painter().add(egui::Shape::convex_polygon(points, icon_color, egui::Stroke::NONE));
                }

                if play_resp.clicked() {
                    self.is_playing = !self.is_playing;
                    if self.is_playing {
                        let _ = self.messenger.send(EngineCommand::Play);
                    } else {
                        let _ = self.messenger.send(EngineCommand::Stop);
                    }
                }
                
                let rec_color = if self.is_recording { crate::ui::theme::THEME.accent_warn } else { crate::ui::theme::THEME.text_secondary };
                
                let (rec_rect, rec_resp) = ui.allocate_exact_size(egui::vec2(crate::ui::theme::BUTTON_WIDTH_SMALL, crate::ui::theme::BUTTON_HEIGHT_SMALL), egui::Sense::click());
                if rec_resp.hovered() {
                    ui.painter().rect_filled(rec_rect, 2.0, crate::ui::theme::THEME.bg_light);
                }
                
                ui.painter().circle_filled(rec_rect.center(), 6.0, rec_color);

                if rec_resp.clicked() {
                    self.is_recording = !self.is_recording;
                    if self.is_recording {
                        let _ = self.messenger.send(EngineCommand::StartRecording);
                    } else {
                        let (tx, rx) = unbounded(); 
                        let _ = self.messenger.send(EngineCommand::StopRecording { response_tx: tx });
                        
                        // Block and wait for created clips (should be fast)
                        if let Ok(new_clips) = rx.recv() {
                            for (track_idx, clip) in new_clips {
                                if track_idx < self.tracks.len() {
                                    self.tracks[track_idx].arrangement.clips.push(clip);
                                }
                            }
                            // Auto-switch to Arrangement View to show result
                            self.show_arrangement_view = true;
                        }
                    }
                }
                
                ui.separator();
                
                // BPM & Volume
                ui.label("BPM:");
                if ui.add(egui::DragValue::new(&mut self.bpm).range(40.0..=240.0).speed(1.0)).changed() {
                    let _ = self.messenger.send(EngineCommand::SetBpm(self.bpm));
                }
                
                ui.add_space(crate::ui::theme::SPACING_MEDIUM);
                ui.label("Vol:");
                if ui::widgets::knob_ui(ui, &mut self.master_volume, 0.0..=1.0).changed() {
                    let _ = self.messenger.send(EngineCommand::SetVolume(self.master_volume));
                }

                ui.separator();

                // Project Controls - New
                let (new_rect, new_resp) = ui.allocate_exact_size(egui::vec2(crate::ui::theme::BUTTON_WIDTH_SMALL, crate::ui::theme::BUTTON_HEIGHT_SMALL), egui::Sense::click());
                if new_resp.hovered() {
                    ui.painter().rect_filled(new_rect, 2.0, crate::ui::theme::THEME.bg_light);
                }
                let icon_color = crate::ui::theme::THEME.text_secondary;
                let c = new_rect.center();
                // Document shape with folded corner
                let doc_w = 10.0;
                let doc_h = 14.0;
                let fold = 4.0;
                let doc_points = vec![
                    c + egui::vec2(-doc_w/2.0, -doc_h/2.0),
                    c + egui::vec2(doc_w/2.0 - fold, -doc_h/2.0),
                    c + egui::vec2(doc_w/2.0, -doc_h/2.0 + fold),
                    c + egui::vec2(doc_w/2.0, doc_h/2.0),
                    c + egui::vec2(-doc_w/2.0, doc_h/2.0),
                ];
                ui.painter().add(egui::Shape::convex_polygon(doc_points, egui::Color32::TRANSPARENT, egui::Stroke::new(1.5, icon_color)));
                // Fold line
                ui.painter().line_segment([c + egui::vec2(doc_w/2.0 - fold, -doc_h/2.0), c + egui::vec2(doc_w/2.0 - fold, -doc_h/2.0 + fold)], egui::Stroke::new(1.0, icon_color));
                ui.painter().line_segment([c + egui::vec2(doc_w/2.0 - fold, -doc_h/2.0 + fold), c + egui::vec2(doc_w/2.0, -doc_h/2.0 + fold)], egui::Stroke::new(1.0, icon_color));
                
                new_resp.clone().on_hover_text("New Project");
                if new_resp.clicked() {
                     let _ = self.messenger.send(EngineCommand::NewProject);
                     self.tracks.clear();
                     self.tracks.push(TrackData::default());
                     self.selected_track = 0;
                     self.last_selected_track = 9999;
                     self.selected_clip = 0;
                     self.bpm = 120.0;
                     let _ = self.messenger.send(EngineCommand::SetBpm(self.bpm));
                }
                
                // Save Project
                let (save_rect, save_resp) = ui.allocate_exact_size(egui::vec2(crate::ui::theme::BUTTON_WIDTH_SMALL, crate::ui::theme::BUTTON_HEIGHT_SMALL), egui::Sense::click());
                if save_resp.hovered() {
                    ui.painter().rect_filled(save_rect, 2.0, crate::ui::theme::THEME.bg_light);
                }
                let save_icon_color = crate::ui::theme::THEME.text_secondary;
                let sc = save_rect.center();
                // Floppy disk shape
                let floppy_size = 12.0;
                let floppy_rect = egui::Rect::from_center_size(sc, egui::vec2(floppy_size, floppy_size));
                ui.painter().rect_stroke(floppy_rect, 1.0, egui::Stroke::new(1.5, save_icon_color), egui::StrokeKind::Outside);
                // Label slot (top)
                let label_w = 6.0;
                let label_h = 4.0;
                ui.painter().rect_filled(egui::Rect::from_center_size(sc + egui::vec2(0.0, -floppy_size/2.0 + label_h/2.0 + 1.0), egui::vec2(label_w, label_h)), 0.0, save_icon_color);
                // Disk slot (bottom)
                let disk_w = 8.0;
                let disk_h = 5.0;
                ui.painter().rect_stroke(egui::Rect::from_center_size(sc + egui::vec2(0.0, floppy_size/2.0 - disk_h/2.0 - 1.0), egui::vec2(disk_w, disk_h)), 0.0, egui::Stroke::new(1.0, save_icon_color), egui::StrokeKind::Outside);
                
                save_resp.clone().on_hover_text("Save Project");
                if save_resp.clicked() {
                    if let Some(path) = rfd::FileDialog::new().add_filter("Omni Project", &["omni"]).save_file() {
                        let mut path_str = path.to_string_lossy().to_string();
                        if !path_str.ends_with(".omni") {
                            path_str.push_str(".omni");
                        }
                        
                        let mut track_plugin_states = Vec::new();
                        for (i, _track) in self.tracks.iter().enumerate() {
                            let (tx, rx) = unbounded();
                            let _ = self.messenger.send(EngineCommand::GetPluginState { track_index: i, response_tx: tx });
                            if let Ok(state) = rx.recv() { 
                                track_plugin_states.push(state);
                            } else {
                                track_plugin_states.push(None);
                            }
                        }
                        
                        let shared_project = Project {
                            name: "Project".to_string(),
                            bpm: self.bpm,
                            tracks: self.tracks.iter().enumerate().map(|(i, t)| {
                                omni_shared::project::Track {
                                    id: uuid::Uuid::new_v4(),
                                    name: t.name.clone(),
                                    volume: t.volume,
                                    pan: t.pan,
                                    mute: t.mute,
                                    solo: false,
                                    clips: t.clips.iter().map(|c| omni_shared::project::Clip {
                                        name: "Clip".to_string(),
                                        notes: c.notes.clone(),
                                        length: c.length,
                                        color: [c.color.r(), c.color.g(), c.color.b()],
                                        use_sequencer: c.use_sequencer,
                                        step_sequencer: c.step_sequencer.clone(),
                                    }).collect(),
                                    active_clip_index: t.active_clip,
                                    parameters: t.parameters.clone(),
                                    plugin_path: t.plugin_path.clone(),
                                    plugin_state: track_plugin_states[i].clone(),
                                    arrangement: t.arrangement.clone(),
                                }
                            }).collect(),
                            arrangement_mode: false, 
                        };
                        if let Err(e) = save_project_file(&shared_project, &path_str) {
                            eprintln!("Failed to save project: {}", e);
                        }
                    }
                }

                // Load Project
                let (load_rect, load_resp) = ui.allocate_exact_size(egui::vec2(crate::ui::theme::BUTTON_WIDTH_SMALL, crate::ui::theme::BUTTON_HEIGHT_SMALL), egui::Sense::click());
                if load_resp.hovered() {
                    ui.painter().rect_filled(load_rect, 2.0, crate::ui::theme::THEME.bg_light);
                }
                let load_icon_color = crate::ui::theme::THEME.text_secondary;
                let lc = load_rect.center();
                // Folder shape
                let folder_w = 14.0;
                let folder_h = 10.0;
                let tab_w = 5.0;
                let tab_h = 2.0;
                // Main folder body
                ui.painter().rect_stroke(egui::Rect::from_center_size(lc + egui::vec2(0.0, tab_h/2.0), egui::vec2(folder_w, folder_h)), 1.0, egui::Stroke::new(1.5, load_icon_color), egui::StrokeKind::Outside);
                // Tab on top left
                ui.painter().rect_filled(egui::Rect::from_min_size(lc + egui::vec2(-folder_w/2.0, -folder_h/2.0 - tab_h/2.0), egui::vec2(tab_w, tab_h)), 1.0, load_icon_color);
                // Arrow pointing up (load/open)
                let arrow_size = 4.0;
                let arrow_points = vec![
                    lc + egui::vec2(0.0, -arrow_size/2.0),
                    lc + egui::vec2(-arrow_size/2.0, arrow_size/2.0),
                    lc + egui::vec2(arrow_size/2.0, arrow_size/2.0),
                ];
                ui.painter().add(egui::Shape::convex_polygon(arrow_points, load_icon_color, egui::Stroke::NONE));
                
                load_resp.clone().on_hover_text("Load Project");
                if load_resp.clicked() {
                    if let Some(path) = rfd::FileDialog::new().add_filter("Omni Project", &["omni"]).pick_file() {
                        self.load_project(path.to_string_lossy().to_string());
                    }
                }
                
                ui.separator();

                let (view_rect, view_resp) = ui.allocate_exact_size(egui::vec2(crate::ui::theme::BUTTON_WIDTH_SMALL, crate::ui::theme::BUTTON_HEIGHT_SMALL), egui::Sense::click());
                if view_resp.hovered() {
                    ui.painter().rect_filled(view_rect, 2.0, crate::ui::theme::THEME.bg_light);
                }
                let view_icon_color = crate::ui::theme::THEME.text_secondary;
                let center = view_rect.center();
                
                if self.show_arrangement_view {
                    // Draw Session Icon (Vertical Bars |||)
                    let v_bar_w = 2.0;
                    let v_bar_h = 14.0;
                    let v_spacing = 5.0;
                    ui.painter().rect_filled(egui::Rect::from_center_size(center + egui::vec2(-v_spacing, 0.0), egui::vec2(v_bar_w, v_bar_h)), 1.0, view_icon_color);
                    ui.painter().rect_filled(egui::Rect::from_center_size(center, egui::vec2(v_bar_w, v_bar_h)), 1.0, view_icon_color);
                    ui.painter().rect_filled(egui::Rect::from_center_size(center + egui::vec2(v_spacing, 0.0), egui::vec2(v_bar_w, v_bar_h)), 1.0, view_icon_color);
                    
                    view_resp.clone().on_hover_text("Switch to Session View");
                } else {
                    // Draw Arrangement Icon (Horizontal Bars ☰)
                    let h_bar_w = 14.0;
                    let h_bar_h = 2.0;
                    let h_spacing = 5.0;
                    ui.painter().rect_filled(egui::Rect::from_center_size(center + egui::vec2(0.0, -h_spacing), egui::vec2(h_bar_w, h_bar_h)), 1.0, view_icon_color);
                    ui.painter().rect_filled(egui::Rect::from_center_size(center, egui::vec2(h_bar_w, h_bar_h)), 1.0, view_icon_color);
                    ui.painter().rect_filled(egui::Rect::from_center_size(center + egui::vec2(0.0, h_spacing), egui::vec2(h_bar_w, h_bar_h)), 1.0, view_icon_color);
                    
                    view_resp.clone().on_hover_text("Switch to Arrangement View");
                }

                if view_resp.clicked() {
                    self.show_arrangement_view = !self.show_arrangement_view;
                }
            });
        });

        // 2. BOTTOM PANEL: Details (Piano Roll / Devices) - RESIZABLE
        if !self.show_arrangement_view {
            egui::TopBottomPanel::bottom("detail_view")
                .resizable(true)
                .min_height(300.0)
                .max_height(800.0)
                .default_height(400.0)
                .show(ctx, |ui| {
                    // Fixed split: Device collapsible at top, then Piano Roll + Expressions
                    if self.selected_track < self.tracks.len() {
                        let track = &mut self.tracks[self.selected_track];
                        
                        ui.collapsing("Device Parameters", |ui| {
                            ui::device::show_device_view(
                                ui, 
                                &self.plugin_params, 
                                &mut track.parameters, 
                                &self.messenger, 
                                self.selected_track
                            );
                        });
                    }
                     
                    ui.separator();

                    // PIANO ROLL + Expressions: Use remaining space
                    let track_len = self.tracks.len();
                    if self.selected_track < track_len {
                        let clip_len = self.tracks[self.selected_track].clips.len();
                        if self.selected_clip < clip_len {
                            // Expression Lane - FIRST (bottom panel, shown below)
                            let expression_height = 100.0;
                            
                            egui::TopBottomPanel::bottom("expression_lane_inner")
                                .resizable(false)
                                .exact_height(expression_height)
                                .show_inside(ui, |ui| {
                                    ui::note_expressions::show(
                                        ui,
                                        &mut self.tracks,
                                        self.selected_track,
                                        self.selected_clip,
                                        &self.messenger,
                                        &mut self.piano_roll_state,
                                    );
                                });
                            
                            // Piano Roll takes remaining space - reborrow track here
                            egui::CentralPanel::default()
                                .show_inside(ui, |ui| {
                                    let track = &mut self.tracks[self.selected_track];
                                    let clip = &mut track.clips[self.selected_clip];
                                    ui::piano_roll::show_piano_roll(
                                        ui,
                                        clip,
                                        &mut self.piano_roll_state,
                                        &self.messenger,
                                        self.selected_track,
                                        self.selected_clip,
                                        &track.name,
                                        track.valid_notes.as_ref(),
                                        self.is_playing,
                                        self.global_sample_pos,
                                        self.bpm,
                                        if let Some(ref e) = self.engine { e.get_sample_rate() as f32 } else { 44100.0 },
                                        &mut self.selected_sequencer_lane,
                                        newly_touched_param,
                                        &self.plugin_params,
                                        &mut self.is_learning,
                                    );
                                });
                        } else {
                            ui.centered_and_justified(|ui| ui.label("No Clip Selected"));
                        }
                    } else {
                        ui.centered_and_justified(|ui| ui.label("No Track Selected"));
                    }
                });
        }

        // 3. CENTRAL PANEL: Session / Arrangement (Fills rest)
        egui::CentralPanel::default().show(ctx, |ui| {
             if !self.show_arrangement_view {
                 // SESSION MATRIX
                 ui::session::show_matrix(
                     ui, 
                     &mut self.tracks, 
                     &self.messenger, 
                     self.bpm, 
                     self.is_playing, 
                     self.global_sample_pos,
                     if let Some(ref e) = self.engine { e.get_sample_rate() as f32 } else { 44100.0 }, // Fix u32->f32
                     &mut self.selected_track,
                     &mut self.selected_clip,
                     &self.deferred_track_remove,
                     &mut self.pending_note_names_rx,
                 );
                 

                 
             } else {
                 // ARRANGEMENT VIEW
                  self.arrangement_ui.show(
                      ui, 
                      &mut self.tracks, 
                      self.bpm, 
                      &self.messenger, 
                      self.current_step, 
                      self.global_sample_pos, 
                      if let Some(ref e) = self.engine { e.get_sample_rate() as f32 } else { 44100.0 },
                      if let Some(ref e) = self.engine { Some(&e.audio_pool) } else { None },
                  );
             }
        });
    }
}

fn main() -> Result<()> {
    let (tx, rx) = unbounded();
    
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default().with_inner_size([1280.0, 800.0]),
        ..Default::default()
    };
    
    eframe::run_native(
        "Omni Host",
        options,
        Box::new(|_cc| Ok(Box::new(OmniApp::new(tx, rx)))),
    ).map_err(|e| anyhow::anyhow!("Eframe error: {}", e))
}
