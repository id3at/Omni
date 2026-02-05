use eframe::egui;
use omni_shared::project::{StepSequencerData, SequencerLane, SequencerDirection};
use omni_shared::scale::ScaleType;
use std::sync::Mutex;
use lazy_static::lazy_static;

#[derive(Clone)]
enum SequencerClipboard {
    None,
    Global(StepSequencerData),
    LaneU8(SequencerLane<u8>),
    LaneF32(SequencerLane<f32>),
}

lazy_static! {
    static ref CLIPBOARD: Mutex<SequencerClipboard> = Mutex::new(SequencerClipboard::None);
}

pub struct SequencerUI;

impl SequencerUI {
    pub fn show(
        ui: &mut egui::Ui, 
        data: &mut StepSequencerData,
        selected_lane: &mut usize,
        current_beat: Option<f64>,
        // New args for Modulation
        newly_touched_param: Option<u32>, 
        param_infos: &[omni_shared::ParamInfo],
        is_learning: &mut bool,
    ) -> bool {
        let mut changed = false;
        ui.vertical(|ui| {
            // Header: Lane Selector and Global Controls
            ui.horizontal(|ui| {
                ui.selectable_value(selected_lane, 0, "Pitch");
                ui.selectable_value(selected_lane, 1, "Velocity");
                ui.selectable_value(selected_lane, 2, "Gate");
                ui.selectable_value(selected_lane, 3, "Probability");
                ui.selectable_value(selected_lane, 4, "Performance");
                ui.selectable_value(selected_lane, 5, "Modulation");
                
                // Add Learn Button if Modulation is selected
                if *selected_lane == 5 {
                    ui.separator();
                    let btn = egui::Button::new(if *is_learning { "👂 Learning..." } else { "Learn" });
                    let btn = if *is_learning { btn.fill(egui::Color32::from_rgb(255, 100, 100)) } else { btn };
                    if ui.add(btn).clicked() {
                        *is_learning = !*is_learning;
                    }
                }
                

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("🎲").on_hover_text("Randomize All").clicked() {
                        data.randomize_all();
                        changed = true;
                    }
                    if ui.button("x").on_hover_text("Reset All").clicked() {
                        data.reset_all();
                        changed = true;
                    }

                    ui.separator();

                    // Global Paste
                    let can_paste = {
                        match CLIPBOARD.lock().unwrap().clone() {
                            SequencerClipboard::Global(_) => true,
                            _ => false,
                        }
                    };
                    if ui.add_enabled(can_paste, egui::Button::new("P")).on_hover_text("Paste All").clicked() {
                        if let SequencerClipboard::Global(clip_data) = CLIPBOARD.lock().unwrap().clone() {
                            *data = clip_data;
                            changed = true;
                        }
                    }

                    // Global Copy
                    if ui.button("C").on_hover_text("Copy All").clicked() {
                        *CLIPBOARD.lock().unwrap() = SequencerClipboard::Global(data.clone());
                    }
                });
            });
            
            ui.separator();
            
            // Shared mute vector
            let shared_muted = &mut data.muted;

            match selected_lane {
                0 => {
                    ui.horizontal(|ui| {
                        ui.label("Root Key:");
                        ui.add(egui::DragValue::new(&mut data.root_key).range(0..=127));
                        ui.label(note_name(data.root_key)); // Helper function needed

                        ui.separator();

                        ui.label("Scale:");
                        egui::ComboBox::from_id_salt("scale_selector")
                            .selected_text(format!("{:?}", data.scale))
                            .show_ui(ui, |ui| {
                                for scale in ScaleType::iter() {
                                    if ui.selectable_value(&mut data.scale, scale, format!("{:?}", scale)).changed() {
                                        changed = true;
                                    }
                                }
                            });
                    });
                    ui.separator();
                    
                    changed |= Self::show_lane_u8(ui, &mut data.pitch, "Pitch", 0..=127, true, shared_muted, current_beat, Some((data.root_key, data.scale)));
                }
                1 => changed |= Self::show_lane_u8(ui, &mut data.velocity, "Velocity", 0..=127, false, shared_muted, current_beat, None),
                2 => changed |= Self::show_lane_f32(ui, &mut data.gate, "Gate", 0.0..=1.0, shared_muted, current_beat),
                3 => changed |= Self::show_lane_u8(ui, &mut data.probability, "Probability", 0..=100, false, shared_muted, current_beat, None),
                4 => changed |= Self::show_lane_u8(ui, &mut data.performance, "Performance", 0..=127, false, shared_muted, current_beat, None),
                5 => {
                    // Start: Modulation Target Logic
                    
                    // 1. Handle Learning
                    if *is_learning {
                        if let Some(pid) = newly_touched_param {
                            // Check if already exists
                            if !data.modulation_targets.iter().any(|t| t.param_id == pid) {
                                // Add new target
                                let name = param_infos.iter().find(|p| p.id == pid)
                                    .map(|p| p.name.clone())
                                    .unwrap_or_else(|| format!("Param {}", pid));
                                
                                data.modulation_targets.push(omni_shared::project::ModulationTarget {
                                    param_id: pid,
                                    name,
                                    lane: SequencerLane::new(16, 0), // Default 0
                                });
                                // Auto-select new target
                                data.active_modulation_target_index = data.modulation_targets.len() - 1;
                                changed = true;
                                *is_learning = false; // Auto-stop learning? Or keep going? Let's auto-stop for UX.
                            } else {
                                // Select existing
                                if let Some(idx) = data.modulation_targets.iter().position(|t| t.param_id == pid) {
                                    data.active_modulation_target_index = idx;
                                    changed = true;
                                    *is_learning = false;
                                }
                            }
                        }
                    }

                    // 2. Target Selector
                    if !data.modulation_targets.is_empty() {
                         ui.horizontal(|ui| {
                             ui.label("Target:");
                             egui::ComboBox::from_id_salt("mod_target_selector")
                                 .selected_text(
                                     data.modulation_targets.get(data.active_modulation_target_index)
                                     .map(|t| t.name.as_str())
                                     .unwrap_or("None")
                                 )
                                 .show_ui(ui, |ui| {
                                     for (i, target) in data.modulation_targets.iter().enumerate() {
                                         if ui.selectable_value(&mut data.active_modulation_target_index, i, &target.name).changed() {
                                             changed = true;
                                         }
                                     }
                                 });
                             
                             // Remove Button
                             if ui.button("🗑").clicked() {
                                 if data.active_modulation_target_index < data.modulation_targets.len() {
                                     data.modulation_targets.remove(data.active_modulation_target_index);
                                     data.active_modulation_target_index = 0; // Reset
                                     changed = true;
                                 }
                             }
                         });
                         ui.separator();
                         
                         if let Some(target) = data.modulation_targets.get_mut(data.active_modulation_target_index) {
                             changed |= Self::show_lane_u8(ui, &mut target.lane, &format!("Mod: {}", target.name), 0..=127, false, shared_muted, current_beat, None);
                         }
                    } else {
                        ui.label("No modulation targets. Click 'Learn' and touch a plugin parameter.");
                        // Fallback to legacy global modulation lane if desired, or just hide it.
                        // Let's show legacy for now if empty? Or just the message.
                        // Message is cleaner.
                    }
                    // End: Modulation Target Logic
                 },
                _ => {}
            }
        });
        changed
    }

    fn show_lane_u8(
        ui: &mut egui::Ui, 
        lane: &mut SequencerLane<u8>, 
        label: &str, 
        range: std::ops::RangeInclusive<u8>,
        is_pitch: bool,
        muted: &mut Vec<bool>,
        current_beat: Option<f64>,
        scale_info: Option<(u8, ScaleType)>,
    ) -> bool {
        // Auto-resize
        if lane.loop_end as usize > lane.steps.len() {
            let default_val = if is_pitch { 60 } else { 100 };
            lane.steps.resize(lane.loop_end as usize, default_val);
            if muted.len() < lane.loop_end as usize {
                 muted.resize(lane.loop_end as usize, false);
            }
        }

        // Sync muted length if loaded from old data
        if muted.len() < lane.steps.len() {
            muted.resize(lane.steps.len(), false);
        }

        let mut changed = if lane.loop_end as usize > lane.steps.len() { true } else { false };
        if changed {
             let default_val = if is_pitch { 60 } else { 100 };
             lane.steps.resize(lane.loop_end as usize, default_val);
             if muted.len() < lane.loop_end as usize {
                muted.resize(lane.loop_end as usize, false);
             }
        }
        
        ui.horizontal(|ui| {
            ui.label(label);
            ui.add_space(20.0);
            
            // Direction ComboBox
            egui::ComboBox::from_id_salt(format!("{}_dir", label))
                .selected_text(format!("{:?}", lane.direction))
                .show_ui(ui, |ui| {
                    if ui.selectable_value(&mut lane.direction, SequencerDirection::Forward, "Forward").changed() { changed = true; }
                    if ui.selectable_value(&mut lane.direction, SequencerDirection::Backward, "Backward").changed() { changed = true; }
                    if ui.selectable_value(&mut lane.direction, SequencerDirection::Random, "Random").changed() { changed = true; }
                    if ui.selectable_value(&mut lane.direction, SequencerDirection::Each2nd, "Each 2nd").changed() { changed = true; }
                    if ui.selectable_value(&mut lane.direction, SequencerDirection::Each3rd, "Each 3rd").changed() { changed = true; }
                    if ui.selectable_value(&mut lane.direction, SequencerDirection::Each4th, "Each 4th").changed() { changed = true; }
                });

            // Loop Controls
            ui.label("Start:");
            if ui.add(egui::DragValue::new(&mut lane.loop_start)).changed() { changed = true; }
            ui.label("End:");
            if ui.add(egui::DragValue::new(&mut lane.loop_end)).changed() { changed = true; }

            ui.separator();

            // Shift Controls
            if ui.button("<").clicked() {
                lane.shift_left();
                changed = true;
            }
            if ui.button(">").clicked() {
                lane.shift_right();
                changed = true;
            }
            if ui.button("v").clicked() {
                let delta = if is_pitch { -1 } else { -5 };
                lane.shift_values(delta, *range.start(), *range.end());
                changed = true;
            }
            if ui.button("^").clicked() {
                let delta = if is_pitch { 1 } else { 5 };
                lane.shift_values(delta, *range.start(), *range.end());
                changed = true;
            }
            

            ui.separator();
            
            // Individual Reset/Random
             if ui.button("x").on_hover_text("Reset Lane").clicked() {
                let default_val = if is_pitch { 60 } else { 100 };
                lane.reset(default_val);
                changed = true;
            }
            if ui.button("🎲").on_hover_text("Randomize Lane").clicked() {
                lane.randomize_values(*range.start(), *range.end());
                changed = true;
            }
            
            ui.separator();

             // Lane Paste
             let can_paste = {
                match CLIPBOARD.lock().unwrap().clone() {
                    SequencerClipboard::LaneU8(_) => true,
                    _ => false,
                }
            };
            if ui.add_enabled(can_paste, egui::Button::new("P")).on_hover_text("Paste Lane").clicked() {
                if let SequencerClipboard::LaneU8(l) = CLIPBOARD.lock().unwrap().clone() {
                    *lane = l;
                    changed = true;
                }
            }

            // Lane Copy
            if ui.button("C").on_hover_text("Copy Lane").clicked() {
                 *CLIPBOARD.lock().unwrap() = SequencerClipboard::LaneU8(lane.clone());
            }
        });
        
        ui.separator();
        
        // Steps Visualizer
        let step_width = 30.0;
        let step_height = 100.0;
        
        egui::ScrollArea::horizontal().show(ui, |ui| {
            ui.horizontal(|ui| {
                for (i, val) in lane.steps.iter_mut().enumerate().take(lane.loop_end as usize) {
                    let is_in_loop = (i as u32) >= lane.loop_start && (i as u32) < lane.loop_end;
                    
                    let mut is_active = false;
                    if let Some(beat) = current_beat {
                         let global_step = (beat * 4.0).floor() as u64;
                         let active_idx = omni_engine::sequencer::StepGenerator::get_step_index(
                             global_step, 
                             lane.direction, 
                             lane.loop_start, 
                             lane.loop_end
                         );
                         if active_idx == i {
                             is_active = true;
                         }
                    }

                    ui.vertical(|ui| {
                        // Loop Bar Indicator (Top)
                        let bar_color = if is_in_loop { egui::Color32::YELLOW } else { egui::Color32::DARK_GRAY };
                        let (rect, _) = ui.allocate_exact_size(egui::vec2(step_width, 5.0), egui::Sense::hover());
                        ui.painter().rect_filled(rect, 0.0, bar_color);
                        
                        // Value Bar
                        // Map range to height 0..1
                        let min = *range.start() as f32;
                        let max = *range.end() as f32;
                        let norm = (*val as f32 - min) / (max - min);
                        
                        let (rect, _response) = ui.allocate_exact_size(egui::vec2(step_width - 2.0, step_height), egui::Sense::click_and_drag());
                        
                        // Background
                        ui.painter().rect_filled(rect, 2.0, egui::Color32::from_gray(30));
                        
                        // Fill
                        let fill_h = norm * step_height;
                        let fill_rect = egui::Rect::from_min_size(
                            egui::pos2(rect.left(), rect.bottom() - fill_h),
                            egui::vec2(rect.width(), fill_h)
                        );
                        let mut fill_color = if is_in_loop { egui::Color32::from_rgb(100, 200, 255) } else { egui::Color32::from_gray(60) };
                        if is_active { fill_color = egui::Color32::WHITE; }
                        ui.painter().rect_filled(fill_rect, 2.0, fill_color);
                        
                        // Interaction: PAINTING
                        // Instead of checking response.dragged() (which locks to one widget),
                        // we check if the pointer is down globaly and inside our rect.
                        let pointer_down = ui.input(|i| i.pointer.primary_down());
                        if pointer_down {
                            if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
                                if rect.contains(pos) {
                                   let rel_y = (rect.bottom() - pos.y).clamp(0.0, step_height);
                                   let new_norm = rel_y / step_height;
                                   let new_val = min + (new_norm * (max - min));
                                   if *val != new_val as u8 {
                                       *val = new_val as u8;
                                       changed = true;
                                   }
                                }
                            }
                        }
                        
                        // MUTE BUTTON
                        let (mute_rect, mute_resp) = ui.allocate_exact_size(egui::vec2(step_width - 2.0, 20.0), egui::Sense::click());
                        // Mute logic: check bounds just in case for shared
                        if i < muted.len() {
                            let is_muted = muted[i];
                            
                            // Draw Background (Dark Area)
                            let bg_color = if is_muted { egui::Color32::from_gray(10) } else { egui::Color32::from_gray(20) };
                            ui.painter().rect_filled(mute_rect, 2.0, bg_color);
                            
                            // Draw Red X if muted
                            if is_muted {
                                let stroke = egui::Stroke::new(2.0, egui::Color32::RED);
                                let margin = 4.0;
                                let r = mute_rect.shrink(margin);
                                ui.painter().line_segment([r.min, r.max], stroke);
                                ui.painter().line_segment([egui::pos2(r.max.x, r.min.y), egui::pos2(r.min.x, r.max.y)], stroke);
                            }
                            
                            if mute_resp.clicked() {
                                muted[i] = !muted[i];
                                changed = true;
                            }
                        }

                        // Label
                        if is_pitch {
                             // Use raw value OR quantized value
                             let display_val = if let Some((root, scale)) = scale_info {
                                 omni_shared::scale::quantize(*val, root, scale)
                             } else {
                                 *val
                             };

                            // Note Name
                            let note_names = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
                            let oct = (display_val / 12) as i32 - 1; 
                            let note = note_names[display_val as usize % 12];
                            ui.label(egui::RichText::new(format!("{}{}", note, oct)).size(10.0));
                        } else {
                            ui.label(egui::RichText::new(format!("{}", val)).size(10.0));
                        }
                        
                        ui.label(format!("{}", i + 1));
                    });
                }
            });
        });
        changed
    }

    fn show_lane_f32(
        ui: &mut egui::Ui, 
        lane: &mut SequencerLane<f32>, 
        label: &str, 
        range: std::ops::RangeInclusive<f32>,
        muted: &mut Vec<bool>,
        current_beat: Option<f64>
    ) -> bool {
         // Auto-resize
        let mut changed = if lane.loop_end as usize > lane.steps.len() { true } else { false };
        if changed {
             lane.steps.resize(lane.loop_end as usize, 0.5);
             if muted.len() < lane.loop_end as usize {
                muted.resize(lane.loop_end as usize, false);
             }
        }
        
        // Sync muted if needed
        if muted.len() < lane.steps.len() {
             muted.resize(lane.steps.len(), false);
        }

         ui.horizontal(|ui| {
            ui.label(label);
            // ... Copy controls from u8 ...
             ui.add_space(20.0);
            
            // Direction ComboBox
            egui::ComboBox::from_id_salt(format!("{}_dir_f", label))
                .selected_text(format!("{:?}", lane.direction))
                .show_ui(ui, |ui| {
                    if ui.selectable_value(&mut lane.direction, SequencerDirection::Forward, "Forward").changed() { changed = true; }
                    if ui.selectable_value(&mut lane.direction, SequencerDirection::Backward, "Backward").changed() { changed = true; }
                    if ui.selectable_value(&mut lane.direction, SequencerDirection::Random, "Random").changed() { changed = true; }
                    if ui.selectable_value(&mut lane.direction, SequencerDirection::Each2nd, "Each 2nd").changed() { changed = true; }
                    if ui.selectable_value(&mut lane.direction, SequencerDirection::Each3rd, "Each 3rd").changed() { changed = true; }
                    if ui.selectable_value(&mut lane.direction, SequencerDirection::Each4th, "Each 4th").changed() { changed = true; }
                });

            // Loop Controls
            ui.label("Start:");
            if ui.add(egui::DragValue::new(&mut lane.loop_start)).changed() { changed = true; }
            ui.label("End:");
            if ui.add(egui::DragValue::new(&mut lane.loop_end)).changed() { changed = true; }

            ui.separator();

             // Shift Controls
            if ui.button("<").clicked() {
                lane.shift_left();
                changed = true;
            }
            if ui.button(">").clicked() {
                lane.shift_right();
                changed = true;
            }
            if ui.button("v").clicked() {
                lane.shift_values(-0.05, *range.start(), *range.end());
                changed = true;
            }
            if ui.button("^").clicked() {
                lane.shift_values(0.05, *range.start(), *range.end());
                changed = true;
            }


            ui.separator();
            
            // Individual Reset/Random
             if ui.button("x").on_hover_text("Reset Lane").clicked() {
                lane.reset(0.5); // Default gate
                changed = true;
            }
            if ui.button("🎲").on_hover_text("Randomize Lane").clicked() {
                lane.randomize_values(*range.start(), *range.end());
                changed = true;
            }

             ui.separator();

             // Lane Paste
             let can_paste = {
                match CLIPBOARD.lock().unwrap().clone() {
                    SequencerClipboard::LaneF32(_) => true,
                    _ => false,
                }
            };
            if ui.add_enabled(can_paste, egui::Button::new("P")).on_hover_text("Paste Lane").clicked() {
                if let SequencerClipboard::LaneF32(l) = CLIPBOARD.lock().unwrap().clone() {
                    *lane = l;
                    changed = true;
                }
            }

            // Lane Copy
            if ui.button("C").on_hover_text("Copy Lane").clicked() {
                 *CLIPBOARD.lock().unwrap() = SequencerClipboard::LaneF32(lane.clone());
            }
        });
        
        ui.separator();
        
        // Steps Visualizer
        let step_width = 30.0;
        let step_height = 100.0;
        
        egui::ScrollArea::horizontal().show(ui, |ui| {
            ui.horizontal(|ui| {
                for (i, val) in lane.steps.iter_mut().enumerate().take(lane.loop_end as usize) {
                    let is_in_loop = (i as u32) >= lane.loop_start && (i as u32) < lane.loop_end;
                    
                    let mut is_active = false;
                    if let Some(beat) = current_beat {
                         let global_step = (beat * 4.0).floor() as u64;
                         let active_idx = omni_engine::sequencer::StepGenerator::get_step_index(
                             global_step, 
                             lane.direction, 
                             lane.loop_start, 
                             lane.loop_end
                         );
                         if active_idx == i {
                             is_active = true;
                         }
                    }

                    ui.vertical(|ui| {
                        // Loop Bar Indicator (Top)
                        let bar_color = if is_in_loop { egui::Color32::YELLOW } else { egui::Color32::DARK_GRAY };
                        let (rect, _) = ui.allocate_exact_size(egui::vec2(step_width, 5.0), egui::Sense::hover());
                        ui.painter().rect_filled(rect, 0.0, bar_color);
                        
                        // Value Bar
                        // Map range to height 0..1
                        let min = *range.start();
                        let max = *range.end();
                        let norm = (*val - min) / (max - min);
                        
                        let (rect, _response) = ui.allocate_exact_size(egui::vec2(step_width - 2.0, step_height), egui::Sense::click_and_drag());
                        
                        // Background
                        ui.painter().rect_filled(rect, 2.0, egui::Color32::from_gray(30));
                        
                        // Fill
                        let fill_h = norm * step_height;
                         let fill_rect = egui::Rect::from_min_size(
                            egui::pos2(rect.left(), rect.bottom() - fill_h),
                            egui::vec2(rect.width(), fill_h)
                        );
                        let mut fill_color = if is_in_loop { egui::Color32::from_rgb(100, 255, 100) } else { egui::Color32::from_gray(60) };
                        if is_active { fill_color = egui::Color32::WHITE; }
                        ui.painter().rect_filled(fill_rect, 2.0, fill_color);
                        
                        // Interaction: PAINTING
                        let pointer_down = ui.input(|i| i.pointer.primary_down());
                        if pointer_down {
                            if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
                                if rect.contains(pos) {
                                   let rel_y = (rect.bottom() - pos.y).clamp(0.0, step_height);
                                   let new_norm = rel_y / step_height;
                                   let new_val = min + (new_norm * (max - min));
                                   if (*val - new_val).abs() > f32::EPSILON {
                                       *val = new_val;
                                       changed = true;
                                   }
                                }
                            }
                        }
                        
                         // MUTE BUTTON (Added here for f32)
                        let (mute_rect, mute_resp) = ui.allocate_exact_size(egui::vec2(step_width - 2.0, 20.0), egui::Sense::click());
                        
                        if i < muted.len() {
                            let is_muted = muted[i];
                            
                            let bg_color = if is_muted { egui::Color32::from_gray(10) } else { egui::Color32::from_gray(20) };
                            ui.painter().rect_filled(mute_rect, 2.0, bg_color);
                            
                            if is_muted {
                                let stroke = egui::Stroke::new(2.0, egui::Color32::RED);
                                let margin = 4.0;
                                let r = mute_rect.shrink(margin);
                                ui.painter().line_segment([r.min, r.max], stroke);
                                ui.painter().line_segment([egui::pos2(r.max.x, r.min.y), egui::pos2(r.min.x, r.max.y)], stroke);
                            }
                            
                            if mute_resp.clicked() {
                                muted[i] = !muted[i];
                                changed = true;
                            }
                        }
                        
                         ui.label(egui::RichText::new(format!("{:.2}", val)).size(10.0));
                         ui.label(format!("{}", i + 1));
                    });
                }
            });
        });
        changed
    }
}

fn note_name(note: u8) -> String {
    let note_names = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    let oct = (note / 12) as i32 - 1;
    let name = note_names[note as usize % 12];
    format!("{}{}", name, oct)
}
