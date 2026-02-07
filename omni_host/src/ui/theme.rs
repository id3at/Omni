use eframe::egui::Color32;

pub const TRACK_WIDTH: f32 = 90.0;
pub const CLIP_HEIGHT: f32 = 30.0;
pub const HEADER_HEIGHT: f32 = 30.0;

// Colors
pub const COLOR_TRACK_BG_HOVER: Color32 = Color32::from_rgb(30, 30, 30);
pub const COLOR_TRACK_BG_NORMAL: Color32 = Color32::TRANSPARENT;
pub const COLOR_TEXT_WHITE: Color32 = Color32::WHITE;
pub const COLOR_CLIP_ACTIVE: Color32 = Color32::from_rgb(100, 200, 100); // Example, actual logic in main is dynamic
pub const COLOR_CLIP_INACTIVE: Color32 = Color32::from_rgb(40, 40, 40);
pub const COLOR_SELECTED_STROKE: Color32 = Color32::YELLOW;
pub const COLOR_MUTE_ACTIVE: Color32 = Color32::RED;
pub const COLOR_MUTE_INACTIVE: Color32 = Color32::from_rgb(60, 60, 60);
pub const COLOR_PIANO_ROLL_BG_BLACK: Color32 = Color32::from_rgb(30, 30, 30);
pub const COLOR_PIANO_ROLL_BG_INVALID: Color32 = Color32::from_rgb(50, 15, 15);
