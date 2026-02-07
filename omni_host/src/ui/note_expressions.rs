use eframe::egui;
use crossbeam_channel::Sender;
use omni_engine::EngineCommand;
use crate::TrackData;

pub fn show(
    ctx: &egui::Context,
    tracks: &mut Vec<TrackData>,
    selected_track_idx: usize,
    selected_clip_idx: usize,
    sender: &Sender<EngineCommand>,
) {
    egui::TopBottomPanel::bottom("note_expressions_panel")
        .show_separator_line(true)
        .show(ctx, |ui| {
            if selected_track_idx < tracks.len() {
                let track = &mut tracks[selected_track_idx];
                if selected_clip_idx < track.clips.len() {
                    let clip = &mut track.clips[selected_clip_idx];
                    
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
                                         // 1. Remove Old
                                         let _ = sender.send(EngineCommand::ToggleNote {
                                            track_index: selected_track_idx,
                                            clip_index: selected_clip_idx,
                                            start: note.start,
                                            duration: note.duration,
                                            note: note.key,
                                            probability: 1.0, 
                                            velocity_deviation: 0,
                                            condition: omni_shared::project::NoteCondition::Always,
                                         });
                                         
                                         // Update local
                                         note.probability = temp_note.probability;
                                         note.velocity_deviation = temp_note.velocity_deviation;
                                         note.condition = temp_note.condition;
                                         
                                         // 2. Add New
                                          let _ = sender.send(EngineCommand::ToggleNote {
                                             track_index: selected_track_idx,
                                             clip_index: selected_clip_idx,
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
}
