use eframe::egui;
use omni_shared::project::{StepSequencerData, SequencerLane, SequencerDirection};
use omni_shared::scale::{ScaleType, ChordType};
use std::sync::Mutex;
use lazy_static::lazy_static;

#[derive(Clone)]
enum SequencerClipboard {
    None,
    Global(StepSequencerData),
    LaneU8(SequencerLane<u8>),
    LaneI8(SequencerLane<i8>),
    LaneF32(SequencerLane<f32>),
}

lazy_static! {
    static ref CLIPBOARD: Mutex<SequencerClipboard> = Mutex::new(SequencerClipboard::None);
}

#[derive(PartialEq, Clone, Copy)]
enum LaneDisplayMode {
    Normal,
    Pitch,
    Octave,
    Bend,
    Chord,
    Roll,
    Probability,
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
                ui.selectable_value(selected_lane, 3, "Prob");
                
                // Group Performance
                egui::ComboBox::from_id_salt("perf_selector")
                    .selected_text(match *selected_lane {
                        4 => "Octave",
                        5 => "Bend",
                        6 => "Chord",
                        7 => "Roll", // Use 7 for Roll
                        8 => "Random",
                        _ => "Perf..."
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(selected_lane, 4, "Octave");
                        ui.selectable_value(selected_lane, 5, "Bend");
                        ui.selectable_value(selected_lane, 6, "Chord");
                        ui.selectable_value(selected_lane, 7, "Roll");
                        ui.selectable_value(selected_lane, 8, "Random");
                    });

                ui.selectable_value(selected_lane, 9, "Mod");
                
                // Add Learn Button if Modulation is selected
                if *selected_lane == 9 {
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
                        ui.label(note_name(data.root_key)); 

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
                    
                    changed |= Self::show_lane_u8(ui, &mut data.pitch, "Pitch", 0..=127, LaneDisplayMode::Pitch, shared_muted, current_beat, Some((data.root_key, data.scale)));
                }
                1 => changed |= Self::show_lane_u8(ui, &mut data.velocity, "Velocity", 0..=127, LaneDisplayMode::Normal, shared_muted, current_beat, None),
                2 => changed |= Self::show_lane_f32(ui, &mut data.gate, "Gate", 0.0..=1.0, shared_muted, current_beat),
                3 => changed |= Self::show_lane_u8(ui, &mut data.probability, "Probability", 0..=100, LaneDisplayMode::Probability, shared_muted, current_beat, None),
                
                // Performance Lanes
                4 => changed |= Self::show_lane_i8(ui, &mut data.performance_octave, "Octave", -2..=2, LaneDisplayMode::Octave, shared_muted, current_beat),
                5 => changed |= Self::show_lane_u8(ui, &mut data.performance_bend, "Bend", 0..=19, LaneDisplayMode::Bend, shared_muted, current_beat, None),
                6 => changed |= Self::show_lane_u8(ui, &mut data.performance_chord, "Chord", 0..=11, LaneDisplayMode::Chord, shared_muted, current_beat, None),
                7 => changed |= Self::show_lane_u8(ui, &mut data.performance_roll, "Roll", 0..=17, LaneDisplayMode::Roll, shared_muted, current_beat, None),
                8 => {
                    // Random Lane + Global Settings
                    ui.horizontal(|ui| {
                        ui.label("Random Targets:");
                        let mut mask = data.random_mask_global;
                        
                        // 0: Pitch, 1: Velocity, 2: Gate, 3: Octave, 4: Bend, 5: Chord, 6: Roll, 7: Mod
                        let mut p = (mask & 1) != 0;
                        if ui.checkbox(&mut p, "Pitch").changed() { if p { mask |= 1 } else { mask &= !1 }; changed = true; }
                        
                        let mut v = (mask & 2) != 0;
                        if ui.checkbox(&mut v, "Vel").changed() { if v { mask |= 2 } else { mask &= !2 }; changed = true; }
                        
                        let mut g = (mask & 4) != 0;
                        if ui.checkbox(&mut g, "Gate").changed() { if g { mask |= 4 } else { mask &= !4 }; changed = true; }
                        
                        let mut o = (mask & 8) != 0;
                        if ui.checkbox(&mut o, "Oct").changed() { if o { mask |= 8 } else { mask &= !8 }; changed = true; }
                        
                        let mut b = (mask & 16) != 0;
                        if ui.checkbox(&mut b, "Bend").changed() { if b { mask |= 16 } else { mask &= !16 }; changed = true; }
                        
                        let mut c = (mask & 32) != 0;
                        if ui.checkbox(&mut c, "Chrd").changed() { if c { mask |= 32 } else { mask &= !32 }; changed = true; }

                        let mut r = (mask & 64) != 0;
                        if ui.checkbox(&mut r, "Roll").changed() { if r { mask |= 64 } else { mask &= !64 }; changed = true; }
                        
                        let mut m = (mask & 128) != 0;
                        if ui.checkbox(&mut m, "Mod").changed() { if m { mask |= 128 } else { mask &= !128 }; changed = true; }
                        
                        data.random_mask_global = mask;
                    });
                    ui.separator();
                    changed |= Self::show_lane_u8(ui, &mut data.performance_random, "Rand Trigger", 0..=100, LaneDisplayMode::Probability, shared_muted, current_beat, None);
                }
                
                9 => {
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
                             changed |= Self::show_lane_u8(ui, &mut target.lane, &format!("Mod: {}", target.name), 0..=127, LaneDisplayMode::Normal, shared_muted, current_beat, None);
                         }
                    } else {
                        ui.label("No modulation targets. Click 'Learn' and touch a plugin parameter.");
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
        display_mode: LaneDisplayMode,
        muted: &mut Vec<bool>,
        current_beat: Option<f64>,
        scale_info: Option<(u8, ScaleType)>,
    ) -> bool {
        // Auto-resize
        if lane.loop_end as usize > lane.steps.len() {
            let default_val = if display_mode == LaneDisplayMode::Pitch { 60 } else { *range.start() };
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
             let default_val = if display_mode == LaneDisplayMode::Pitch { 60 } else { *range.start() };
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
                let delta = if display_mode == LaneDisplayMode::Pitch { -1 } else { -1 };
                lane.shift_values(delta, *range.start(), *range.end());
                changed = true;
            }
            if ui.button("^").clicked() {
                let delta = if display_mode == LaneDisplayMode::Pitch { 1 } else { 1 };
                lane.shift_values(delta, *range.start(), *range.end());
                changed = true;
            }
            

            ui.separator();
            
            // Individual Reset/Random
             if ui.button("x").on_hover_text("Reset Lane").clicked() {
                let default_val = if display_mode == LaneDisplayMode::Pitch { 60 } else { *range.start() };
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
                        // For i8/centered values (like Octave), we might want center axis, but u8 is usually 0..max.
                        
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
                        match display_mode {
                            LaneDisplayMode::Pitch => {
                                 let display_val = if let Some((root, scale)) = scale_info {
                                     omni_shared::scale::quantize(*val, root, scale)
                                 } else {
                                     *val
                                 };
                                let note = note_name(display_val);
                                ui.label(egui::RichText::new(note).size(10.0));
                            },
                            LaneDisplayMode::Chord => {
                                // Map 0-11 to Chord Types
                                let type_idx = (*val as usize) % 12; // Simple wrap
                                let types: Vec<ChordType> = ChordType::iter().collect();
                                let chord = types.get(type_idx).unwrap_or(&ChordType::None);
                                ui.label(egui::RichText::new(chord.name()).size(9.0));
                            },
                             LaneDisplayMode::Roll => {
                                let type_idx = *val;
                                let _name = if type_idx == 0 { "-" } else { "Roll" }; // Simplified
                                // Just show number for now, or short code
                                ui.label(egui::RichText::new(format!("{}", type_idx)).size(10.0));
                            },
                             LaneDisplayMode::Bend => {
                                ui.label(egui::RichText::new(format!("{}", val)).size(10.0));
                            },
                             LaneDisplayMode::Probability => {
                                ui.label(egui::RichText::new(format!("{}%", val)).size(10.0));
                            },
                            _ => {
                                ui.label(egui::RichText::new(format!("{}", val)).size(10.0));
                            }
                        }
                        
                        ui.label(format!("{}", i + 1));
                    });
                }
            });
        });
        changed
    }
    
    fn show_lane_i8(
        ui: &mut egui::Ui, 
        lane: &mut SequencerLane<i8>, 
        label: &str, 
        range: std::ops::RangeInclusive<i8>,
        display_mode: LaneDisplayMode,
        muted: &mut Vec<bool>,
        current_beat: Option<f64>
    ) -> bool {
        // Almost identical copy of show_lane_u8 but with i8 type and centered rendering
        // Auto-resize
        if lane.loop_end as usize > lane.steps.len() {
            lane.steps.resize(lane.loop_end as usize, 0);
            if muted.len() < lane.loop_end as usize {
                 muted.resize(lane.loop_end as usize, false);
            }
        }
        if muted.len() < lane.steps.len() {
            muted.resize(lane.steps.len(), false);
        }

        let mut changed = if lane.loop_end as usize > lane.steps.len() { true } else { false };
        if changed {
             lane.steps.resize(lane.loop_end as usize, 0);
             if muted.len() < lane.loop_end as usize {
                muted.resize(lane.loop_end as usize, false);
             }
        }
        
        ui.horizontal(|ui| {
            ui.label(label);
            ui.add_space(20.0);
            // ... Copy controls ...
             egui::ComboBox::from_id_salt(format!("{}_dir_i", label))
                .selected_text(format!("{:?}", lane.direction))
                .show_ui(ui, |ui| {
                    if ui.selectable_value(&mut lane.direction, SequencerDirection::Forward, "Forward").changed() { changed = true; }
                    if ui.selectable_value(&mut lane.direction, SequencerDirection::Backward, "Backward").changed() { changed = true; }
                    if ui.selectable_value(&mut lane.direction, SequencerDirection::Random, "Random").changed() { changed = true; }
                    if ui.selectable_value(&mut lane.direction, SequencerDirection::Each2nd, "Each 2nd").changed() { changed = true; }
                    if ui.selectable_value(&mut lane.direction, SequencerDirection::Each3rd, "Each 3rd").changed() { changed = true; }
                    if ui.selectable_value(&mut lane.direction, SequencerDirection::Each4th, "Each 4th").changed() { changed = true; }
                });

            ui.label("Start:");
            if ui.add(egui::DragValue::new(&mut lane.loop_start)).changed() { changed = true; }
            ui.label("End:");
            if ui.add(egui::DragValue::new(&mut lane.loop_end)).changed() { changed = true; }

            ui.separator();

            if ui.button("<").clicked() { lane.shift_left(); changed = true; }
            if ui.button(">").clicked() { lane.shift_right(); changed = true; }
            if ui.button("v").clicked() { lane.shift_values(-1, *range.start(), *range.end()); changed = true; }
            if ui.button("^").clicked() { lane.shift_values(1, *range.start(), *range.end()); changed = true; }
            
            ui.separator();
             if ui.button("x").on_hover_text("Reset Lane").clicked() { lane.reset(0); changed = true; }
            if ui.button("🎲").on_hover_text("Randomize Lane").clicked() { lane.randomize_values(*range.start(), *range.end()); changed = true; }
            
             ui.separator();
             let can_paste = {
                match CLIPBOARD.lock().unwrap().clone() {
                    SequencerClipboard::LaneI8(_) => true,
                    _ => false,
                }
            };
            if ui.add_enabled(can_paste, egui::Button::new("P")).on_hover_text("Paste Lane").clicked() {
                if let SequencerClipboard::LaneI8(l) = CLIPBOARD.lock().unwrap().clone() {
                    *lane = l;
                    changed = true;
                }
            }
            if ui.button("C").on_hover_text("Copy Lane").clicked() {
                 *CLIPBOARD.lock().unwrap() = SequencerClipboard::LaneI8(lane.clone());
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
                         if active_idx == i { is_active = true; }
                    }

                    ui.vertical(|ui| {
                        let bar_color = if is_in_loop { egui::Color32::YELLOW } else { egui::Color32::DARK_GRAY };
                        let (rect, _) = ui.allocate_exact_size(egui::vec2(step_width, 5.0), egui::Sense::hover());
                        ui.painter().rect_filled(rect, 0.0, bar_color);
                        
                        // Centered Value Bar
                        let min = *range.start() as f32;
                        let max = *range.end() as f32;
                        let range_span = max - min;
                        // Map val to 0..1
                        let _norm = (*val as f32 - min) / range_span;
                        
                        // For rendering, 0 is center? 
                        // If range is -2..2, center is 0.
                        // (0 - (-2)) / 4 = 0.5. Correct.
                        
                        let (rect, _) = ui.allocate_exact_size(egui::vec2(step_width - 2.0, step_height), egui::Sense::click_and_drag());
                        
                        ui.painter().rect_filled(rect, 2.0, egui::Color32::from_gray(30));
                        
                        // Draw Center Line
                        let center_y = rect.center().y;
                        ui.painter().line_segment([egui::pos2(rect.left(), center_y), egui::pos2(rect.right(), center_y)], egui::Stroke::new(1.0, egui::Color32::from_gray(50)));
                        
                        // Calculate bar from center
                        let val_f = *val as f32;
                        // Height from center
                        // Max height is step_height/2
                        // normalized from center: val / max_abs
                        
                        let center_norm = (val_f - 0.0) / (max.max(min.abs()));
                        let bar_h = center_norm * (step_height / 2.0);
                        
                        let bar_rect = if bar_h >= 0.0 {
                            egui::Rect::from_min_size(
                                egui::pos2(rect.left(), center_y - bar_h),
                                egui::vec2(rect.width(), bar_h)
                            )
                        } else {
                            egui::Rect::from_min_size(
                                egui::pos2(rect.left(), center_y),
                                egui::vec2(rect.width(), -bar_h)
                            )
                        };
                        
                        let mut fill_color = if is_in_loop { egui::Color32::from_rgb(100, 200, 255) } else { egui::Color32::from_gray(60) };
                        if is_active { fill_color = egui::Color32::WHITE; }
                        ui.painter().rect_filled(bar_rect, 2.0, fill_color);
                        
                        // Interaction
                         let pointer_down = ui.input(|i| i.pointer.primary_down());
                        if pointer_down {
                            if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
                                if rect.contains(pos) {
                                   let rel_y = (rect.bottom() - pos.y).clamp(0.0, step_height);
                                   let new_norm = rel_y / step_height;
                                   let new_val_f = min + (new_norm * range_span);
                                   let new_val = new_val_f.round() as i8;
                                   if *val != new_val {
                                       *val = new_val;
                                       changed = true;
                                   }
                                }
                            }
                        }
                        
                         // MUTE BUTTON (Copy paste)
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

                        // Label
                        match display_mode {
                            LaneDisplayMode::Octave => {
                                ui.label(egui::RichText::new(format!("{:+}", val)).size(10.0));
                            }
                            _ => {
                                ui.label(egui::RichText::new(format!("{}", val)).size(10.0));
                            }
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
        // ... (Keep existing implementation but add paste support for F32) ...
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
