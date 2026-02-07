use eframe::egui;
use crossbeam_channel::Sender;
use omni_engine::EngineCommand;
use std::collections::HashMap;

pub fn show_device_view(
    ui: &mut egui::Ui,
    plugin_params: &[omni_shared::ParamInfo],
    param_states: &mut HashMap<u32, f32>,
    sender: &Sender<EngineCommand>,
    selected_track_idx: usize,
) {
    if plugin_params.is_empty() {
        return;
    }

    ui.heading("Device View: CLAP Plugin");
    ui.horizontal(|ui| {
        if ui.button(egui::RichText::new("KILL PLUGIN (TEST)").color(egui::Color32::RED)).clicked() {
            let _ = sender.send(EngineCommand::SimulateCrash { track_index: 0 }); // Hardcoded index 0 in original too
        }
    });

    egui::ScrollArea::horizontal()
        .id_salt("device_view_scroll")
        .show(ui, |ui| {
        ui.horizontal(|ui| {
            // Limit to first 16 params for UI safety in prototype
            for param in plugin_params.iter().take(16) {
                ui.push_id(param.id, |ui| {
                    ui.group(|ui| {
                        ui.set_width(100.0);
                        ui.vertical_centered(|ui| {
                            ui.label(&param.name);
                            // Simple detection for boolean params: Stepped + Min 0 + Max 1
                            let is_stepped = (param.flags & 1) != 0;
                            let is_bool = is_stepped && param.min_value == 0.0 && param.max_value == 1.0;

                            // Get current value from local map, fallback to default
                            let current_val = param_states.get(&param.id).copied().unwrap_or(param.default_value as f32);
                            let mut val = current_val;

                            if is_bool {
                                let mut bool_val = val > 0.5;
                                if ui.checkbox(&mut bool_val, "").changed() {
                                    val = if bool_val { 1.0 } else { 0.0 };
                                    param_states.insert(param.id, val); // Update local state
                                    let _ = sender.send(EngineCommand::SetPluginParam { 
                                        track_index: selected_track_idx, 
                                        id: param.id, 
                                        value: val 
                                    });
                                }
                            } else {
                                if ui.add(egui::Slider::new(&mut val, param.min_value as f32..=param.max_value as f32).show_value(false)).changed() {
                                    param_states.insert(param.id, val); // Update local state
                                    let _ = sender.send(EngineCommand::SetPluginParam { 
                                        track_index: selected_track_idx, 
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
