use serde::{Deserialize, Serialize};
use uuid::Uuid;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Clip {
    pub name: String,
    /// Matrix of notes: Step -> List of MIDI note numbers
    pub notes: Vec<Vec<u8>>, 
    pub length: u32,
    pub color: [u8; 3],
}

impl Default for Clip {
    fn default() -> Self {
        Self {
            name: "New Clip".to_string(),
            notes: vec![vec![]; 16],
            length: 16,
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
