use serde::{Deserialize, Serialize};
use uuid::Uuid;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum NoteCondition {
    Always,
    // Loop Iteration (1-based index)
    Iteration { expected: u8, cycle: u8 }, // e.g., 1 of 4, 3 of 4
    // Logic Operators
    PreviousNotePlayed,
    PreviousNoteSilenced,
}

impl Default for NoteCondition {
    fn default() -> Self {
        Self::Always
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Note {
    pub start: f64,      // Start time in beats
    pub duration: f64,   // Duration in beats
    pub key: u8,         // MIDI note number
    pub velocity: u8,    // 0-127
    
    #[serde(default = "default_probability")]
    pub probability: f64, // 0.0 - 1.0 (default 1.0)
    
    #[serde(default)]
    pub velocity_deviation: i8, // +/- variation (default 0)
    
    #[serde(default)]
    pub condition: NoteCondition, // Logic Operator (default Always)

    #[serde(skip)]
    pub selected: bool,  // UI selection state
}

fn default_probability() -> f64 { 1.0 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Clip {
    pub name: String,
    pub notes: Vec<Note>, 
    pub length: f64,     // Length in beats (was u32 steps)
    pub color: [u8; 3],
}

impl Default for Clip {
    fn default() -> Self {
        Self {
            name: "New Clip".to_string(),
            notes: Vec::new(),
            length: 4.0, // Default 1 bar (4 beats)
            color: [100, 100, 100],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
    pub id: Uuid,
    pub name: String,
    pub plugin_path: String,
    pub volume: f32,
    pub pan: f32,
    pub mute: bool,
    pub solo: bool,
    pub clips: Vec<Clip>,
    pub active_clip_index: Option<usize>,
    pub parameters: HashMap<u32, f32>,
}

impl Default for Track {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: "New Track".to_string(),
            plugin_path: String::new(),
            volume: 1.0,
            pan: 0.0,
            mute: false,
            solo: false,
            clips: vec![Clip::default(); 8], // 8 Clips per track (Matrix)
            active_clip_index: None,
            parameters: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub name: String,
    pub bpm: f32,
    pub tracks: Vec<Track>,
}

impl Default for Project {
    fn default() -> Self {
        Self {
            name: "New Project".to_string(),
            bpm: 120.0,
            tracks: Vec::new(),
        }
    }
}
