use eframe::egui;
use crossbeam_channel::Sender;
use omni_engine::EngineCommand;
use crate::ClipData;

use crate::sequencer_ui::SequencerUI;
use lazy_static::lazy_static;
use std::sync::Mutex;

// ============================================================================
// CLIPBOARD (Static for Copy/Paste across clips)
// ============================================================================

lazy_static! {
    static ref NOTE_CLIPBOARD: Mutex<Vec<omni_shared::project::Note>> = Mutex::new(Vec::new());
}

// ============================================================================
// CONSTANTS
// ============================================================================



const DEFAULT_ZOOM_X: f32 = 50.0;
const DEFAULT_ZOOM_Y: f32 = 20.0;
const DEFAULT_SCROLL_Y: f32 = 60.0 * DEFAULT_ZOOM_Y;
const DEFAULT_NOTE_LENGTH: f64 = 0.25;
const LOOP_MARKER_HIT_WIDTH: f32 = 16.0;

const MIN_NOTE_DURATION: f64 = 0.125;
const DEFAULT_VELOCITY: u8 = 100;

// ============================================================================
// THEME
// ============================================================================

#[derive(Clone, Copy)]
pub struct PianoRollTheme {
    pub bg_color: egui::Color32,
    pub grid_line_primary: egui::Color32,
    pub grid_line_secondary: egui::Color32,
    pub white_key_color: egui::Color32,
    pub black_key_color: egui::Color32,
    pub note_color: egui::Color32,
    pub note_selected_color: egui::Color32,
    pub playhead_color: egui::Color32,
    pub loop_marker_color: egui::Color32,
    pub text_color: egui::Color32,
}

impl Default for PianoRollTheme {
    fn default() -> Self {
        Self {
            bg_color: crate::ui::theme::THEME.bg_dark,
            grid_line_primary: crate::ui::theme::THEME.grid_line,
            grid_line_secondary: crate::ui::theme::THEME.grid_line.gamma_multiply(0.5),
            white_key_color: crate::ui::theme::THEME.piano_key_white,
            black_key_color: crate::ui::theme::THEME.piano_key_black,
            note_color: crate::ui::theme::THEME.note_bg,
            note_selected_color: crate::ui::theme::THEME.accent_primary,
            playhead_color: egui::Color32::RED,
            loop_marker_color: crate::ui::theme::THEME.accent_secondary,
            text_color: egui::Color32::GRAY,
        }
    }
}
// ============================================================================
// ENUMS
// ============================================================================

#[derive(Clone, Copy, PartialEq, Default)]
pub enum SnapGrid {
    Off,
    Beat,      // 1.0
    Half,      // 0.5
    #[default]
    Quarter,   // 0.25
    Eighth,    // 0.125
    Sixteenth, // 0.0625
}

impl SnapGrid {
    pub fn value(&self) -> Option<f64> {
        match self {
            Self::Off => None,
            Self::Beat => Some(1.0),
            Self::Half => Some(0.5),
            Self::Quarter => Some(0.25),
            Self::Eighth => Some(0.125),
            Self::Sixteenth => Some(0.0625),
        }
    }
    
    pub fn label(&self) -> &'static str {
        match self {
            Self::Off => "OFF",
            Self::Beat => "1",
            Self::Half => "1/2",
            Self::Quarter => "1/4",
            Self::Eighth => "1/8",
            Self::Sixteenth => "1/16",
        }
    }
    
    pub fn all() -> &'static [SnapGrid] {
        &[Self::Off, Self::Beat, Self::Half, Self::Quarter, Self::Eighth, Self::Sixteenth]
    }
}

#[derive(Clone, Copy, PartialEq)]
enum NoteAction {
    Delete,
    SelectExclusive,
}

#[derive(Clone, Copy, PartialEq, Default, Debug)]
pub enum PianoRollTool {
    #[default]
    Select,
    Pencil,
    Eraser,
}

impl PianoRollTool {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Select => "Select",
            Self::Pencil => "Pencil",
            Self::Eraser => "Eraser",
        }
    }
    
    pub fn icon(&self) -> &'static str {
        match self {
            Self::Select => "⬚",
            Self::Pencil => "✏",
            Self::Eraser => "⌫",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ExpressionMode {
    Velocity,
    Probability,
    VelocityDeviation,
}

// ============================================================================
// STATE
// ============================================================================

pub struct PianoRollState {
    pub scroll_x: f32,
    pub scroll_y: f32,
    pub zoom_x: f32,
    pub zoom_y: f32,
    pub drag_original_notes: Option<Vec<(usize, omni_shared::project::Note)>>,
    pub drag_accumulated_delta: egui::Vec2,
    pub last_note_length: f64,
    // Loop marker drag state
    pub loop_drag_original: Option<f64>,
    pub loop_drag_accumulated: f32,
    // Snap grid
    pub snap_grid: SnapGrid,
    // Marquee selection
    pub marquee_start: Option<egui::Pos2>,
    pub marquee_current: Option<egui::Pos2>,
    // Undo
    pub undo_stack: Vec<Vec<omni_shared::project::Note>>,
    pub redo_stack: Vec<Vec<omni_shared::project::Note>>,
    // Internal flag for undo capture
    pub pending_undo: bool,
    // Active expression lane mode
    pub expression_mode: ExpressionMode,
    // Active tool
    pub current_tool: PianoRollTool,
}

impl Default for PianoRollState {
    fn default() -> Self {
        Self {
            scroll_x: 0.0,
            scroll_y: DEFAULT_SCROLL_Y,
            zoom_x: DEFAULT_ZOOM_X,
            zoom_y: DEFAULT_ZOOM_Y,
            drag_original_notes: None,
            drag_accumulated_delta: egui::Vec2::ZERO,
            last_note_length: DEFAULT_NOTE_LENGTH,
            loop_drag_original: None,
            loop_drag_accumulated: 0.0,
            snap_grid: SnapGrid::default(),
            marquee_start: None,
            marquee_current: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            pending_undo: false,
            expression_mode: ExpressionMode::Velocity,
            current_tool: PianoRollTool::default(),
        }
    }
}

// ============================================================================
// HELPER FUNCTIONS
// ============================================================================

/// Snap a value to the nearest grid point, if snap is enabled
fn snap_to_grid(value: f64, snap: Option<f64>) -> f64 {
    match snap {
        Some(grid) if grid > 0.0 => (value / grid).round() * grid,
        _ => value,
    }
}

/// Send a toggle note command to the engine
pub fn send_toggle_note(
    sender: &Sender<EngineCommand>,
    track_idx: usize,
    clip_idx: usize,
    note: &omni_shared::project::Note,
) {
    let _ = sender.send(EngineCommand::ToggleNote {
        track_index: track_idx,
        clip_index: clip_idx,
        start: note.start,
        duration: note.duration,
        note: note.key,
        velocity: note.velocity,
        probability: note.probability,
        velocity_deviation: note.velocity_deviation,
        condition: note.condition,
    });
}

pub fn send_remove_note(
    sender: &Sender<EngineCommand>,
    track_index: usize,
    clip_index: usize,
    note: &omni_shared::project::Note,
) {
    let _ = sender.send(EngineCommand::RemoveNote {
        track_index,
        clip_index,
        start: note.start,
        note: note.key,
    });
}

/// Push current notes state to undo stack
pub fn push_undo(state: &mut PianoRollState, notes: &[omni_shared::project::Note]) {
    state.undo_stack.push(notes.to_vec());
    state.redo_stack.clear();
    // Limit undo stack size
    if state.undo_stack.len() > 50 {
        state.undo_stack.remove(0);
    }
}

/// Delete all selected notes
fn delete_selected_notes(
    clip: &mut ClipData,
    sender: &Sender<EngineCommand>,
    track_idx: usize,
    clip_idx: usize,
    state: &mut PianoRollState,
) {
    push_undo(state, &clip.notes);
    let to_delete: Vec<_> = clip.notes.iter().filter(|n| n.selected).cloned().collect();
    clip.notes.retain(|n| !n.selected);
    for note in to_delete {
        send_remove_note(sender, track_idx, clip_idx, &note);
    }
}

/// Duplicate selected notes (offset by 1 beat)
fn duplicate_selected_notes(
    clip: &mut ClipData,
    sender: &Sender<EngineCommand>,
    track_idx: usize,
    clip_idx: usize,
    state: &mut PianoRollState,
) {
    push_undo(state, &clip.notes);
    let selected: Vec<_> = clip.notes.iter().filter(|n| n.selected).cloned().collect();
    
    // Deselect originals
    for note in &mut clip.notes {
        note.selected = false;
    }
    
    // Add duplicates offset by 1 beat
    for mut note in selected {
        note.start += 1.0;
        note.selected = true;
        clip.notes.push(note.clone());
        send_toggle_note(sender, track_idx, clip_idx, &note);
    }
}

// ============================================================================
// MAIN FUNCTION
// ============================================================================

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
            // ================================================================
            // TOOLBAR
            // ================================================================
            ui.horizontal(|ui| {
                // Tool Selector
                ui.label("Tool:");
                for tool in [PianoRollTool::Select, PianoRollTool::Pencil, PianoRollTool::Eraser] {
                    let is_selected = state.current_tool == tool;
                    let btn = egui::Button::new(format!("{} {}", tool.icon(), tool.label()))
                        .fill(if is_selected { crate::ui::theme::THEME.accent_primary } else { crate::ui::theme::THEME.bg_dark });
                    if ui.add(btn).clicked() {
                        state.current_tool = tool;
                    }
                }
                
                ui.separator();
                
                // Snap Grid
                ui.label("Snap:");
                for grid in SnapGrid::all() {
                    let is_selected = state.snap_grid == *grid;
                    let btn = egui::Button::new(grid.label())
                        .fill(if is_selected { crate::ui::theme::THEME.accent_primary } else { crate::ui::theme::THEME.bg_dark });
                    if ui.add(btn).clicked() {
                        state.snap_grid = *grid;
                    }
                }
                
                ui.separator();
                ui.label("1/2/3=Tools | Del=Delete | Ctrl+Z=Undo");
            });
            
            // ================================================================
            // KEYBOARD SHORTCUTS
            // ================================================================
            ui.input(|i| {
                // Tool switching (1=Select, 2=Pencil, 3=Eraser)
                if i.key_pressed(egui::Key::Num1) { state.current_tool = PianoRollTool::Select; }
                if i.key_pressed(egui::Key::Num2) { state.current_tool = PianoRollTool::Pencil; }
                if i.key_pressed(egui::Key::Num3) { state.current_tool = PianoRollTool::Eraser; }
                
                // Delete selected notes
                if i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace) {
                    delete_selected_notes(clip, sender, track_idx, clip_idx, state);
                }
                // Select all (Ctrl+A)
                if i.modifiers.ctrl && i.key_pressed(egui::Key::A) {
                    for note in &mut clip.notes {
                        note.selected = true;
                    }
                }
                // Duplicate (Ctrl+D)
                if i.modifiers.ctrl && i.key_pressed(egui::Key::D) {
                    duplicate_selected_notes(clip, sender, track_idx, clip_idx, state);
                }
                // Undo (Ctrl+Z)
                if i.modifiers.ctrl && !i.modifiers.shift && i.key_pressed(egui::Key::Z) {
                    if let Some(prev_notes) = state.undo_stack.pop() {
                        state.redo_stack.push(clip.notes.clone());
                        // Remove current notes from engine
                        for note in &clip.notes {
                            send_remove_note(sender, track_idx, clip_idx, note);
                        }
                        // Add restored notes to engine
                        for note in &prev_notes {
                            send_toggle_note(sender, track_idx, clip_idx, note);
                        }
                        clip.notes = prev_notes;
                    }
                }
                // Redo (Ctrl+Shift+Z)
                if i.modifiers.ctrl && i.modifiers.shift && i.key_pressed(egui::Key::Z) {
                    if let Some(next_notes) = state.redo_stack.pop() {
                        state.undo_stack.push(clip.notes.clone());
                        // Remove current notes from engine (Use RemoveNote for safety)
                        for note in &clip.notes {
                            send_remove_note(sender, track_idx, clip_idx, note);
                        }
                        // Add restored notes to engine
                        for note in &next_notes {
                            send_toggle_note(sender, track_idx, clip_idx, note);
                        }
                        clip.notes = next_notes;
                    }
                }
                // Copy (Ctrl+C)
                if i.modifiers.ctrl && i.key_pressed(egui::Key::C) {
                    let selected: Vec<_> = clip.notes.iter()
                        .filter(|n| n.selected)
                        .cloned()
                        .collect();
                    if !selected.is_empty() {
                        *NOTE_CLIPBOARD.lock().unwrap() = selected;
                    }
                }
                // Paste (Ctrl+V)
                if i.modifiers.ctrl && i.key_pressed(egui::Key::V) {
                    let clipboard = NOTE_CLIPBOARD.lock().unwrap();
                    if !clipboard.is_empty() {
                        push_undo(state, &clip.notes);
                        
                        // Find the earliest start in clipboard to calculate relative offset
                        let min_start = clipboard.iter().map(|n| n.start).fold(f64::INFINITY, f64::min);
                        
                        // Find current playhead or use end of existing notes
                        let paste_at = clip.notes.iter()
                            .filter(|n| n.selected)
                            .map(|n| n.start + n.duration)
                            .fold(0.0f64, f64::max);
                        
                        // Deselect all existing
                        for note in &mut clip.notes {
                            note.selected = false;
                        }
                        
                        // Paste with offset
                        for clipboard_note in clipboard.iter() {
                            let mut new_note = clipboard_note.clone();
                            new_note.start = paste_at + (clipboard_note.start - min_start);
                            new_note.selected = true;
                            clip.notes.push(new_note.clone());
                            send_toggle_note(sender, track_idx, clip_idx, &new_note);
                        }
                    }
                }
            });

        // 1. Layout: Vertical Split (Piano Roll vs Note Expressions)
        // We render them sequentially to avoid jumping.
        let available_size = ui.available_size();
        
        // Use full available height for piano roll (parent controls size)
        let piano_height = available_size.y;
        
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
        let theme = PianoRollTheme::default();
        
        // Draw Background
        painter.rect_filled(piano_rect, 0.0, theme.bg_color);
        
        // 3. Draw Background (Time Grid) with visual hierarchy
        let beat_width = state.zoom_x;
        let keyboard_width = 40.0f32; // Width of the piano keyboard on the left - defined early for grid
        let grid_left = piano_rect.left() + keyboard_width; // Grid starts after keyboard
        let grid_width = piano_rect.width() - keyboard_width;
        
        let start_beat = (state.scroll_x / beat_width).max(0.0);
        let end_beat = start_beat + (grid_width / beat_width);
        
        // Draw sub-beats first (finest grid) - only if zoomed in enough
        if beat_width >= 40.0 {
            let snap_value = state.snap_grid.value().unwrap_or(0.25);
            if snap_value < 1.0 {
                let sub_beat_step = snap_value;
                let start_sub = ((start_beat as f64) / sub_beat_step).floor() * sub_beat_step;
                let mut sub = start_sub;
                while sub <= end_beat as f64 {
                    let x = grid_left + (sub as f32 * beat_width) - state.scroll_x;
                    if x >= grid_left && x <= piano_rect.right() {
                        // Skip if it's a beat or bar line (will be drawn later)
                        if (sub * 4.0).fract().abs() > 0.001 {
                            painter.line_segment(
                                [egui::pos2(x, piano_rect.top()), egui::pos2(x, piano_rect.bottom())],
                                (1.0, theme.grid_line_secondary.gamma_multiply(0.3))
                            );
                        }
                    }
                    sub += sub_beat_step;
                }
            }
        }
        
        // Draw Beats (medium lines)
        for b in (start_beat as usize)..(end_beat as usize + 1) {
            let x = grid_left + (b as f32 * beat_width) - state.scroll_x;
            if x >= grid_left && x <= piano_rect.right() {
                let is_bar = b % 4 == 0;
                if is_bar {
                    // Bar line (thick, bright)
                    painter.line_segment(
                        [egui::pos2(x, piano_rect.top()), egui::pos2(x, piano_rect.bottom())],
                        (2.0, theme.grid_line_primary)
                    );
                    // Bar number label
                    painter.text(
                        egui::pos2(x + 3.0, piano_rect.top() + 2.0),
                        egui::Align2::LEFT_TOP,
                        format!("{}", b / 4 + 1),
                        egui::FontId::proportional(10.0),
                        theme.text_color.gamma_multiply(0.7)
                    );
                } else {
                    // Beat line (thin)
                    painter.line_segment(
                        [egui::pos2(x, piano_rect.top()), egui::pos2(x, piano_rect.bottom())],
                        (1.0, theme.grid_line_secondary)
                    );
                }
            }
        }
        
        // Visualize Loop End & Handle Interaction
        let loop_x = grid_left + (clip.length as f32 * beat_width) - state.scroll_x;
        
        // Interaction Layer for Loop Marker - with wider hit area for better grabbing
        if loop_x > piano_rect.left() - LOOP_MARKER_HIT_WIDTH && loop_x < piano_rect.right() + LOOP_MARKER_HIT_WIDTH {
            let marker_rect = egui::Rect::from_min_size(
                egui::pos2(loop_x - LOOP_MARKER_HIT_WIDTH/2.0, piano_rect.top()), 
                egui::vec2(LOOP_MARKER_HIT_WIDTH, piano_rect.height())
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
        let draw_loop_x = grid_left + (clip.length as f32 * beat_width) - state.scroll_x;
        
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
                (2.0, theme.loop_marker_color)
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

        // 4. Draw Background (Pitch Grid) + Piano Keyboard Ruler
        let note_height = state.zoom_y;
        
        for note in 0..128u8 {
            let y = piano_rect.top() + ((127 - note) as f32 * note_height) - state.scroll_y;
            
            if y >= piano_rect.top() - note_height && y <= piano_rect.bottom() {
                // Check if this note is valid for the plugin
                let is_valid_note = match valid_notes {
                    None => true, 
                    Some(keys) => keys.contains(&(note as i16)),
                };
                
                // Black keys
                let is_black = matches!(note % 12, 1 | 3 | 6 | 8 | 10);
                
                // --- Draw lane background (right of keyboard) ---
                let lane_rect = egui::Rect::from_min_size(
                    egui::pos2(piano_rect.left() + keyboard_width, y), 
                    egui::vec2(piano_rect.width() - keyboard_width, note_height)
                );
                
                let lane_color = if !is_valid_note {
                    theme.black_key_color.gamma_multiply(0.3)
                } else if is_black {
                    theme.black_key_color.gamma_multiply(0.6)
                } else {
                    egui::Color32::TRANSPARENT
                };
                
                if lane_color != egui::Color32::TRANSPARENT {
                    painter.rect_filled(lane_rect, 0.0, lane_color);
                }
                
                // Draw horizontal line
                painter.line_segment(
                    [egui::pos2(piano_rect.left() + keyboard_width, y), egui::pos2(piano_rect.right(), y)], 
                    (1.0, theme.grid_line_secondary.gamma_multiply(0.5))
                );
                
                // --- Draw piano key on the left ---
                let key_rect = egui::Rect::from_min_size(
                    egui::pos2(piano_rect.left(), y), 
                    egui::vec2(keyboard_width, note_height)
                );
                
                if is_black {
                    // Black key: narrower, dark
                    let black_key_rect = egui::Rect::from_min_size(
                        egui::pos2(piano_rect.left(), y), 
                        egui::vec2(keyboard_width * 0.65, note_height)
                    );
                    painter.rect_filled(black_key_rect, 2.0, egui::Color32::from_rgb(30, 30, 35));
                    painter.rect_stroke(black_key_rect, 2.0, egui::Stroke::new(1.0, egui::Color32::from_rgb(50, 50, 55)), egui::StrokeKind::Inside);
                } else {
                    // White key: full width, light
                    painter.rect_filled(key_rect, 0.0, egui::Color32::from_rgb(220, 220, 225));
                    painter.rect_stroke(key_rect, 0.0, egui::Stroke::new(1.0, egui::Color32::from_rgb(180, 180, 185)), egui::StrokeKind::Inside);
                    
                    // Draw label on C notes
                    if note % 12 == 0 {
                        let octave = (note as i32 / 12) - 1;
                        let label_color = if is_valid_note { egui::Color32::from_rgb(60, 60, 65) } else { egui::Color32::from_rgb(150, 80, 80) };
                        painter.text(
                            egui::pos2(piano_rect.left() + 3.0, y + note_height / 2.0),
                            egui::Align2::LEFT_CENTER,
                            format!("C{}", octave),
                            egui::FontId::proportional(9.0),
                            label_color
                        );
                    }
                }
            }
        }
        
        // 5. Draw Notes & Handle Interactions
        let mut note_actions: Vec<(NoteAction, usize, omni_shared::project::Note)> = Vec::new();
        
        // Drag state tracking for this frame
        let mut drag_started = false;
        let mut drag_delta = egui::Vec2::ZERO;
        let mut drag_stopped = false;
        let mut is_resizing = false;

        // Use a loop index since we might modify collection if we deleted, but we only record actions here
        for (idx, note) in clip.notes.iter_mut().enumerate() {
            let x = piano_rect.left() + keyboard_width + (note.start as f32 * beat_width) - state.scroll_x;
            let y = piano_rect.top() + ((127 - note.key) as f32 * note_height) - state.scroll_y;
            let w = note.duration as f32 * beat_width;
            let h = note_height - 1.0;
            
            // Culling
            if x + w > piano_rect.left() && x < piano_rect.right() && y + h > piano_rect.top() && y < piano_rect.bottom() {
                    let note_rect = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, h));
                    
                    let base_color = if note.selected {
                        theme.note_selected_color
                    } else {
                        theme.note_color
                    };
                    
                    // Main note body with rounded corners
                    painter.rect_filled(note_rect, 3.0, base_color);
                    
                    // Darker header/top edge for depth
                    let header_height = (h * 0.25).min(6.0);
                    let header_rect = egui::Rect::from_min_size(
                        note_rect.min, 
                        egui::vec2(w, header_height)
                    );
                    painter.rect_filled(header_rect, egui::CornerRadius { nw: 3, ne: 3, sw: 0, se: 0 }, base_color.gamma_multiply(0.7));
                    
                    // Border
                    painter.rect_stroke(
                        note_rect,
                        3.0,
                        egui::Stroke::new(1.0, if note.selected { egui::Color32::WHITE } else { base_color.gamma_multiply(1.3) }),
                        egui::StrokeKind::Inside
                    );
                    
                    // Note name label (only if wide enough)
                    if w > 25.0 && h > 12.0 {
                        let note_names = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
                        let note_name = note_names[(note.key % 12) as usize];
                        let octave = (note.key as i32 / 12) - 1;
                        painter.text(
                            egui::pos2(x + 3.0, y + h / 2.0),
                            egui::Align2::LEFT_CENTER,
                            format!("{}{}", note_name, octave),
                            egui::FontId::proportional(9.0),
                            egui::Color32::from_rgb(30, 30, 35)
                        );
                    }

                    // Interaction
                    
                    // Eraser: delete on right-click OR left-click when Eraser tool is active
                    let is_erasing = ui.rect_contains_pointer(note_rect) && (
                        ui.input(|i| i.pointer.secondary_down()) ||
                        (state.current_tool == PianoRollTool::Eraser && ui.input(|i| i.pointer.primary_clicked()))
                    );
                    if is_erasing {
                        note_actions.push((NoteAction::Delete, idx, note.clone()));
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
                        // For resizing, we usually resize just ONE note unless we implement multi-resize later.
                        // For now, let's keep it simple: if you drag resize handle, you resize THIS note.
                        // If it's selected, maybe we resize ALL selected? Let's stick to single for validation first, 
                        // OR follow the plan which implies multi-edit. 
                        // The user request was "dragging notes", usually implies moving.
                        // Let's implement multi-resize if they are selected too, for consistency.
                        
                        // If this note is not selected, select it exclusively
                        if !note.selected {
                             // Deselect others? Or just let it be. 
                             // If I resize an unselected note, usually it selects it and deselects others.
                             // But let's keep it simple.
                        }
                        
                        drag_started = true;
                        is_resizing = true;
                        state.pending_undo = true;
                        // For resize, we treat the interacted note as the "leader" if it wasn't selected
                        if !note.selected {
                            note.selected = true;
                            note_actions.push((NoteAction::SelectExclusive, idx, note.clone()));
                        }
                    }
                    
                    if resize_response.dragged() && !state.pending_undo {
                        drag_delta += resize_response.drag_delta();
                        is_resizing = true;
                    }
                    
                    if resize_response.drag_stopped() {
                        drag_stopped = true;
                        is_resizing = true;
                    }

                    // Check Move Body
                    if !resize_response.dragged() && !resize_response.hovered() {
                        let body_response = ui.allocate_rect(body_rect, egui::Sense::click_and_drag());
                        if body_response.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                        }
                        if body_response.dragged() || body_response.clicked() || (body_response.hovered() && ui.input(|i| i.pointer.primary_down())) { note_interacted_this_frame = true; }
                        
                        // Context Menu (right-click)
                        body_response.context_menu(|ui| {
                            if ui.button("Delete Note").clicked() {
                                note_actions.push((NoteAction::Delete, idx, note.clone()));
                                ui.close();
                            }
                        });
                        
                        if body_response.clicked() {
                            if ui.input(|i| i.modifiers.ctrl) {
                                note.selected = !note.selected;
                            } else {
                                note_actions.push((NoteAction::SelectExclusive, idx, note.clone()));
                            }
                        }
                        
                        if body_response.drag_started() {
                            if !note.selected {
                                note.selected = true;
                                note_actions.push((NoteAction::SelectExclusive, idx, note.clone()));
                            }
                            drag_started = true;
                            is_resizing = false;
                            state.pending_undo = true;
                        }
                        
                        if body_response.dragged() && !state.pending_undo {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                            drag_delta += body_response.drag_delta();
                        }
                        
                        if body_response.drag_stopped() {
                            drag_stopped = true;
                        }
                    }
            }
        }

        // --- Handle Drag Logic (Post-Loop) ---

        // 1. Drag Start: Capture all selected notes
        if drag_started {
            // Apply any selection changes from this frame first (e.g. click-select on drag start)
           if !note_actions.is_empty() {
                // Apply exclusively selection if needed so the capture picks up the right notes
                 for (action, idx, _) in &note_actions {
                    if let NoteAction::SelectExclusive = action {
                         // Exclusive select: deselect others
                        for (other_idx, other_note) in clip.notes.iter_mut().enumerate() {
                            other_note.selected = other_idx == *idx;
                        }
                    }
                }
           }

            let selected_notes: Vec<(usize, omni_shared::project::Note)> = clip.notes.iter()
                .enumerate()
                .filter(|(_, n)| n.selected)
                .map(|(i, n)| (i, n.clone()))
                .collect();
            
            if !selected_notes.is_empty() {
                state.drag_original_notes = Some(selected_notes);
                state.drag_accumulated_delta = egui::Vec2::ZERO;
            }
        }

        // 2. Drag Update: Apply delta to all captured notes
        if drag_delta != egui::Vec2::ZERO {
            state.drag_accumulated_delta += drag_delta;
            
            if let Some(originals) = &state.drag_original_notes {
                let delta_beats = state.drag_accumulated_delta.x / beat_width;
                let delta_keys = -(state.drag_accumulated_delta.y / note_height); // Y inverted

                let snap = if ui.input(|i| i.modifiers.shift) { None } else { state.snap_grid.value() };

                for (idx, orig_note) in originals {
                    if *idx < clip.notes.len() { // Safety check
                        let note = &mut clip.notes[*idx];
                        
                        if is_resizing {
                            // Resize logic
                             let raw_duration = (orig_note.duration + delta_beats as f64).max(MIN_NOTE_DURATION);
                             note.duration = snap_to_grid(raw_duration, snap).max(MIN_NOTE_DURATION);
                        } else {
                            // Move logic
                            let raw_start = (orig_note.start + delta_beats as f64).max(0.0);
                             note.start = snap_to_grid(raw_start, snap);

                             let new_key = (orig_note.key as f32 + delta_keys).clamp(0.0, 127.0) as u8;
                             // Verify validity
                              let is_valid = match valid_notes {
                                None => true,
                                Some(keys) => keys.contains(&(new_key as i16)),
                            };
                            if is_valid {
                                note.key = new_key;
                            }
                        }
                    }
                }
            }
        }

        // 3. Drag Stop: Finalize and send updates
        if drag_stopped {
            if let Some(originals) = &state.drag_original_notes {
                 for (idx, orig_note) in originals {
                    // Update engine: Remove old, Add new
                     if *idx < clip.notes.len() {
                        let note = &clip.notes[*idx];
                        // Optimize: Only send if changed? The engine might handle it, but allow force update
                        // We use remove/toggle to be safe.
                         send_remove_note(sender, track_idx, clip_idx, orig_note);
                         send_toggle_note(sender, track_idx, clip_idx, note);
                         
                         if is_resizing {
                             state.last_note_length = note.duration;
                         }
                     }
                 }
            }
            state.drag_original_notes = None;
        }
        
        // Handle deferred undo capture
        if state.pending_undo {
            push_undo(state, &clip.notes);
            state.pending_undo = false;
        }

        // Apply One-Shot Actions (Deletes)
        note_actions.sort_by(|a, b| b.1.cmp(&a.1));
        for (action, idx, note) in note_actions {
            match action {
                NoteAction::Delete => {
                    clip.notes.remove(idx);
                    send_toggle_note(sender, track_idx, clip_idx, &note);
                }
                NoteAction::SelectExclusive => {
                    // Exclusive select: deselect others
                    for (other_idx, other_note) in clip.notes.iter_mut().enumerate() {
                        other_note.selected = other_idx == idx;
                    }
                }
            }
        }

        // 6. Interaction: Add Note (Background Click)
        // Only in Pencil or Select mode (not Eraser)
        if !note_interacted_this_frame && state.current_tool != PianoRollTool::Eraser {
            let pointer_pos = ui.input(|i| i.pointer.interact_pos());
            if let Some(pos) = pointer_pos {
                // Define the note area (after keyboard)
                let note_area = egui::Rect::from_min_size(
                    egui::pos2(piano_rect.left() + keyboard_width, piano_rect.top()),
                    egui::vec2(piano_rect.width() - keyboard_width, piano_rect.height())
                );
                
                if note_area.contains(pos) {
                    let local_x = pos.x - note_area.left() + state.scroll_x;
                    let local_y = pos.y - piano_rect.top() + state.scroll_y;
                    
                    let start_exact = local_x as f64 / beat_width as f64;
                    let snap = if ui.input(|i| i.modifiers.shift) { None } else { state.snap_grid.value() };
                    let start = snap_to_grid(start_exact, snap);
                    
                    let row_idx = (local_y / note_height).floor();
                    let key = (127.0 - row_idx).clamp(0.0, 127.0) as u8;
                    
                    let is_valid = if let Some(notes) = valid_notes {
                        notes.contains(&(key as i16))
                    } else { true };

                    if is_valid {
                        // Find existing note at this position?
                        let exists = clip.notes.iter().any(|n| n.key == key && n.start <= start_exact && n.start + n.duration >= start_exact);
                        
                        // Pencil: continuous paint (primary_down), Select: add on click
                        let should_add = match state.current_tool {
                            PianoRollTool::Pencil => ui.input(|i| i.pointer.primary_down()),
                            PianoRollTool::Select => ui.input(|i| i.pointer.primary_clicked()),
                            PianoRollTool::Eraser => false,
                        };
                        
                        if !exists && should_add {
                            // Deselect others first (unless modifier)
                            if !ui.input(|i| i.modifiers.shift) && !ui.input(|i| i.modifiers.ctrl) {
                                for note in clip.notes.iter_mut() {
                                    note.selected = false;
                                }
                            }

                            let new_note = omni_shared::project::Note {
                                start,
                                duration: state.last_note_length,
                                key,
                                velocity: DEFAULT_VELOCITY,
                                selected: true,
                                probability: 1.0,
                                velocity_deviation: 0,
                                condition: omni_shared::project::NoteCondition::Always,
                            };
                            clip.notes.push(new_note.clone());
                            
                            send_toggle_note(sender, track_idx, clip_idx, &new_note);
                        }
                    }
                }
            }
        }
        
        // ================================================================
        // MARQUEE SELECTION (Only in Select mode)
        // ================================================================
        let primary_down = ui.input(|i| i.pointer.primary_down());
        let primary_released = ui.input(|i| i.pointer.primary_released());
        
        // Only allow marquee in Select mode AND if not dragging loop marker
        if state.current_tool == PianoRollTool::Select && !note_interacted_this_frame && state.loop_drag_original.is_none() && piano_rect.contains(ui.input(|i| i.pointer.interact_pos()).unwrap_or_default()) {
            if primary_down && state.marquee_start.is_none() {
                // Start marquee
                if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
                    state.marquee_start = Some(pos);
                }
            }
        }
        
        if let Some(start) = state.marquee_start {
            if let Some(current) = ui.input(|i| i.pointer.interact_pos()) {
                state.marquee_current = Some(current);
                
                // Draw marquee rectangle
                let marquee_rect = egui::Rect::from_two_pos(start, current);
                painter.rect_stroke(
                    marquee_rect,
                    0.0,
                    egui::Stroke::new(1.0, egui::Color32::from_rgb(100, 150, 255)),
                    egui::StrokeKind::Middle
                );
                painter.rect_filled(
                    marquee_rect,
                    0.0,
                    egui::Color32::from_rgba_unmultiplied(100, 150, 255, 30)
                );
            }
            
            // On release, select notes that intersect
            if primary_released {
                if let Some(current) = state.marquee_current {
                    let marquee_rect = egui::Rect::from_two_pos(start, current);
                    
                    // Deselect all if not adding to selection
                    if !ui.input(|i| i.modifiers.shift) && !ui.input(|i| i.modifiers.ctrl) {
                        for note in &mut clip.notes {
                            note.selected = false;
                        }
                    }
                    
                    // Select notes that intersect marquee
                    for note in &mut clip.notes {
                        let note_x = piano_rect.left() + (note.start as f32 * beat_width) - state.scroll_x;
                        let note_y = piano_rect.top() + ((127 - note.key) as f32 * note_height) - state.scroll_y;
                        let note_w = note.duration as f32 * beat_width;
                        let note_h = note_height;
                        
                        let note_rect = egui::Rect::from_min_size(
                            egui::pos2(note_x, note_y),
                            egui::vec2(note_w, note_h)
                        );
                        
                        if marquee_rect.intersects(note_rect) {
                            note.selected = true;
                        }
                    }
                }
                
                state.marquee_start = None;
                state.marquee_current = None;
            }
        }
        
        // 7. Draw Playhead
        if is_playing {
            let current_beat_global = global_sample_pos as f64 / samples_per_beat as f64;
            let loop_beat = current_beat_global % clip.length;
            
            let playhead_x = grid_left + (loop_beat as f32 * beat_width) - state.scroll_x;
            
            if playhead_x >= grid_left && playhead_x <= piano_rect.right() {
                // Playhead line
                painter.line_segment(
                    [egui::pos2(playhead_x, piano_rect.top()), egui::pos2(playhead_x, piano_rect.bottom())],
                    (2.0, theme.playhead_color)
                );
                
                // Triangle flag at the top
                let flag_size = 8.0;
                let flag_points = vec![
                    egui::pos2(playhead_x, piano_rect.top()),
                    egui::pos2(playhead_x + flag_size, piano_rect.top()),
                    egui::pos2(playhead_x, piano_rect.top() + flag_size),
                ];
                painter.add(egui::Shape::convex_polygon(flag_points, theme.playhead_color, egui::Stroke::NONE));
            }
        }
        
        // Velocity Lane removed (moved to Note Expressions)
    }
}
