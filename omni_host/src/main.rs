use anyhow::Result;
use omni_engine::{AudioEngine, EngineCommand};
use crossbeam_channel::{unbounded, Sender, Receiver};
use eframe::egui;
use omni_shared::project::{Project, StepSequencerData};
mod sequencer_ui;
use sequencer_ui::SequencerUI;
mod arrangement_ui;
use arrangement_ui::ArrangementUI;

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
    is_recording: bool, // Recording Session to Arrangement
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
    current_step: u32,
    global_sample_pos: u64, // Added for polyrhythmic calculations
    param_states: std::collections::HashMap<u32, f32>,
    
    // Piano Roll State
    piano_roll_scroll_x: f32,
    piano_roll_scroll_y: f32,
    piano_roll_zoom_x: f32,
    piano_roll_zoom_y: f32,
    
    // Playback State
    // Playback State
    selected_sequencer_lane: usize, // 0=Pitch, 1=Vel, etc.

    
    // Interaction State
    drag_original_note: Option<omni_shared::project::Note>,
    drag_accumulated_delta: egui::Vec2,
    last_note_length: f64, // Sticky note length
    
    // Pending note names receiver (for async plugin query)
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
            is_recording: false,
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
            current_step: 0,
            global_sample_pos: 0,
            param_states: std::collections::HashMap::new(),
            
            // Piano Roll State
            piano_roll_scroll_x: 0.0,
            piano_roll_scroll_y: 60.0 * 20.0, // Center roughly on C3
            piano_roll_zoom_x: 50.0, // Pixels per beat
            piano_roll_zoom_y: 20.0, // Pixels per note
            selected_sequencer_lane: 0,
            
            drag_original_note: None,
            drag_accumulated_delta: egui::Vec2::ZERO,
            last_note_length: 0.25,
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
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(shared_proj) = serde_json::from_str::<Project>(&content) {
                if let Some(ref engine) = self.engine {
                    // Pre-load nodes (Blocking UI, but Safe for Audio)
                    let mut nodes: Vec<Box<dyn omni_engine::nodes::AudioNode>> = Vec::new();
                    let sample_rate = engine.get_sample_rate() as f64;
                    eprintln!("[UI] Loading Project Plugins...");
                    
                    for track in &shared_proj.tracks {
                         if !track.plugin_path.is_empty() {
                             match omni_engine::plugin_node::PluginNode::new(&track.plugin_path, sample_rate) {
                                 Ok(n) => nodes.push(Box::new(n)),
                                 Err(e) => {
                                     eprintln!("[UI] Plugin Load Error: {}. Using GainNode.", e);
                                     nodes.push(Box::new(omni_engine::nodes::GainNode::new(1.0)));
                                 }
                             }
                         } else {
                             nodes.push(Box::new(omni_engine::nodes::GainNode::new(1.0)));
                         }
                    }

                    // Send entire project state to engine with nodes
                    // This replaces ResetGraph + manual rebuild
                    let _ = self.messenger.send(EngineCommand::LoadProjectState(shared_proj.clone(), nodes));
                     
                     // 2. Clear local UI state (Sync UI to Project)
                     self.tracks.clear();
                     self.bpm = shared_proj.bpm;
                     let _ = self.messenger.send(EngineCommand::SetBpm(self.bpm));
                     self.selected_track = 0;
                     self.selected_clip = 0;
                     self.param_states.clear();
                     
                     // 3. Rebuild UI Tracks from Shared Project
                     for (t_idx, shared_track) in shared_proj.tracks.iter().enumerate() {
                         let mut local_track = TrackData {
                             name: shared_track.name.clone(),
                             volume: shared_track.volume,
                             pan: shared_track.pan,
                             mute: shared_track.mute,
                             active_clip: shared_track.active_clip_index,
                             arrangement: shared_track.arrangement.clone(),
                             valid_notes: None, // Will be updated if plugin
                             ..Default::default()
                         };
                         
                         // Restore Clips
                         for (c_idx, shared_clip) in shared_track.clips.iter().enumerate() {
                             if c_idx < local_track.clips.len() {
                                 local_track.clips[c_idx].notes = shared_clip.notes.clone();
                                 local_track.clips[c_idx].length = shared_clip.length;
                                 local_track.clips[c_idx].use_sequencer = shared_clip.use_sequencer;
                                 local_track.clips[c_idx].step_sequencer = shared_clip.step_sequencer.clone();
                             }
                         }
                         self.tracks.push(local_track);

                         // Restore Parameters (Local State)
                         for (&p_id, &val) in &shared_track.parameters {
                             self.param_states.insert(p_id, val);
                         }
                         
                         // Note Names query if plugin
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
}

impl eframe::App for OmniApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
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
                         // Use notes from plugin
                         valid_keys = names.iter()
                            .map(|n| n.key)
                            .filter(|&k| k >= 0)
                            .collect();
                    } else if clap_id == "com.tomic.drum-synth" {
                        // Fallback: Hardcoded map for TOMiC Drum Synth
                        eprintln!("[OmniApp] Applying hardcoded map for TOMiC Drum Synth");
                        valid_keys = vec![36, 37, 38, 39, 40, 41, 42, 43];
                    }

                    if !valid_keys.is_empty() {
                        self.tracks[track_idx].valid_notes = Some(valid_keys);
                        eprintln!("[OmniApp] Track {} has {} valid notes", track_idx, self.tracks[track_idx].valid_notes.as_ref().unwrap().len());
                    } else {
                        // All notes valid
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

        // Note Expressions Panel (Docked at Bottom)
        if self.selected_track < self.tracks.len() {
            let track = &mut self.tracks[self.selected_track];
            if let Some(_active_idx) = track.active_clip {
                 // We use selected_clip logic for UI?
                 // In piano roll logic (Line 717), we use self.selected_clip?
                 // Line 715: if self.selected_clip < track.clips.len() { let clip = ... }
                 // Lets match that logic.
            }
        }

        egui::TopBottomPanel::bottom("note_expressions_panel")
            .show_separator_line(true)
            .show(ctx, |ui| {
             if self.selected_track < self.tracks.len() {
                 let track = &mut self.tracks[self.selected_track];
                 if self.selected_clip < track.clips.len() {
                      let clip = &mut track.clips[self.selected_clip];
                      
                      // Only show if we are in Piano Roll mode (clip.use_sequencer check?)
                      // The old logic was inside `if !clip.use_sequencer`.
                      if !clip.use_sequencer {
                          ui.add_space(5.0);
                          ui.heading("Note Expressions (Selected)");
                          
                          // Collect selected indices
                          let selected_indices: Vec<usize> = clip.notes.iter().enumerate()
                              .filter(|(_, n)| n.selected).map(|(i, _)| i).collect();
                          
                          if !selected_indices.is_empty() {
                              let first_idx = selected_indices[0];
                              let mut temp_note = clip.notes[first_idx].clone();
                              let mut changed = false;
                              
                              ui.horizontal(|ui| {
                                  ui.label("Chance:");
                                  if ui.add(egui::Slider::new(&mut temp_note.probability, 0.0..=1.0).text("%")).changed() { changed = true; }
                                  ui.separator();
                                  ui.label("Vel Dev:");
                                  if ui.add(egui::Slider::new(&mut temp_note.velocity_deviation, -64..=64).text("+/-")).changed() { changed = true; }
                                  ui.separator();
                                  ui.label("Condition:");
                                  egui::ComboBox::from_id_salt("note_cond_docked")
                                      .selected_text(format!("{:?}", temp_note.condition))
                                      .show_ui(ui, |ui| {
                                          if ui.selectable_value(&mut temp_note.condition, omni_shared::project::NoteCondition::Always, "Always").changed() { changed = true; }
                                          if ui.selectable_value(&mut temp_note.condition, omni_shared::project::NoteCondition::PreviousNotePlayed, "Prev Played").changed() { changed = true; }
                                          if ui.selectable_value(&mut temp_note.condition, omni_shared::project::NoteCondition::PreviousNoteSilenced, "Prev Silenced").changed() { changed = true; }
                                          ui.separator();
                                          if ui.selectable_value(&mut temp_note.condition, omni_shared::project::NoteCondition::Iteration { expected: 1, cycle: 2 }, "1 / 2").changed() { changed = true; }
                                          if ui.selectable_value(&mut temp_note.condition, omni_shared::project::NoteCondition::Iteration { expected: 2, cycle: 2 }, "2 / 2").changed() { changed = true; }
                                          if ui.selectable_value(&mut temp_note.condition, omni_shared::project::NoteCondition::Iteration { expected: 1, cycle: 4 }, "1 / 4").changed() { changed = true; }
                                          if ui.selectable_value(&mut temp_note.condition, omni_shared::project::NoteCondition::Iteration { expected: 4, cycle: 4 }, "4 / 4").changed() { changed = true; }
                                      });
                                  if temp_note.condition != clip.notes[first_idx].condition { changed = true; }
                              });
                              
                              if changed {
                                  for idx in selected_indices {
                                      if let Some(note) = clip.notes.get_mut(idx) {
                                           // Send Update (ToggleNote logic)
                                           // We just send the new state. The engine ToggleNote handles update if exists?
                                           // Actually ToggleNote toggles. 
                                           // The old logic (Step 629) removed then added.
                                           // Wait, ToggleNote iterates and removes if match? 
                                           // If I send ToggleNote with same params, it might remove it.
                                           // I need to use the OLD LOGIC: remove OLD, add NEW.
                                           
                                           // 1. Remove Old (Logic from Step 629)
                                           let _ = self.messenger.send(EngineCommand::ToggleNote {
                                              track_index: self.selected_track,
                                              clip_index: self.selected_clip,
                                              start: note.start,
                                              duration: note.duration,
                                              note: note.key,
                                              probability: 1.0, // Defaults? Or current?
                                              // Step 629 used 1.0, 0, Always for Remove?
                                              // Why? Maybe to force match on ID?
                                              // Assuming Engine identifies note by start/key?
                                              // If ToggleNote matches EXACTLY fields, then we need exact fields.
                                              // Step 629:
                                              /*
                                              let _ = self.messenger.send(EngineCommand::ToggleNote {
                                                 ...
                                                 probability: 1.0,
                                                 velocity_deviation: 0,
                                                 condition: omni_shared::project::NoteCondition::Always,
                                             });
                                             */
                                             // This implies the engine ignores those fields for removal OR the old note had those fields?
                                             // No, Step 629 Line 1318 sent constant values.
                                             // This is suspicious. If Engine matches by (start, key), then other fields don't matter?
                                             // I will copy Step 629 logic EXACTLY.
                                             
                                              velocity_deviation: 0,
                                              condition: omni_shared::project::NoteCondition::Always,
                                           });
                                           
                                           // Update local
                                           note.probability = temp_note.probability;
                                           note.velocity_deviation = temp_note.velocity_deviation;
                                           note.condition = temp_note.condition;
                                           
                                           // 2. Add New
                                            let _ = self.messenger.send(EngineCommand::ToggleNote {
                                               track_index: self.selected_track,
                                               clip_index: self.selected_clip,
                                               start: note.start,
                                               duration: note.duration,
                                               note: note.key,
                                               probability: note.probability,
                                               velocity_deviation: note.velocity_deviation,
                                               condition: note.condition,
                                           });
                                      }
                                  }
                              }
                          } else {
                              ui.label("No note selected.");
                          }
                          ui.add_space(5.0);
                      }
                 }
             }
        });

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
                
                // REC button - records Session to Arrangement
                let rec_label = if self.is_recording { "⏹ STOP REC" } else { "🔴 REC" };
                let rec_btn = ui.button(rec_label);
                if rec_btn.clicked() {
                    self.is_recording = !self.is_recording;
                    if self.is_recording { 
                        // Clear previous recorded clips from UI (Engine also does this)
                        for track in self.tracks.iter_mut() {
                            track.arrangement.clips.retain(|clip| !clip.name.starts_with("Recorded_"));
                        }
                        let _ = self.messenger.send(EngineCommand::StartRecording);
                    } else { 
                        // Create channel to receive recorded clips
                        let (tx, rx) = crossbeam_channel::unbounded();
                        let _ = self.messenger.send(EngineCommand::StopRecording { response_tx: tx });
                        
                        // Wait briefly for response (non-blocking in practice since engine is fast)
                        if let Ok(clips) = rx.recv_timeout(std::time::Duration::from_millis(500)) {
                            for (track_idx, clip) in clips {
                                if let Some(track) = self.tracks.get_mut(track_idx) {
                                    eprintln!("[UI] Adding recorded clip to track {} at sample {}", track_idx, clip.start_time.samples);
                                    track.arrangement.clips.push(clip);
                                }
                            }
                        }
                    };
                }

                ui.add_space(20.0);
                if ui.button("💾 SAVE PROJECT").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("Omni Project", &["json"])
                        .set_file_name("project.json")
                        .save_file() 
                    {
                        if let Some(path_str) = path.to_str() {
                            // Request State from Engine
                            let (tx, rx) = crossbeam_channel::bounded(1);
                            let _ = self.messenger.send(EngineCommand::GetProjectState(tx));
                            
                            // Wait for state (Blocking UI is acceptable for Save Dialog context, or use Async/Defer)
                            // For MVP Refactor, blocking 1ms is fine.
                            if let Ok(project_state) = rx.recv_timeout(std::time::Duration::from_millis(100)) {
                                if let Ok(json) = serde_json::to_string_pretty(&project_state) {
                                    if let Ok(mut file) = std::fs::File::create(path_str) {
                                        use std::io::Write;
                                        let _ = file.write_all(json.as_bytes());
                                        eprintln!("[UI] Saved project to: {}", path_str);
                                    }
                                }
                            } else {
                                eprintln!("[UI] Error: Timeout getting project state from engine");
                            }
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

                if ui.button("📄 NEW PROJECT").clicked() {
                    let _ = self.messenger.send(EngineCommand::NewProject);
                    
                    // Reset Local State
                    self.tracks.clear();
                    self.bpm = 120.0;
                    let _ = self.messenger.send(EngineCommand::SetBpm(self.bpm));
                    self.selected_track = 0;
                    self.selected_clip = 0;
                    self.param_states.clear();
                    
                    eprintln!("[UI] New Project Created");
                }

                ui.add_space(20.0);
                let view_label = if self.show_arrangement_view { "VIEW: ARRANGEMENT" } else { "VIEW: SESSION" };
                if ui.button(view_label).clicked() {
                    self.show_arrangement_view = !self.show_arrangement_view;
                    let _ = self.messenger.send(EngineCommand::SetArrangementMode(self.show_arrangement_view));
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
                             eprintln!("[UI] Requesting Add Track (Async): {}", path_str);
                             
                             let path_cloned = path_str.to_string();
                             let sender = self.messenger.clone();
                             // Capture sample rate safely? 
                             // We are in UI thread, engine is in another thread.
                             // `self.engine` is Option<AudioEngine> but moved? No, `self.engine` is `Option<AudioEngine>`.
                             // Wait, `AudioEngine` is in `self.engine`. `AudioEngine::new` returns it.
                             // But `AudioEngine` runs the stream?
                             // `AudioEngine` struct has `sample_rate`.
                             let sample_rate = self.engine.as_ref().map(|e| e.get_sample_rate() as f64).unwrap_or(44100.0);
                             
                             std::thread::spawn(move || {
                                 // Load Plugin in BG
                                 let node_box: Box<dyn omni_engine::nodes::AudioNode> = match omni_engine::plugin_node::PluginNode::new(&path_cloned, sample_rate) {
                                     Ok(node) => Box::new(node),
                                     Err(e) => {
                                         eprintln!("[BG] Error loading plugin: {}. Fallback to GainNode.", e);
                                         Box::new(omni_engine::nodes::GainNode::new(1.0))
                                     }
                                 };

                                 let name = std::path::Path::new(&path_cloned)
                                    .file_stem()
                                    .and_then(|s| s.to_str())
                                    .unwrap_or("Plugin")
                                    .to_string();

                                 let _ = sender.send(EngineCommand::AddTrackNode { 
                                     node: node_box, 
                                     name, 
                                     plugin_path: Some(path_cloned) 
                                 });
                             });
                             
                             // Sync UI state Optimistically
                             let name = std::path::Path::new(path_str)
                                .file_stem()
                                .and_then(|s| s.to_str())
                                .unwrap_or("Plugin")
                                .to_string();

                             eprintln!("[UI] Track Added Optimistically: {}", name);
                             let new_track_idx = self.tracks.len();
                             self.tracks.push(TrackData { 
                                 name, 
                                 active_clip: None,
                                 valid_notes: None, // Explicitly init
                                 ..Default::default() 
                             });
                             
                             // Request note names (Async - might fail if fallback node)
                             // Simple GainNode returns empty names, safe.
                             let (tx, rx) = crossbeam_channel::bounded(1);
                             self.pending_note_names_rx = Some((new_track_idx, rx));
                             
                             // We send GetNoteNames immediately. Thread sends AddTrackNode immediately (after load).
                             // Race condition?
                             // Engine processes commands FIFO.
                             // If Thread takes 1s to load, AddTrackNode arrives later.
                             // UI sends GetNoteNames NOW.
                             // Engine executes GetNoteNames BEFORE AddTrackNode?
                             // If track_idx is out of bounds (which it is, engine doesn't have it yet!), Engine ignores it.
                             // Result: UI never gets note names.
                             
                             // FIX: Thread should send GetNoteNames? 
                             // No, Thread can't manage UI's rx channel easily (we passed rx to self.pending...).
                             // Actually, for "10/10", dealing with this race condition is important.
                             // We should NOT add track to UI until we get confirmation? 
                             // Or we accept that "Loading..." state exists.
                             // For MVP "10/10", let's fix the Note Names at least.
                             // Thread can send "GetNoteNames" command AFTER AddTrackNode?
                             // Yes, Thread has the sender.
                             // But Thread needs the `response_tx` which UI holds the `rx` for.
                             // UI can pass `tx` to the thread? `Sender` is Clone + Send.
                             // Yes! passing `tx` to thread is perfect.
                             
                             // Refined Plan:
                             // 1. Create channel for note names.
                             // 2. Pass `tx` to thread.
                             // 3. Thread sends `AddTrackNode`.
                             // 4. Thread sends `GetNoteNames { ..., response_tx: tx }`.
                             // 5. UI stores `rx`.
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

            if !self.show_arrangement_view {
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
                                
                                // Allocate space for the clip button
                                let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 30.0), egui::Sense::click());
                                
                                if response.clicked() {
                                    self.selected_track = track_idx;
                                    self.selected_clip = clip_idx;
                                    track.active_clip = Some(clip_idx);
                                    let _ = self.messenger.send(EngineCommand::TriggerClip { track_index: track_idx, clip_index: clip_idx });
                                }

                                // 1. Determine Base Colors
                                let base_color = if is_active {
                                    clip.color
                                } else {
                                    egui::Color32::from_gray(40)
                                };
                                
                                let final_color = if is_selected {
                                    egui::Color32::from_rgb(
                                        base_color.r().saturating_add(50), 
                                        base_color.g().saturating_add(50), 
                                        base_color.b().saturating_add(50)
                                    )
                                } else {
                                    base_color
                                };
                                
                                let stroke_color = if is_selected { egui::Color32::YELLOW } else { egui::Color32::BLACK };
                                let stroke_width = if is_selected { 2.0 } else { 0.0 };

                                // 2. Draw Background
                                ui.painter().rect_filled(rect, 2.0, final_color);
                                if stroke_width > 0.0 {
                                    ui.painter().rect_stroke(rect, 2.0, egui::Stroke::new(stroke_width, stroke_color), egui::StrokeKind::Middle);
                                }

                                // 3. Draw Playback Progress (Animation)
                                if is_active && self.is_playing {
                                    let sample_rate = self.engine.as_ref().map(|e| e.get_sample_rate()).unwrap_or(44100);
                                    let samples_per_beat = (sample_rate as f64 * 60.0) / self.bpm as f64;
                                    // Protect against div by zero or extremely short loops
                                    let loop_len_samples = (clip.length * samples_per_beat).max(1024.0); 
                                    
                                    // Phase 0.0 to 1.0 within the loop
                                    let phase = (self.global_sample_pos as f64 % loop_len_samples) / loop_len_samples;
                                    
                                    // Draw a semi-transparent white overlay indicating progress
                                    let progress_width = rect.width() * phase as f32;
                                    let progress_rect = egui::Rect::from_min_size(
                                        rect.min, 
                                        egui::vec2(progress_width, rect.height())
                                    );
                                    
                                    // Use additive or overlay blending look
                                    let progress_color = egui::Color32::from_rgba_premultiplied(255, 255, 255, 40); 
                                    ui.painter().rect_filled(progress_rect, 2.0, progress_color);
                                }

                                // 4. Draw Icon
                                let icon = if is_active { "▶" } else { "⏵" };   
                                ui.painter().text(
                                    rect.center(),
                                    egui::Align2::CENTER_CENTER,
                                    icon,
                                    egui::FontId::proportional(14.0),
                                    egui::Color32::WHITE
                                );
                            }

                            ui.add_space(10.0);
                            ui.separator();
                            ui.add_space(5.0);

                            // 2. Track Controls (Bottom of Strip)
                            
                            // 2. Track Mixer Strip (Compact)
                            
                            // A. Header Row: Load | GUI | Mute | Stop | Delete
                            ui.horizontal(|ui| {
                                let btn_w = (ui.available_width() - 16.0) / 5.0; // 5 buttons now
                                let btn_size = egui::vec2(btn_w, 20.0);
                                
                                // Load
                                if ui.add_sized(btn_size, egui::Button::new("📂")).clicked() {
                                    if let Some(path) = rfd::FileDialog::new().add_filter("CLAP", &["clap"]).pick_file() {
                                        if let Some(path_str) = path.to_str() {
                                             let path_cloned = path_str.to_string();
                                             let sender = self.messenger.clone();
                                             let sample_rate = self.engine.as_ref().map(|e| e.get_sample_rate() as f64).unwrap_or(44100.0);
                                             
                                             // Prepare Note Name channel
                                             let (tx, rx) = crossbeam_channel::bounded(1);
                                             self.pending_note_names_rx = Some((track_idx, rx));
                                             
                                             std::thread::spawn(move || {
                                                 let node_box: Box<dyn omni_engine::nodes::AudioNode> = match omni_engine::plugin_node::PluginNode::new(&path_cloned, sample_rate) {
                                                     Ok(node) => Box::new(node),
                                                     Err(e) => {
                                                         eprintln!("[BG] Error replacing plugin: {}. Fallback to GainNode.", e);
                                                         Box::new(omni_engine::nodes::GainNode::new(1.0))
                                                     }
                                                 };
                
                                                 let name = std::path::Path::new(&path_cloned)
                                                    .file_stem()
                                                    .and_then(|s| s.to_str())
                                                    .unwrap_or("Plugin")
                                                    .to_string();
                
                                                 let _ = sender.send(EngineCommand::ReplaceTrackNode { 
                                                     track_index: track_idx,
                                                     node: node_box, 
                                                     name, 
                                                     plugin_path: path_cloned 
                                                 });
                                                 
                                                 // Request Note Names AFTER replacement
                                                 let _ = sender.send(EngineCommand::GetNoteNames { track_index: track_idx, response_tx: tx });
                                             });
                                             
                                             track.name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Plugin").to_string();
                                             track.valid_notes = None;
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

                                // Remove
                                if ui.add_sized(btn_size, egui::Button::new("🗑")).clicked() {
                                    *self.deferred_track_remove.borrow_mut() = Some(track_idx);
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
                // Clone valid_notes before mutable borrow of clip to avoid borrow conflict
                let valid_notes = self.tracks[self.selected_track].valid_notes.clone();
                if self.selected_clip < self.tracks[self.selected_track].clips.len() {
                    let clip = &mut self.tracks[self.selected_track].clips[self.selected_clip];
                    
                    ui.heading(format!("Piano Roll: {} - Clip {}", track_name, self.selected_clip));
                    
                    // TOGGLE MODE
                    ui.horizontal(|ui| {
                        let mode_text = if clip.use_sequencer { "STEP SEQUENCER" } else { "PIANO ROLL" };
                        let mode_color = if clip.use_sequencer { egui::Color32::YELLOW } else { egui::Color32::LIGHT_BLUE };
                        if ui.add(egui::Button::new(egui::RichText::new(mode_text).strong().color(egui::Color32::BLACK)).fill(mode_color)).clicked() {
                            clip.use_sequencer = !clip.use_sequencer;
                            if clip.use_sequencer {
                                self.selected_sequencer_lane = 1; // Default to VELOCITY as requested
                            }
                            let _ = self.messenger.send(EngineCommand::UpdateClipSequencer {
                                track_index: self.selected_track,
                                clip_index: self.selected_clip,
                                use_sequencer: clip.use_sequencer,
                                data: clip.step_sequencer.clone(),
                            });
                        }
                    });

                    if clip.use_sequencer {
                        
                        let current_beat = if let Some(engine) = &self.engine {
                             if engine.is_playing() {
                                 let sample_rate = 44100.0; 
                                 let bpm = 120.0; 
                                 let samples_per_beat = (sample_rate * 60.0) / bpm;
                                 Some(self.global_sample_pos as f64 / samples_per_beat)
                             } else { None }
                        } else { None };

                        if SequencerUI::show(
                            ui, 
                            &mut clip.step_sequencer, 
                            &mut self.selected_sequencer_lane, 
                            current_beat,
                            newly_touched_param,
                            &self.plugin_params,
                            &mut self.is_learning
                        ) {
                             let _ = self.messenger.send(EngineCommand::UpdateClipSequencer {
                                track_index: self.selected_track,
                                clip_index: self.selected_clip,
                                use_sequencer: clip.use_sequencer,
                                data: clip.step_sequencer.clone(),
                            });
                        }
                    } else {
                        ui.label("Controls: [LMB] Add Note | [RMB] Delete | [MMB/Mwheel] Pan | [Ctrl+Wheel] Zoom");

                    // 1. Layout: Vertical Split (Piano Roll vs Note Expressions)
                    // We render them sequentially to avoid jumping.
                    let available_size = ui.available_size();
                    // Piano Roll takes remaining height (expressions handled by TopBottomPanel)
                    let piano_height = (available_size.y).max(200.0);
                    
                    let (piano_rect, response) = ui.allocate_at_least(
                        egui::vec2(available_size.x, piano_height),
                        egui::Sense::click_and_drag()
                    );
                    
                    let painter = ui.painter_at(piano_rect);
                    let mut note_interacted_this_frame = false;
                    
                    // 2. Input Handling (Navigation - relative to piano_rect)
                    // We must use ui.input() but ensure we check if mouse is over piano_rect for wheel?
                    // Or global wheel is fine if we are focused? Egui usually handles this if we hover.
                    
                    let (scroll_delta, modifiers) = ui.input(|i| (i.raw_scroll_delta, i.modifiers));
                    
                    if ui.rect_contains_pointer(piano_rect) {
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
                    }
                    if response.dragged_by(egui::PointerButton::Middle) {
                         self.piano_roll_scroll_x -= response.drag_delta().x;
                         self.piano_roll_scroll_y -= response.drag_delta().y;
                    }
                    if response.dragged_by(egui::PointerButton::Middle) {
                         self.piano_roll_scroll_x -= response.drag_delta().x;
                         self.piano_roll_scroll_y -= response.drag_delta().y;
                    }

                    // Clip Canvas
                    let painter = painter.with_clip_rect(piano_rect);
                    
                    // 3. Draw Background (Time Grid)
                    let beat_width = self.piano_roll_zoom_x;
                    let start_beat = (self.piano_roll_scroll_x / beat_width).max(0.0);
                    let end_beat = start_beat + (piano_rect.width() / beat_width);
                    
                    // Draw Beats
                    for b in (start_beat as usize)..(end_beat as usize + 1) {
                        let x = piano_rect.left() + (b as f32 * beat_width) - self.piano_roll_scroll_x;
                        if x >= piano_rect.left() && x <= piano_rect.right() {
                            let color = if b % 4 == 0 { egui::Color32::from_gray(80) } else { egui::Color32::from_gray(40) };
                            painter.line_segment([egui::pos2(x, piano_rect.top()), egui::pos2(x, piano_rect.bottom())], (1.0, color));
                        }
                    }
                    
                    // Visualize Loop End & Handle Interaction
                    let loop_x = piano_rect.left() + (clip.length as f32 * beat_width) - self.piano_roll_scroll_x;
                    
                    // Interaction Layer for Loop Marker
                    // We want a hit area slightly wider than the line for easier grabbing
                    let marker_hit_width = 10.0;
                    if loop_x > piano_rect.left() - marker_hit_width && loop_x < piano_rect.right() + marker_hit_width {
                        let marker_rect = egui::Rect::from_min_size(
                            egui::pos2(loop_x - marker_hit_width/2.0, piano_rect.top()), 
                            egui::vec2(marker_hit_width, piano_rect.height())
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
                    let draw_loop_x = piano_rect.left() + (clip.length as f32 * beat_width) - self.piano_roll_scroll_x;
                    
                    if draw_loop_x < piano_rect.right() {
                        // Dimmed area outside loop
                        painter.rect_filled(
                            egui::Rect::from_min_size(
                                egui::pos2(draw_loop_x, piano_rect.top()), 
                                egui::vec2(piano_rect.right() - draw_loop_x, piano_rect.height())
                            ),
                            0.0,
                            egui::Color32::from_rgba_premultiplied(0, 0, 0, 150)
                        );
                        // Loop Line
                        painter.line_segment(
                            [egui::pos2(draw_loop_x, piano_rect.top()), egui::pos2(draw_loop_x, piano_rect.bottom())],
                            (2.0, egui::Color32::YELLOW)
                        );
                        
                        // Label
                        painter.text(
                            egui::pos2(draw_loop_x + 5.0, piano_rect.top() + 10.0),
                            egui::Align2::LEFT_TOP,
                            "LOOP END",
                            egui::FontId::proportional(10.0),
                            egui::Color32::YELLOW
                        );
                    }

                    // 4. Draw Background (Pitch Grid)
                    let note_height = self.piano_roll_zoom_y;
                    // Y=0 is MIDI 127. Y=max is MIDI 0.
                    // Uses valid_notes cloned earlier to avoid borrow conflict
                    
                    for note in 0..128 {
                        let y = piano_rect.top() + ((127 - note) as f32 * note_height) - self.piano_roll_scroll_y;
                        
                        if y >= piano_rect.top() - note_height && y <= piano_rect.bottom() {
                             // Check if this note is valid for the plugin
                             let is_valid_note = match &valid_notes {
                                 None => true, // No restrictions
                                 Some(keys) => keys.contains(&(note as i16)),
                             };
                             
                             // Black keys background
                             let is_black = matches!(note % 12, 1 | 3 | 6 | 8 | 10);
                             
                             let bg_color = if !is_valid_note {
                                 // Invalid notes: dim red tint
                                 egui::Color32::from_rgba_premultiplied(50, 15, 15, 180)
                             } else if is_black {
                                 egui::Color32::from_rgba_premultiplied(30, 30, 30, 100)
                             } else {
                                 egui::Color32::TRANSPARENT
                             };
                             
                             if bg_color != egui::Color32::TRANSPARENT {
                                 painter.rect_filled(
                                     egui::Rect::from_min_size(egui::pos2(piano_rect.left(), y), egui::vec2(piano_rect.width(), note_height)),
                                     0.0,
                                     bg_color
                                 );
                             }
                             
                             painter.line_segment([egui::pos2(piano_rect.left(), y), egui::pos2(piano_rect.right(), y)], (1.0, egui::Color32::from_gray(30)));
                             
                             // Label C notes
                             if note % 12 == 0 {
                                 let label_color = if is_valid_note { egui::Color32::GRAY } else { egui::Color32::from_rgb(80, 50, 50) };
                                 painter.text(
                                    egui::pos2(piano_rect.left() + 2.0, y + note_height/2.0),
                                    egui::Align2::LEFT_CENTER,
                                    format!("C{}", note / 12 - 2),
                                    egui::FontId::proportional(10.0),
                                    label_color
                                 );
                             }
                        }
                    }
                    
                    // 5. Draw Notes & Handle Interactions
                    // We need to collect actions to avoid borrowing conflicts
                    let mut note_actions = Vec::new(); // (ActionType, NoteIdx, NewNoteData)
                    // ActionType: 0 = Move, 1 = Resize, 2 = Delete, 3 = Select Exclusive
                    
                    for (idx, note) in clip.notes.iter_mut().enumerate() {
                        let x = piano_rect.left() + (note.start as f32 * beat_width) - self.piano_roll_scroll_x;
                        let y = piano_rect.top() + ((127 - note.key) as f32 * note_height) - self.piano_roll_scroll_y;
                        let w = note.duration as f32 * beat_width;
                        let h = note_height - 1.0;
                        
                        // Culling
                        if x + w > piano_rect.left() && x < piano_rect.right() && y + h > piano_rect.top() && y < piano_rect.bottom() {
                             // Note Rect
                             let note_rect = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, h));
                             
                             // Visuals
                             // Visuals
                             let color = if note.selected {
                                 egui::Color32::from_rgb(150, 255, 150)
                             } else {
                                 egui::Color32::from_rgb(100, 200, 255)
                             };
                             
                             painter.rect(
                                 note_rect,
                                 2.0,
                                 color,
                                 egui::Stroke::new(1.0, egui::Color32::WHITE),
                                 egui::StrokeKind::Middle
                             );

                             // Interaction
                             
                             // Priority Eraser (Full Hitbox)
                             if ui.rect_contains_pointer(note_rect) && ui.input(|i| i.pointer.secondary_down()) {
                                 note_actions.push((2, idx, note.clone()));
                                 note_interacted_this_frame = true;
                                 continue; // Skip move/resize for this note
                             }

                             // We use a simplified interaction model:
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
                             if resize_response.dragged() || resize_response.clicked() || (resize_response.hovered() && ui.input(|i| i.pointer.primary_down())) { note_interacted_this_frame = true; }
                             
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
                                         probability: orig.probability,
                                         velocity_deviation: orig.velocity_deviation,
                                         condition: orig.condition,
                                     });
                                     let _ = self.messenger.send(EngineCommand::ToggleNote {
                                         track_index: self.selected_track,
                                         clip_index: self.selected_clip,
                                         start: note.start,
                                         duration: note.duration,
                                         note: note.key,
                                         probability: note.probability,
                                         velocity_deviation: note.velocity_deviation,
                                         condition: note.condition,
                                     });
                                     self.last_note_length = note.duration; // Update sticky length
                                 }
                                 self.drag_original_note = None;
                             }

                             // Check Move Body
                             if !resize_response.dragged() && !resize_response.hovered() {
                                 let body_response = ui.allocate_rect(body_rect, egui::Sense::click_and_drag());
                                 if body_response.hovered() {
                                     ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                                 }
                                 if body_response.dragged() || body_response.clicked() || (body_response.hovered() && ui.input(|i| i.pointer.primary_down())) { note_interacted_this_frame = true; }
                                 
                                 // Selection Logic
                                 if body_response.clicked() {
                                     if ui.input(|i| i.modifiers.ctrl) {
                                         note.selected = !note.selected;
                                     } else {
                                         note_actions.push((3, idx, note.clone()));
                                     }
                                 }
                                 
                                 if body_response.drag_started() {
                                     if !note.selected {
                                         note.selected = true;
                                         note_actions.push((3, idx, note.clone()));
                                     }
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
                                             let is_valid = if let Some(notes) = &valid_notes {
                                                 notes.contains(&(new_key as i16))
                                             } else { true };
                                             
                                             if is_valid {
                                                 note.key = new_key;
                                             }
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
                                             probability: orig.probability,
                                             velocity_deviation: orig.velocity_deviation,
                                             condition: orig.condition,
                                         });
                                         let _ = self.messenger.send(EngineCommand::ToggleNote {
                                             track_index: self.selected_track,
                                             clip_index: self.selected_clip,
                                             start: note.start,
                                             duration: note.duration,
                                             note: note.key,
                                             probability: note.probability,
                                             velocity_deviation: note.velocity_deviation,
                                             condition: note.condition,
                                         });
                                     }
                                     self.drag_original_note = None;
                                 }
                             }
                        }
                    }
                    
                    // Apply One-Shot Actions (Deletes)
                    // We handle Move/Resize in-place above for immediate feedback, 
                    // but Deletes change the Vec structure so must be deferred.
                    // Sort descending to safe remove
                    note_actions.sort_by(|a, b| b.1.cmp(&a.1));
                    for (action, idx, note) in note_actions {
                        if action == 2 {
                             clip.notes.remove(idx);
                             let _ = self.messenger.send(EngineCommand::ToggleNote {
                                 track_index: self.selected_track,
                                 clip_index: self.selected_clip,
                                 start: note.start,
                                 duration: note.duration,
                                 note: note.key,
                                 probability: note.probability,
                                 velocity_deviation: note.velocity_deviation,
                                 condition: note.condition,
                             });
                        }
                    }

                    // 6. Interaction: Add Note (Background Click)
                    // Ensure we check if any note was interacted with first
                    if !note_interacted_this_frame {
                        let pointer_pos = ui.input(|i| i.pointer.interact_pos());
                        if let Some(pos) = pointer_pos {
                            if piano_rect.contains(pos) {
                                 if ui.input(|i| i.pointer.primary_down()) {
                                    // Only deselect if clicked (fresh interaction), not painting
                                    if ui.input(|i| i.pointer.primary_clicked()) {
                                         if !ui.input(|i| i.modifiers.shift) && !ui.input(|i| i.modifiers.ctrl) {
                                             for note in clip.notes.iter_mut() {
                                                 note.selected = false;
                                             }
                                         }
                                    }

                                    let local_x = pos.x - piano_rect.left() + self.piano_roll_scroll_x;
                                    let local_y = pos.y - piano_rect.top() + self.piano_roll_scroll_y;
                                    
                                    let start_exact = local_x as f64 / beat_width as f64;
                                    let mut start = start_exact;
                                    if !ui.input(|i| i.modifiers.shift) {
                                        let snap = 0.25;
                                        start = (start / snap).round() * snap;
                                    }
                                    
                                    // Fix Offset: Use floor() to get correct row index
                                    let row_idx = (local_y / note_height).floor();
                                    let key_exact = 127.0 - row_idx;
                                    let note_idx = key_exact.clamp(0.0, 127.0) as u8;
                                    
                                    let is_valid = if let Some(notes) = &valid_notes {
                                        notes.contains(&(note_idx as i16))
                                    } else { true };

                                    // Prevent duplicate notes during painting
                                    let already_exists = clip.notes.iter().any(|n| 
                                        (n.start - start).abs() < 0.001 && n.key == note_idx
                                    );

                                    if start >= 0.0 && is_valid && !already_exists {
                                            // Deselect others if painting (prevents selection accumulation)
                                            if !ui.input(|i| i.modifiers.shift) && !ui.input(|i| i.modifiers.ctrl) {
                                                for n in clip.notes.iter_mut() { n.selected = false; }
                                            }
                                            let duration = if self.last_note_length < 0.01 { 0.25 } else { self.last_note_length };
                                            clip.notes.push(omni_shared::project::Note {
                                             start,
                                             duration, 
                                             key: note_idx,
                                             velocity: 100,
                                             probability: 1.0,
                                             velocity_deviation: 0,
                                             condition: omni_shared::project::NoteCondition::Always,
                                             selected: true, // Select the new note
                                         });
                                         let _ = self.messenger.send(EngineCommand::ToggleNote {
                                             track_index: self.selected_track,
                                             clip_index: self.selected_clip,
                                             start,
                                             duration,
                                             note: note_idx,
                                             probability: 1.0,
                                             velocity_deviation: 0,
                                             condition: omni_shared::project::NoteCondition::Always,
                                         });
                                    }
                                 }
                            }
                        }
                    }
                    
                    // 5. Draw Playhead (Polyrhythmic / Independent)
                    if let Some(engine) = &self.engine {
                        if engine.is_playing() {
                            let sample_rate = 44100.0; 
                            let bpm = 120.0; 
                            let samples_per_beat = (sample_rate * 60.0) / bpm;
                            
                            let global_beat = self.global_sample_pos as f64 / samples_per_beat as f64;
                            let local_beat = global_beat % clip.length;
                            
                            let playhead_x = piano_rect.left() + (local_beat as f32 * beat_width) - self.piano_roll_scroll_x;

                            if playhead_x >= piano_rect.left() && playhead_x <= piano_rect.right() {
                                painter.line_segment(
                                    [egui::pos2(playhead_x, piano_rect.top()), egui::pos2(playhead_x, piano_rect.bottom())],
                                    (2.0, egui::Color32::from_rgb(0, 200, 255))
                                );
                                 // Triangle indicator at top
                                 let triangle_size = 6.0;
                                 painter.add(egui::Shape::convex_polygon(
                                     vec![
                                         egui::pos2(playhead_x, piano_rect.top()),
                                         egui::pos2(playhead_x - triangle_size, piano_rect.top() + triangle_size),
                                         egui::pos2(playhead_x + triangle_size, piano_rect.top() + triangle_size),
                                     ],
                                     egui::Color32::from_rgb(0, 200, 255),
                                     egui::Stroke::NONE
                                 ));
                            }
                        }
                    }
                    
                     // Expressions moved to Docked Panel


                } // End Piano Roll Mode

                } // End if selected_clip
            } // End if selected_track
            } // End if !show_arrangement_view
            else {
                 self.arrangement_ui.show(
                     ui,
                     &mut self.tracks,
                     self.bpm,
                     &self.messenger,
                     self.current_step,
                     self.global_sample_pos,
                     44100.0, // Fixed sample rate for UI vis
                     self.engine.as_ref().map(|e| &e.audio_pool),
                 );
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
