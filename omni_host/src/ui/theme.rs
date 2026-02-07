use eframe::egui::Color32;

pub const TRACK_WIDTH: f32 = 90.0;
pub const CLIP_HEIGHT: f32 = 30.0;
pub const HEADER_HEIGHT: f32 = 30.0;

pub struct Theme {
    // Base Backgrounds
    pub bg_dark: Color32,   // Main background (darkest)
    pub bg_medium: Color32, // Panels, sidebars
    pub bg_light: Color32,  // Element backgrounds, slots

    // Accents
    pub accent_primary: Color32,   // Active items, selection
    pub accent_secondary: Color32, // Secondary highlights
    pub accent_warn: Color32,      // Recording, Mute
    
    // Text
    pub text_primary: Color32,
    pub text_secondary: Color32,
    
    // Borders & Dividers
    pub border: Color32,
    pub grid_line: Color32,
    
    // Specific UI Elements
    pub clip_active: Color32,
    pub clip_inactive: Color32,
    pub knob_base: Color32,
    pub knob_arc: Color32,

    // Piano Roll specific
    pub piano_key_black: Color32,
    pub piano_key_white: Color32,
    pub note_bg: Color32,
}

pub const THEME: Theme = Theme {
    bg_dark: Color32::from_rgb(15, 15, 15),
    bg_medium: Color32::from_rgb(25, 25, 25),
    bg_light: Color32::from_rgb(40, 40, 40),
    
    accent_primary: Color32::from_rgb(100, 200, 100), // Omni Green
    accent_secondary: Color32::from_rgb(100, 150, 255), // Omni Blue
    accent_warn: Color32::from_rgb(220, 50, 50),      // Recording/Mute Red
    
    text_primary: Color32::from_rgb(240, 240, 240),
    text_secondary: Color32::from_rgb(160, 160, 160),
    
    border: Color32::from_rgb(60, 60, 60),
    grid_line: Color32::from_rgb(45, 45, 45),
    
    clip_active: Color32::from_rgb(100, 200, 100),
    clip_inactive: Color32::from_rgb(50, 50, 50),
    
    knob_base: Color32::from_rgb(30, 30, 30),
    knob_arc: Color32::from_rgb(100, 200, 100),

    piano_key_black: Color32::from_rgb(20, 20, 20),
    piano_key_white: Color32::from_rgb(220, 220, 220),
    note_bg: Color32::from_rgb(100, 150, 255),
};

// Deprecated constants (keeping temporarily to avoid immediate breakage, will remove after refactor)
pub const COLOR_TRACK_BG_HOVER: Color32 = Color32::from_rgb(30, 30, 30);
pub const COLOR_TRACK_BG_NORMAL: Color32 = Color32::TRANSPARENT;
pub const COLOR_TEXT_WHITE: Color32 = Color32::WHITE;
pub const COLOR_CLIP_ACTIVE: Color32 = Color32::from_rgb(100, 200, 100); 
pub const COLOR_CLIP_INACTIVE: Color32 = Color32::from_rgb(40, 40, 40);
pub const COLOR_SELECTED_STROKE: Color32 = Color32::YELLOW;
pub const COLOR_MUTE_ACTIVE: Color32 = Color32::RED;
pub const COLOR_MUTE_INACTIVE: Color32 = Color32::from_rgb(60, 60, 60);
pub const COLOR_PIANO_ROLL_BG_BLACK: Color32 = Color32::from_rgb(30, 30, 30);
pub const COLOR_PIANO_ROLL_BG_INVALID: Color32 = Color32::from_rgb(50, 15, 15);
