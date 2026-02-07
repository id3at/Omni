use eframe::egui;
use crossbeam_channel::Sender;
use omni_engine::EngineCommand;
use crate::TrackData;
use crate::ui::theme;
use crate::ui::mixer;

pub fn show_matrix(
    ui: &mut egui::Ui,
    tracks: &mut Vec<TrackData>,
    sender: &Sender<EngineCommand>,
    bpm: f32,
    is_playing: bool,
    global_sample_pos: u64,
    engine_sample_rate: f32,
    selected_track_idx: &mut usize,
    selected_clip_idx: &mut usize,
    deferred_track_remove: &std::cell::RefCell<Option<usize>>,
    pending_note_names_state: &mut Option<(usize, crossbeam_channel::Receiver<(String, Vec<omni_shared::NoteNameInfo>)>)>,
) {
    ui.heading("Session Matrix");
    ui.add_space(5.0);
    
    // MATRIX GRID (Cols = Tracks, Rows = Clips)
    egui::ScrollArea::horizontal()
        .id_salt("session_matrix_scroll")
        .show(ui, |ui| {
        ui.horizontal(|ui| {
            // MASTER SCENE COLUMN
            ui.vertical(|ui| {
                let (rect, _resp) = ui.allocate_exact_size(egui::vec2(theme::TRACK_WIDTH, theme::HEADER_HEIGHT), egui::Sense::hover());
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "Master",
                    egui::FontId::proportional(18.0), 
                    theme::THEME.text_primary,
                );
                ui.add_space(5.0);
                
                for scene_idx in 0..8 {
                    let btn_size = egui::vec2(theme::TRACK_WIDTH, theme::CLIP_HEIGHT); 
                    let btn = egui::Button::new(format!("Scene {}", scene_idx + 1));
                    
                    // Scene Button
                    if ui.add_sized(btn_size, btn).clicked() {
                        // Trigger this clip index on ALL tracks
                        for (track_idx, track) in tracks.iter_mut().enumerate() {
                            let _ = sender.send(EngineCommand::TriggerClip { 
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
            for (track_idx, track) in tracks.iter_mut().enumerate() {
                ui.push_id(track_idx, |ui| {
                    ui.vertical(|ui| {
                        ui.set_width(theme::TRACK_WIDTH); 

                        // 1. Track Header (Duplicated logic from mixer? No, mixer part only does header? 
                        // Wait, my mixer.rs implemented `show_track_strip` which did Header THEN `controls`.
                        // But here we need Header -> Clips -> Controls.
                        // So I should arguably Split `mixer.rs` into `header` and `controls`?
                        // Or just duplicate header logic here (it's simple text).
                        // I'll duplicate for now to avoid micro-fragmentation, or extract if needed.
                        
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
                        
                        // 2. Clips
                        for (clip_idx, clip) in track.clips.iter_mut().enumerate() {
                            let is_active = track.active_clip == Some(clip_idx);
                            let is_selected = *selected_track_idx == track_idx && *selected_clip_idx == clip_idx;
                            
                            let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), theme::CLIP_HEIGHT), egui::Sense::click());
                            
                            if response.clicked() {
                                *selected_track_idx = track_idx;
                                *selected_clip_idx = clip_idx;
                                track.active_clip = Some(clip_idx);
                                let _ = sender.send(EngineCommand::TriggerClip { track_index: track_idx, clip_index: clip_idx });
                            }

                            // Colors
                            let base_color = if is_active { clip.color } else { theme::THEME.clip_inactive };
                            
                            let final_color = if is_selected {
                                egui::Color32::from_rgb(
                                    base_color.r().saturating_add(50), 
                                    base_color.g().saturating_add(50), 
                                    base_color.b().saturating_add(50)
                                )
                            } else {
                                base_color
                            };
                            
                            let stroke_width = if is_selected { 2.0 } else { 0.0 };
                            let stroke_color = if is_selected { theme::THEME.accent_secondary } else { theme::THEME.border };

                            ui.painter().rect_filled(rect, 2.0, final_color);
                            if stroke_width > 0.0 {
                                ui.painter().rect_stroke(rect, 2.0, egui::Stroke::new(stroke_width, stroke_color), egui::StrokeKind::Middle);
                            }

                            // Playback Progress
                            if is_active && is_playing {
                                let samples_per_beat = (engine_sample_rate as f64 * 60.0) / bpm as f64;
                                let loop_len_samples = (clip.length * samples_per_beat).max(1024.0); 
                                let phase = (global_sample_pos as f64 % loop_len_samples) / loop_len_samples;
                                
                                let progress_width = rect.width() * phase as f32;
                                let progress_rect = egui::Rect::from_min_size(
                                    rect.min, 
                                    egui::vec2(progress_width, rect.height())
                                );
                                ui.painter().rect_filled(progress_rect, 2.0, egui::Color32::from_rgba_premultiplied(255, 255, 255, 40));
                            }

                            let icon = if is_active { "▶" } else { "⏵" };   
                            ui.painter().text(
                                rect.center(),
                                egui::Align2::CENTER_CENTER,
                                icon,
                                egui::FontId::proportional(14.0),
                                theme::THEME.text_primary
                            );
                        }

                        ui.add_space(10.0);
                        ui.separator();
                        ui.add_space(5.0);

                        // 3. Track Controls (Mixer)
                        mixer::show_track_controls(
                            ui, 
                            track, 
                            track_idx, 
                            sender, 
                            deferred_track_remove, 
                            pending_note_names_state, 
                            engine_sample_rate
                        );
                        
                        ui.add_space(10.0);
                    }); 
                });
                
                ui.add_space(4.0);
                ui.separator();
                ui.add_space(4.0);
            }
            
            // ADD TRACK COLUMN
            ui.vertical(|ui| {
                ui.set_width(theme::TRACK_WIDTH);
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(theme::TRACK_WIDTH, theme::HEADER_HEIGHT), egui::Sense::click());
                
                let bg_color = if resp.hovered() { theme::THEME.bg_light } else { theme::THEME.bg_medium };
                ui.painter().rect_filled(rect, 2.0, bg_color);
                
                // Plus Icon
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "+",
                    egui::FontId::proportional(24.0), 
                    theme::THEME.accent_primary,
                );
                
                if resp.clicked() {
                    let node = Box::new(omni_engine::nodes::GainNode::new(1.0));
                    let _ = sender.send(EngineCommand::AddTrackNode { 
                        node, 
                        name: format!("Track {}", tracks.len() + 1),
                        plugin_path: None 
                    });
                     
                    tracks.push(TrackData {
                        name: format!("Track {}", tracks.len() + 1),
                        ..Default::default()
                    });
                }
            });
        }); 
    });
}
