use eframe::egui;
use crossbeam_channel::Sender;
use omni_engine::EngineCommand;
use crate::TrackData;
use crate::ui::piano_roll::{PianoRollState, ExpressionMode};

pub fn show(
    ui: &mut egui::Ui,
    tracks: &mut Vec<TrackData>,
    selected_track_idx: usize,
    selected_clip_idx: usize,
    sender: &Sender<EngineCommand>,
    state: &mut PianoRollState,
) {
    if selected_track_idx >= tracks.len() { return; }
    let track = &mut tracks[selected_track_idx];
    if selected_clip_idx >= track.clips.len() { return; }
    let clip = &mut track.clips[selected_clip_idx];
    
    if clip.use_sequencer {
        ui.label("Expressions not available for Sequencer clips yet.");
        return;
    }

    // Local state for mode (hack: using ID to store primitive? No, simplified to always default or use a static/field if possible. 
    // For now, default to Velocity or use a static in this module? 
    // Better: Add `expression_mode` to PianoRollState. 
    // For now, I'll rely on a thread-local or static, OR just add a simple UI selector that resets. 
    // Actually, user wants to switch. Resetting every frame is bad.
    // I should add `expression_mode` to PianoRollState. 
    // BUT I can't modify PianoRollState here (it's passed as &Reference per signature plan, wait, &mut?
    // In main.rs I can pass &mut.
    // Let's change signature to &mut PianoRollState.
    
    ui.vertical(|ui| {
        // 1. Toolbar
        ui.horizontal(|ui| {
            ui.label("Mode:");
            ui.selectable_value(&mut state.expression_mode, ExpressionMode::Velocity, "Velocity");
            ui.selectable_value(&mut state.expression_mode, ExpressionMode::Probability, "Chance");
            ui.selectable_value(&mut state.expression_mode, ExpressionMode::VelocityDeviation, "Vel Dev");
            
            ui.separator();
            
            // Existing Selected Note Editors
            let selected_indices: Vec<usize> = clip.notes.iter().enumerate()
                .filter(|(_, n)| n.selected).map(|(i, _)| i).collect();
            
            if !selected_indices.is_empty() {
                let first_idx = selected_indices[0];
                let mut temp_note = clip.notes[first_idx].clone();
                let mut changed = false;
                
                ui.label("Selected:");
                // Simplified editor based on mode?? No, show all relevant?
                match state.expression_mode {
                    ExpressionMode::Velocity => {
                        if ui.add(egui::Slider::new(&mut temp_note.velocity, 0..=127).text("Vel")).changed() { changed = true; }
                    }
                    ExpressionMode::Probability => {
                        if ui.add(egui::Slider::new(&mut temp_note.probability, 0.0..=1.0).text("Prob")).changed() { changed = true; }
                    }
                    ExpressionMode::VelocityDeviation => {
                        if ui.add(egui::Slider::new(&mut temp_note.velocity_deviation, -64..=64).text("Dev")).changed() { changed = true; }
                    }
                }
                
                if changed {
                    // Push Undo State First
                    crate::ui::piano_roll::push_undo(state, &clip.notes);
                    
                    // Update all selected
                    for idx in selected_indices {
                        if let Some(note) = clip.notes.get_mut(idx) {
                            // Remove Old (using current start/key)
                            crate::ui::piano_roll::send_remove_note(sender, selected_track_idx, selected_clip_idx, note);
                            
                            // Apply change
                            match state.expression_mode {
                                ExpressionMode::Velocity => note.velocity = temp_note.velocity,
                                ExpressionMode::Probability => note.probability = temp_note.probability,
                                ExpressionMode::VelocityDeviation => note.velocity_deviation = temp_note.velocity_deviation,
                            }

                            // Add New
                            crate::ui::piano_roll::send_toggle_note(sender, selected_track_idx, selected_clip_idx, note);
                        }
                    }
                }
            } else {
                ui.label("(Select notes to edit group)");
            }
        });
        
        // 2. Lane Drawing
        let rect = ui.available_rect_before_wrap();
        let painter = ui.painter_at(rect);
        
        painter.rect_filled(rect, 0.0, crate::ui::theme::THEME.bg_dark.gamma_multiply(0.5));
        
        let beat_width = state.zoom_x;
        
        // Allocate the ENTIRE lane area ONCE (not per note!)
        let lane_response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
        
        for note in clip.notes.iter_mut() {
            let x = rect.left() + (note.start as f32 * beat_width) - state.scroll_x;
            let w = (note.duration as f32 * beat_width).max(4.0);
            
            if x + w > rect.left() && x < rect.right() {
                // Calculate height based on mode
                let value_normalized = match state.expression_mode {
                    ExpressionMode::Velocity => note.velocity as f32 / 127.0,
                    ExpressionMode::Probability => note.probability as f32, // 0.0-1.0
                    ExpressionMode::VelocityDeviation => (note.velocity_deviation as f32 + 64.0) / 128.0,
                };
                
                let bar_height = value_normalized * (rect.height() - 10.0);
                
                let bar_rect = egui::Rect::from_min_size(
                    egui::pos2(x, rect.bottom() - bar_height),
                    egui::vec2(w - 1.0, bar_height)
                );
                
                let color = if note.selected { crate::ui::theme::THEME.accent_primary } else { crate::ui::theme::THEME.note_bg };
                
                painter.rect_filled(bar_rect, 2.0, color);
                
                // Check interaction using pointer position (no allocate_rect!)
                let bar_hovered = ui.input(|i| {
                    i.pointer.hover_pos().map(|p| bar_rect.expand(2.0).contains(p)).unwrap_or(false)
                });
                
                if bar_hovered && lane_response.drag_started() {
                    state.pending_undo = true;
                }
                
                if bar_hovered && lane_response.dragged() && !state.pending_undo {
                    let pointer_y = ui.input(|i| i.pointer.interact_pos().map(|p| p.y).unwrap_or(rect.center().y));
                    let normalized = 1.0 - ((pointer_y - rect.top()) / (rect.height())).clamp(0.0, 1.0);
                    
                    // Update value
                    match state.expression_mode {
                        ExpressionMode::Velocity => {
                            note.velocity = (normalized * 127.0) as u8;
                        }
                        ExpressionMode::Probability => {
                            note.probability = normalized as f64;
                        }
                        ExpressionMode::VelocityDeviation => {
                            note.velocity_deviation = ((normalized * 128.0) - 64.0) as i8;
                        }
                    }
                    
                    // Send update (Remove then Add)
                    crate::ui::piano_roll::send_remove_note(sender, selected_track_idx, selected_clip_idx, note);
                    crate::ui::piano_roll::send_toggle_note(sender, selected_track_idx, selected_clip_idx, note);
                }
            }
        }
        
        // Handle deferred undo
        if state.pending_undo {
            crate::ui::piano_roll::push_undo(state, &clip.notes);
            state.pending_undo = false;
        }
    });
}
