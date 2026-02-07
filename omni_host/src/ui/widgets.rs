use eframe::egui;

pub fn knob_ui(ui: &mut egui::Ui, value: &mut f32, range: std::ops::RangeInclusive<f32>) -> egui::Response {
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
