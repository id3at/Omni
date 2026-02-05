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

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum SequencerDirection {
    Forward,
    Backward,
    Random,
    Each2nd,
    Each3rd,
    Each4th,
}

impl Default for SequencerDirection {
    fn default() -> Self {
        Self::Forward
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SequencerLane<T> {
    pub steps: Vec<T>,
    pub loop_start: u32,
    pub loop_end: u32,    // Exclusive? or Inclusive? Let's say length-based or index. Thesys uses "Loop Bar". Let's use start/length mostly? Or start/end indices. Thesys Docs: "beginning and start steps". Let's use indices.
    pub direction: SequencerDirection,
    pub active: bool, // For Modulation tracks
}

impl<T: Default + Clone> SequencerLane<T> {
    pub fn new(size: usize, default_val: T) -> Self {
        Self {
            steps: vec![default_val; size],
            loop_start: 0,
            loop_end: size as u32,
            direction: SequencerDirection::default(),
            active: true,
        }
    }
}

impl<T: Default + Clone> Default for SequencerLane<T> {
    fn default() -> Self {
        Self {
            steps: Vec::new(),
            loop_start: 0,
            loop_end: 16,
            direction: SequencerDirection::default(),
            active: true,
        }
    }
}

impl<T: Clone> SequencerLane<T> {
    pub fn shift_left(&mut self) {
        if self.loop_end <= self.loop_start { return; }
        
        let start = self.loop_start as usize;
        let end = self.loop_end as usize;
        
        if end > self.steps.len() { return; }
        if start >= end { return; }
        
        let slice = &mut self.steps[start..end];
        slice.rotate_left(1);
    }
    
    pub fn shift_right(&mut self) {
        if self.loop_end <= self.loop_start { return; }

        let start = self.loop_start as usize;
        let end = self.loop_end as usize;
        
        if end > self.steps.len() { return; }
        if start >= end { return; }
        
        let slice = &mut self.steps[start..end];
        slice.rotate_right(1);
    }
}

impl SequencerLane<u8> {
    pub fn shift_values(&mut self, delta: i32, min: u8, max: u8) {
        let start = self.loop_start as usize;
        let end = self.loop_end as usize;
        
        if end > self.steps.len() { return; }
        
        for i in start..end {
            let val = self.steps[i] as i32 + delta;
            self.steps[i] = val.clamp(min as i32, max as i32) as u8;
        }
    }
}

impl SequencerLane<f32> {
    pub fn shift_values(&mut self, delta: f32, min: f32, max: f32) {
        let start = self.loop_start as usize;
        let end = self.loop_end as usize;
        
        if end > self.steps.len() { return; }
        
        for i in start..end {
            let val = self.steps[i] + delta;
            self.steps[i] = val.clamp(min, max);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepSequencerData {
    pub pitch: SequencerLane<u8>,       // 0-127
    pub velocity: SequencerLane<u8>,    // 0-127
    pub gate: SequencerLane<f32>,       // 0.0 - 1.0+
    pub performance: SequencerLane<u8>, // Enum? For now u8 index.
    pub modulation: SequencerLane<u8>,  // 0-127
    
    #[serde(default)]
    pub muted: Vec<bool>,               // Shared mute state
}

impl Default for StepSequencerData {
    fn default() -> Self {
        Self {
            pitch: SequencerLane::new(16, 60), // C3
            velocity: SequencerLane::new(16, 100),
            gate: SequencerLane::new(16, 0.5),
            performance: SequencerLane::new(16, 0),
            modulation: SequencerLane::new(16, 0),
            muted: vec![false; 16],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Clip {
    pub name: String,
    pub notes: Vec<Note>, 
    pub length: f64,     // Length in beats
    pub color: [u8; 3],
    
    #[serde(default)]
    pub use_sequencer: bool,
    
    #[serde(default)]
    pub step_sequencer: StepSequencerData,
}

impl Default for Clip {
    fn default() -> Self {
        Self {
            name: "New Clip".to_string(),
            notes: Vec::new(),
            length: 4.0, // Default 1 bar (4 beats)
            color: [100, 100, 100],
            use_sequencer: false,
            step_sequencer: StepSequencerData::default(),
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
