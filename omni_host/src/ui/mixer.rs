use eframe::egui;
use crossbeam_channel::Sender;
use omni_engine::EngineCommand;
use crate::TrackData;
use crate::ui::widgets::knob_ui;
use crate::ui::theme;

pub fn show_track_strip(
    ui: &mut egui::Ui,
    track: &mut TrackData,
    track_idx: usize,
    _sender: &Sender<EngineCommand>,
    _deferred_track_remove: &std::cell::RefCell<Option<usize>>,
    _pending_note_names_state: &mut Option<(usize, crossbeam_channel::Receiver<(String, Vec<omni_shared::NoteNameInfo>)>)>,
    selected_track_idx: &mut usize,
    _engine_sample_rate: f32,
) {
    ui.push_id(track_idx, |ui| {
        ui.vertical(|ui| {
            ui.set_width(theme::TRACK_WIDTH); 

            // Track Header
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(theme::TRACK_WIDTH, theme::HEADER_HEIGHT), egui::Sense::click());
            let bg_color = if resp.hovered() { theme::THEME.bg_light } else { theme::THEME.bg_medium };
            ui.painter().rect_filled(rect, 2.0, bg_color);
            
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                &track.name,
                egui::TextStyle::Body.resolve(ui.style()), 
                theme::THEME.text_primary,
            );
            
            if resp.clicked() {
                *selected_track_idx = track_idx;
            }
            ui.add_space(5.0);
            
            // 1. Clips Area is handled by Session View, not here. 
            // WAIT - In the original code, clips were INSIDE the loop. 
            // In the refactor, Main iterates tracks. 
            // Use Case: Main main loop calls `session::show_matrix` ? 
            // OR Main main loop calls `mixer::show_strip`.
            // The original code had: 
            /*
                // TRACK COLUMNS
                for (track_idx, track) in self.tracks.iter_mut().enumerate() {
                    ui.push_id(track_idx, |ui| {
                        ui.vertical(|ui| {
                           // Header
                           // Clips Loop
                           // Controls
                        })
                    })
                }
            */
            // So the "Mixer Strip" technically INCLUDES the clips in the current layout.
            // But strict modularity suggests "Session Matrix" and "Mixer Controls" might be separate?
            // However, structurally they are in the same vertical column.
            // If I separate them, I need to align them.
            // Ideally, `session::show_column` handles the clips, and `mixer::show_controls` handles the bottom.
            // But Main needs to iterate the columns once.
            // So:
            /*
               for track in tracks {
                   ui.vertical(|ui| {
                       session::show_header(...);
                       session::show_clips(...);
                       mixer::show_controls(...);
                   });
               }
            */
            // I will implement `show_controls` here.
            // The logic above consumes the UI.
            
            // NOTE: I am ONLY implementing the Bottom Controls here for now, 
            // OR I can make this function invoke `session::show_clips`? 
            // No, that creates circular dependency on structure or complex iteration.
            // Let's make `mixer::show_strip` handle the whole vertical column?
            // But then `mixer` knows about `clips`.
            // That's fine.
            
            // HOLD ON: To make it truly modular, `Session Matrix` and `Mixer` should be distinct.
            // But layout-wise they are interleaved.
            // Let's stick to extraction of logically distinct parts.
            // I will implement `show_track_controls` here, to be called AFTER clips.
        });
    });
}

pub fn show_track_controls(
    ui: &mut egui::Ui,
    track: &mut TrackData,
    track_idx: usize,
    sender: &Sender<EngineCommand>,
    deferred_track_remove: &std::cell::RefCell<Option<usize>>,
    pending_note_names_state: &mut Option<(usize, crossbeam_channel::Receiver<(String, Vec<omni_shared::NoteNameInfo>)>)>,
    engine_sample_rate: f32,
) {
     // A. Header Row: Load | GUI | Mute | Stop | Delete
    ui.horizontal(|ui| {
        let btn_w = (ui.available_width() - 16.0) / 5.0; // 5 buttons now
        let btn_size = egui::vec2(btn_w, 20.0);
        
        // Load
        if ui.add_sized(btn_size, egui::Button::new("📂")).clicked() {
            if let Some(path) = rfd::FileDialog::new().add_filter("CLAP", &["clap"]).pick_file() {
                if let Some(path_str) = path.to_str() {
                        let path_cloned = path_str.to_string();
                        let sender_clone = sender.clone();
                        let sample_rate = engine_sample_rate as f64;
                        
                        // Prepare Note Name channel
                        let (tx, rx) = crossbeam_channel::bounded(1);
                        *pending_note_names_state = Some((track_idx, rx));
                        
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

                            let _ = sender_clone.send(EngineCommand::ReplaceTrackNode { 
                                track_index: track_idx,
                                node: node_box, 
                                name, 
                                plugin_path: path_cloned 
                            });
                            
                            // Request Note Names AFTER replacement
                            let _ = sender_clone.send(EngineCommand::GetNoteNames { track_index: track_idx, response_tx: tx });
                        });
                        
                        track.name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Plugin").to_string();
                        track.valid_notes = None;
                }
            }
        }
        
        // GUI
        if ui.add_sized(btn_size, egui::Button::new("GUI")).clicked() {
            let _ = sender.send(EngineCommand::OpenPluginEditor { track_index: track_idx });
        }
        
        // Mute
        let mute_color = if track.mute { theme::COLOR_MUTE_ACTIVE } else { theme::COLOR_MUTE_INACTIVE };
        if ui.add_sized(btn_size, egui::Button::new("M").fill(mute_color)).clicked() {
            track.mute = !track.mute;
            let _ = sender.send(EngineCommand::SetMute { track_index: track_idx, muted: track.mute });
        }

        // Stop
        if ui.add_sized(btn_size, egui::Button::new("■")).clicked() {
            track.active_clip = None;
            let _ = sender.send(EngineCommand::StopTrack { track_index: track_idx });
        }

        // Remove
        if ui.add_sized(btn_size, egui::Button::new("🗑")).clicked() {
            *deferred_track_remove.borrow_mut() = Some(track_idx);
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
                    let _ = sender.send(EngineCommand::SetTrackVolume { track_index: track_idx, volume: track.volume });
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
                    let _ = sender.send(EngineCommand::SetTrackPan { track_index: track_idx, pan: track.pan });
                }
            });
            ui.label(egui::RichText::new(format!("{:.2}", track.pan)).small());
        });
    });

    ui.add_space(10.0);
}
