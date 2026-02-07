use eframe::egui;
use crossbeam_channel::Sender;
use omni_engine::EngineCommand;
use crate::ClipData;

use crate::sequencer_ui::SequencerUI;

pub struct PianoRollState {
    pub scroll_x: f32,
    pub scroll_y: f32,
    pub zoom_x: f32,
    pub zoom_y: f32,
    pub drag_original_note: Option<omni_shared::project::Note>,
    pub drag_accumulated_delta: egui::Vec2,
    pub last_note_length: f64,
    // Loop marker drag state
    pub loop_drag_original: Option<f64>,
    pub loop_drag_accumulated: f32,
}

impl Default for PianoRollState {
    fn default() -> Self {
        Self {
            scroll_x: 0.0,
            scroll_y: 60.0 * 20.0,
            zoom_x: 50.0,
            zoom_y: 20.0,
            drag_original_note: None,
            drag_accumulated_delta: egui::Vec2::ZERO,
            last_note_length: 0.25,
            loop_drag_original: None,
            loop_drag_accumulated: 0.0,
        }
    }
}

// Function signature needs to match what logic requires
pub fn show_piano_roll(
    ui: &mut egui::Ui,
    clip: &mut ClipData,
    state: &mut PianoRollState,
    sender: &Sender<EngineCommand>,
    track_idx: usize,
    clip_idx: usize,
    track_name: &str,
    valid_notes: Option<&Vec<i16>>,
    is_playing: bool,
    global_sample_pos: u64,
    bpm: f32,
    sample_rate: f32,
    selected_sequencer_lane: &mut usize,
    newly_touched_param: Option<u32>,
    plugin_params: &[omni_shared::ParamInfo],
    is_learning: &mut bool,
) {
        ui.heading(format!("Piano Roll: {} - Clip {}", track_name, clip_idx));
        
        let samples_per_beat = (sample_rate * 60.0) / bpm;

        
        // TOGGLE MODE
        ui.horizontal(|ui| {
            let mode_text = if clip.use_sequencer { "STEP SEQUENCER" } else { "PIANO ROLL" };
            let mode_color = if clip.use_sequencer { egui::Color32::YELLOW } else { egui::Color32::LIGHT_BLUE };
            if ui.add(egui::Button::new(egui::RichText::new(mode_text).strong().color(egui::Color32::BLACK)).fill(mode_color)).clicked() {
                clip.use_sequencer = !clip.use_sequencer;
                if clip.use_sequencer {
                    *selected_sequencer_lane = 1; // Default to VELOCITY as requested
                }
                let _ = sender.send(EngineCommand::UpdateClipSequencer {
                    track_index: track_idx,
                    clip_index: clip_idx,
                    use_sequencer: clip.use_sequencer,
                    data: clip.step_sequencer.clone(),
                });
            }
        });

        if clip.use_sequencer {
            
            let current_beat = if is_playing {
                    let samples_per_beat = (sample_rate * 60.0) / bpm;
                    Some(global_sample_pos as f64 / samples_per_beat as f64)
             } else { None };

            if SequencerUI::show(
                ui, 
                &mut clip.step_sequencer, 
                selected_sequencer_lane, 
                current_beat,
                newly_touched_param,
                plugin_params,
                is_learning
            ) {
                    let _ = sender.send(EngineCommand::UpdateClipSequencer {
                    track_index: track_idx,
                    clip_index: clip_idx,
                    use_sequencer: clip.use_sequencer,
                    data: clip.step_sequencer.clone(),
                });
            }
        } else {
            ui.label("Controls: [LMB] Add Note | [RMB] Delete | [MMB/Mwheel] Pan | [Ctrl+Wheel] Zoom");

        // 1. Layout: Vertical Split (Piano Roll vs Note Expressions)
        // We render them sequentially to avoid jumping.
        let available_size = ui.available_size();
        // Piano Roll takes remaining height (expressions handled by TopBottomPanel in Main, OR we assume Main handles Expressions?)
        // In original Main, Expressions were in TopBottomPanel::bottom, OUTSIDE this closure.
        // So here we just render the Piano Roll rect.
        // The original code calculated height: `(available_size.y).max(200.0)`.
        
        let piano_height = (available_size.y).max(200.0);
        
        // Use Sense::hover() for main rect - we handle specific interactions manually
        // This prevents the main rect from "stealing" drag events from child elements like loop marker
        let (piano_rect, _response) = ui.allocate_at_least(
            egui::vec2(available_size.x, piano_height),
            egui::Sense::hover()
        );
        
        let painter = ui.painter_at(piano_rect);
        let mut note_interacted_this_frame = false;
        
        // 2. Input Handling (Navigation - relative to piano_rect)
        let (scroll_delta, modifiers, pointer_delta, middle_down) = ui.input(|i| {
            (i.raw_scroll_delta, i.modifiers, i.pointer.delta(), i.pointer.middle_down())
        });
        
        if ui.rect_contains_pointer(piano_rect) {
            // Zoom (Ctrl + Scroll)
            if modifiers.ctrl {
                if scroll_delta.y != 0.0 {
                    state.zoom_x = (state.zoom_x + scroll_delta.y * 0.1).clamp(10.0, 200.0);
                }
            } else {
                // Pan (Scroll Wheel)
                if scroll_delta.x != 0.0 || scroll_delta.y != 0.0 {
                    state.scroll_x -= scroll_delta.x;
                    state.scroll_y -= scroll_delta.y; 
                }
            }
            // Middle mouse drag for panning
            if middle_down {
                state.scroll_x -= pointer_delta.x;
                state.scroll_y -= pointer_delta.y;
            }
        }

        // Clip Canvas
        let painter = painter.with_clip_rect(piano_rect);
        
        // Draw Background
        painter.rect_filled(piano_rect, 0.0, crate::ui::theme::THEME.bg_dark);
        
        // 3. Draw Background (Time Grid)
        let beat_width = state.zoom_x;
        let start_beat = (state.scroll_x / beat_width).max(0.0);
        let end_beat = start_beat + (piano_rect.width() / beat_width);
        
        // Draw Beats
        for b in (start_beat as usize)..(end_beat as usize + 1) {
            let x = piano_rect.left() + (b as f32 * beat_width) - state.scroll_x;
            if x >= piano_rect.left() && x <= piano_rect.right() {
                let color = if b % 4 == 0 { crate::ui::theme::THEME.grid_line } else { crate::ui::theme::THEME.grid_line.gamma_multiply(0.5) };
                painter.line_segment([egui::pos2(x, piano_rect.top()), egui::pos2(x, piano_rect.bottom())], (1.0, color));
            }
        }
        
        // Visualize Loop End & Handle Interaction
        let loop_x = piano_rect.left() + (clip.length as f32 * beat_width) - state.scroll_x;
        
        // Interaction Layer for Loop Marker - with wider hit area for better grabbing
        let marker_hit_width = 16.0;
        if loop_x > piano_rect.left() - marker_hit_width && loop_x < piano_rect.right() + marker_hit_width {
            let marker_rect = egui::Rect::from_min_size(
                egui::pos2(loop_x - marker_hit_width/2.0, piano_rect.top()), 
                egui::vec2(marker_hit_width, piano_rect.height())
            );
            
            let marker_response = ui.allocate_rect(marker_rect, egui::Sense::drag());
            
            // Show resize cursor
            if marker_response.hovered() || marker_response.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            }

            // Start drag - remember original value
            if marker_response.drag_started() {
                state.loop_drag_original = Some(clip.length);
                state.loop_drag_accumulated = 0.0;
            }
            
            // During drag - accumulate delta, apply snap only to output
            if marker_response.dragged() {
                if let Some(original) = state.loop_drag_original {
                    // Accumulate pixel delta
                    state.loop_drag_accumulated += marker_response.drag_delta().x;
                    
                    // Calculate new length from original + accumulated
                    let delta_beats = state.loop_drag_accumulated / beat_width;
                    let raw_length = (original + delta_beats as f64).max(1.0);
                    
                    // Snap only the final value (Shift to disable)
                    clip.length = if !ui.input(|i| i.modifiers.shift) {
                        let snap = 1.0;
                        (raw_length / snap).round() * snap
                    } else {
                        raw_length
                    };
                    if clip.length < 1.0 { clip.length = 1.0; }
                    
                    let _ = sender.send(EngineCommand::SetClipLength {
                        track_index: track_idx,
                        clip_index: clip_idx,
                        length: clip.length
                    });
                    let _ = sender.send(EngineCommand::TriggerClip {
                        track_index: track_idx,
                        clip_index: clip_idx,
                    });
                }
            }
            
            // End drag - clear state
            if marker_response.drag_stopped() {
                state.loop_drag_original = None;
                state.loop_drag_accumulated = 0.0;
            }
        }

        // Draw Loop Visuals (Background dimming + Line)
        let draw_loop_x = piano_rect.left() + (clip.length as f32 * beat_width) - state.scroll_x;
        
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
                (2.0, crate::ui::theme::THEME.accent_secondary)
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
        let note_height = state.zoom_y;
        
        for note in 0..128 {
            let y = piano_rect.top() + ((127 - note) as f32 * note_height) - state.scroll_y;
            
            if y >= piano_rect.top() - note_height && y <= piano_rect.bottom() {
                    // Check if this note is valid for the plugin
                    let is_valid_note = match valid_notes {
                        None => true, 
                        Some(keys) => keys.contains(&(note as i16)),
                    };
                    
                    // Black keys background
                    let is_black = matches!(note % 12, 1 | 3 | 6 | 8 | 10);
                    
                    let bg_color = if !is_valid_note {
                        // Invalid notes: dim red tint
                        crate::ui::theme::THEME.piano_key_black.gamma_multiply(0.5).linear_multiply(0.5) // Approximate invalid
                    } else if is_black {
                        crate::ui::theme::THEME.piano_key_black
                    } else {
                        crate::ui::theme::THEME.piano_key_white.gamma_multiply(0.1) // Slight tint for white keys if needed, or transparent
                    };
                    
                    if bg_color != egui::Color32::TRANSPARENT {
                        painter.rect_filled(
                            egui::Rect::from_min_size(egui::pos2(piano_rect.left(), y), egui::vec2(piano_rect.width(), note_height)),
                            0.0,
                            bg_color
                        );
                    }
                    
                    painter.line_segment([egui::pos2(piano_rect.left(), y), egui::pos2(piano_rect.right(), y)], (1.0, crate::ui::theme::THEME.grid_line));
                    
                    // Label C notes
                    if note % 12 == 0 {
                        let label_color = if is_valid_note { egui::Color32::GRAY } else { egui::Color32::from_rgb(80, 50, 50) };
                        painter.text(
                            egui::pos2(piano_rect.left() + 2.0, y + note_height/2.0),
                            egui::Align2::LEFT_CENTER,
                            format!("C{}", note / 12 - 3),
                            egui::FontId::proportional(10.0),
                            label_color
                        );
                    }
            }
        }
        
        // 5. Draw Notes & Handle Interactions
        let mut note_actions = Vec::new(); // (ActionType, NoteIdx, NewNoteData)
        // ActionType: 0 = Move, 1 = Resize, 2 = Delete, 3 = Select Exclusive
        
        for (idx, note) in clip.notes.iter_mut().enumerate() {
            let x = piano_rect.left() + (note.start as f32 * beat_width) - state.scroll_x;
            let y = piano_rect.top() + ((127 - note.key) as f32 * note_height) - state.scroll_y;
            let w = note.duration as f32 * beat_width;
            let h = note_height - 1.0;
            
            // Culling
            if x + w > piano_rect.left() && x < piano_rect.right() && y + h > piano_rect.top() && y < piano_rect.bottom() {
                    let note_rect = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, h));
                    
                    let color = if note.selected {
                        crate::ui::theme::THEME.accent_primary
                    } else {
                        crate::ui::theme::THEME.note_bg
                    };
                    
                    painter.rect(
                        note_rect,
                        2.0,
                        color,
                        egui::Stroke::new(1.0, egui::Color32::WHITE),
                        egui::StrokeKind::Middle
                    );

                    // Interaction
                    
                    // Eraser
                    if ui.rect_contains_pointer(note_rect) && ui.input(|i| i.pointer.secondary_down()) {
                        note_actions.push((2, idx, note.clone()));
                        note_interacted_this_frame = true;
                        continue; 
                    }

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
                        state.drag_original_note = Some(note.clone());
                        state.drag_accumulated_delta = egui::Vec2::ZERO;
                    }
                    
                    if resize_response.dragged() {
                        state.drag_accumulated_delta += resize_response.drag_delta();
                        if let Some(orig) = &state.drag_original_note {
                            let delta_beats = state.drag_accumulated_delta.x / beat_width;
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
                        if let Some(orig) = &state.drag_original_note {
                            // Copy logic from main: toggle off/on
                            let _ = sender.send(EngineCommand::ToggleNote {
                                track_index: track_idx,
                                clip_index: clip_idx,
                                start: orig.start,
                                duration: orig.duration,
                                note: orig.key,
                                probability: orig.probability,
                                velocity_deviation: orig.velocity_deviation,
                                condition: orig.condition,
                            });
                            let _ = sender.send(EngineCommand::ToggleNote {
                                track_index: track_idx,
                                clip_index: clip_idx,
                                start: note.start,
                                duration: note.duration,
                                note: note.key,
                                probability: note.probability,
                                velocity_deviation: note.velocity_deviation,
                                condition: note.condition,
                            });
                            state.last_note_length = note.duration; 
                        }
                        state.drag_original_note = None;
                    }

                    // Check Move Body
                    if !resize_response.dragged() && !resize_response.hovered() {
                        let body_response = ui.allocate_rect(body_rect, egui::Sense::click_and_drag());
                        if body_response.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                        }
                        if body_response.dragged() || body_response.clicked() || (body_response.hovered() && ui.input(|i| i.pointer.primary_down())) { note_interacted_this_frame = true; }
                        
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
                            state.drag_original_note = Some(note.clone());
                            state.drag_accumulated_delta = egui::Vec2::ZERO;
                        }
                        
                        if body_response.dragged() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                            state.drag_accumulated_delta += body_response.drag_delta();
                            
                            if let Some(orig) = &state.drag_original_note {
                                let delta_beats = state.drag_accumulated_delta.x / beat_width;
                                let delta_keys = -(state.drag_accumulated_delta.y / note_height); // Y inverted
                                
                                note.start = (orig.start + delta_beats as f64).max(0.0);
                                
                                // Snap Beat
                                if !ui.input(|i| i.modifiers.shift) {
                                    let snap = 0.25;
                                    note.start = (note.start / snap).round() * snap;
                                }
                                
                                // Snap Key (Integral)
                                let new_key = (orig.key as f32 + delta_keys).clamp(0.0, 127.0) as u8;
                                if new_key != note.key {
                                    let is_valid = if let Some(notes) = valid_notes {
                                        notes.contains(&(new_key as i16))
                                    } else { true };
                                    
                                    if is_valid {
                                        note.key = new_key;
                                    }
                                }
                            }
                        }
                        
                        if body_response.drag_stopped() {
                            if let Some(orig) = &state.drag_original_note {
                                // Update Engine
                                let _ = sender.send(EngineCommand::ToggleNote {
                                    track_index: track_idx,
                                    clip_index: clip_idx,
                                    start: orig.start,
                                    duration: orig.duration,
                                    note: orig.key,
                                    probability: orig.probability,
                                    velocity_deviation: orig.velocity_deviation,
                                    condition: orig.condition,
                                });
                                let _ = sender.send(EngineCommand::ToggleNote {
                                    track_index: track_idx,
                                    clip_index: clip_idx,
                                    start: note.start,
                                    duration: note.duration,
                                    note: note.key,
                                    probability: note.probability,
                                    velocity_deviation: note.velocity_deviation,
                                    condition: note.condition,
                                });
                            }
                            state.drag_original_note = None;
                        }
                    }
            }
        }
        
        // Apply One-Shot Actions (Deletes)
        note_actions.sort_by(|a, b| b.1.cmp(&a.1));
        for (action, idx, note) in note_actions {
            if action == 2 {
                    clip.notes.remove(idx);
                    let _ = sender.send(EngineCommand::ToggleNote {
                        track_index: track_idx,
                        clip_index: clip_idx,
                        start: note.start,
                        duration: note.duration,
                        note: note.key,
                        probability: note.probability,
                        velocity_deviation: note.velocity_deviation,
                        condition: note.condition,
                    });
            } else if action == 3 {
                // Exclusive select: deselect others
                for (other_idx, other_note) in clip.notes.iter_mut().enumerate() {
                    other_note.selected = other_idx == idx;
                }
            }
        }

        // 6. Interaction: Add Note (Background Click)
        if !note_interacted_this_frame {
            let pointer_pos = ui.input(|i| i.pointer.interact_pos());
            if let Some(pos) = pointer_pos {
                if piano_rect.contains(pos) {
                        let local_x = pos.x - piano_rect.left() + state.scroll_x;
                        let local_y = pos.y - piano_rect.top() + state.scroll_y;
                        
                        let start_exact = local_x as f64 / beat_width as f64;
                        let mut start = start_exact;
                        if !ui.input(|i| i.modifiers.shift) {
                            let snap = 0.25;
                            start = (start / snap).round() * snap;
                        }
                        
                        let row_idx = (local_y / note_height).floor();
                        let key = (127.0 - row_idx).clamp(0.0, 127.0) as u8;
                        
                        let is_valid = if let Some(notes) = valid_notes {
                            notes.contains(&(key as i16))
                        } else { true };

                        if is_valid {
                            // Find existing?
                            let exists = clip.notes.iter().any(|n| n.key == key && n.start <= start_exact && n.start + n.duration >= start_exact);
                            
                            // Only add if not existing and clicked
                            if !exists && ui.input(|i| i.pointer.primary_clicked()) {
                                // Deselect others first
                                if !ui.input(|i| i.modifiers.shift) && !ui.input(|i| i.modifiers.ctrl) {
                                    for note in clip.notes.iter_mut() {
                                        note.selected = false;
                                    }
                                }

                                let new_note = omni_shared::project::Note {
                                    start,
                                    duration: state.last_note_length,
                                    key,
                                    velocity: 100,
                                    selected: true,
                                    probability: 1.0,
                                    velocity_deviation: 0,
                                    condition: omni_shared::project::NoteCondition::Always,
                                };
                                clip.notes.push(new_note.clone());
                                
                                let _ = sender.send(EngineCommand::ToggleNote {
                                    track_index: track_idx,
                                    clip_index: clip_idx,
                                    start: new_note.start,
                                    duration: new_note.duration,
                                    note: new_note.key,
                                    probability: new_note.probability,
                                    velocity_deviation: new_note.velocity_deviation,
                                    condition: new_note.condition,
                                });
                            }
                        }
                    }
            }
        }
        
        // 7. Draw Playhead
        if is_playing {
            let current_beat_global = global_sample_pos as f64 / samples_per_beat as f64;
            // Draw Playhead relative to scroll
            let playhead_beat = current_beat_global; // Using global beat for now, could be relative to clip logic in future
            
            // If piano roll is showing "Clip Content", we might want to mod it by length if looping?
            // For now, let's just draw the raw position. 
            // Better: loop it by clip length to visualize "where we are in the clip"
            let loop_beat = playhead_beat % clip.length;
            
            let playhead_x = piano_rect.left() + (loop_beat as f32 * beat_width) - state.scroll_x;
            
            if playhead_x >= piano_rect.left() && playhead_x <= piano_rect.right() {
                painter.line_segment(
                    [egui::pos2(playhead_x, piano_rect.top()), egui::pos2(playhead_x, piano_rect.bottom())],
                    (2.0, egui::Color32::RED)
                );
            }
        }
    }
}
