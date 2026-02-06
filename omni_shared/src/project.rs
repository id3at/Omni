use serde::{Deserialize, Serialize};
use uuid::Uuid;
use std::collections::HashMap;
use crate::scale::ScaleType;

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

    pub fn reset(&mut self, default_val: T) {
        // Reset steps to default value
        for step in &mut self.steps {
            *step = default_val.clone();
        }
        // Reset loop points
        self.loop_start = 0;
        self.loop_end = self.steps.len() as u32;
        // Reset direction
        self.direction = SequencerDirection::default();
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

    pub fn randomize_values(&mut self, min: u8, max: u8) {
        let start = self.loop_start as usize;
        let end = self.loop_end as usize;
        
        if end > self.steps.len() { return; }
        
        for i in start..end {
            self.steps[i] = fastrand::u8(min..=max);
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

    pub fn randomize_values(&mut self, min: f32, max: f32) {
        let start = self.loop_start as usize;
        let end = self.loop_end as usize;
        
        if end > self.steps.len() { return; }
        
        for i in start..end {
            // fastrand f32 is 0..1
            let r = fastrand::f32(); 
            self.steps[i] = min + r * (max - min);
        }
    }
}

impl SequencerLane<i8> {
    pub fn shift_values(&mut self, delta: i32, min: i8, max: i8) {
        let start = self.loop_start as usize;
        let end = self.loop_end as usize;
        
        if end > self.steps.len() { return; }
        
        for i in start..end {
            let val = self.steps[i] as i32 + delta;
            self.steps[i] = val.clamp(min as i32, max as i32) as i8;
        }
    }

    pub fn randomize_values(&mut self, min: i8, max: i8) {
        let start = self.loop_start as usize;
        let end = self.loop_end as usize;
        
        if end > self.steps.len() { return; }
        
        for i in start..end {
            self.steps[i] = fastrand::i8(min..=max);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModulationTarget {
    pub param_id: u32,
    pub name: String,
    pub lane: SequencerLane<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepSequencerData {
    pub pitch: SequencerLane<u8>,       // 0-127
    pub velocity: SequencerLane<u8>,    // 0-127
    pub gate: SequencerLane<f32>,       // 0.0 - 1.0+
    pub probability: SequencerLane<u8>, // 0-100%
    pub performance_octave: SequencerLane<i8>, // -2, -1, 0, 1, 2
    pub performance_bend: SequencerLane<u8>,   // Shape ID 0-19
    pub performance_chord: SequencerLane<u8>,  // Chord Type ID
    pub performance_roll: SequencerLane<u8>,   // Roll Type ID 0-17
    pub performance_random: SequencerLane<u8>, // Probability 0-100%
    
    // Global Randomization Targets (Bitmask)
    // 0: Pitch, 1: Velocity, 2: Gate, 3: Octave, 4: Bend, 5: Chord, 6: Roll, 7: Mod
    #[serde(default)]
    pub random_mask_global: u8,
    
    // pub performance: SequencerLane<u8>, // Enum? For now u8 index. REMOVED in favor of granular lanes
    pub modulation: SequencerLane<u8>,  // Legacy/Default Global Modulation
    
    #[serde(default)]
    pub modulation_targets: Vec<ModulationTarget>,
    
    #[serde(default)]
    pub muted: Vec<bool>,               // Shared mute state

    #[serde(default)]
    pub active_modulation_target_index: usize,

    #[serde(default = "default_root_key")]
    pub root_key: u8, // Default 60 (C3)
    
    #[serde(default)]
    pub scale: ScaleType, // Default Chromatic
}

fn default_root_key() -> u8 { 60 }

impl StepSequencerData {
    pub fn reset_all(&mut self) {
        self.pitch.reset(self.root_key); // Use Root Key for pitch reset
        self.velocity.reset(100);
        self.gate.reset(0.5);
        self.probability.reset(100);
        
        self.performance_octave.reset(0);
        self.performance_bend.reset(0);
        self.performance_chord.reset(0);
        self.performance_roll.reset(0);
        self.performance_random.reset(0);
        
        self.modulation.reset(0);
        
        // Reset mutes
        for m in &mut self.muted {
            *m = false;
        }
    }

    pub fn randomize_all(&mut self) {
        // Pitch: 0-127
        self.pitch.randomize_values(0, 127);
        // Velocity: 0-127
        self.velocity.randomize_values(0, 127);
        // Gate: 0.0-1.0
        self.gate.randomize_values(0.0, 1.0);
        // Probability: 0-100
        self.probability.randomize_values(0, 100);
        
        self.performance_octave.randomize_values(-2, 2);
        self.performance_bend.randomize_values(0, 19);
        self.performance_chord.randomize_values(0, 11); // Num chords
        self.performance_roll.randomize_values(0, 17); // Num rolls
        self.performance_random.randomize_values(0, 100);
        // Modulation: 0-127
        self.modulation.randomize_values(0, 127);
    }
}

impl Default for StepSequencerData {
    fn default() -> Self {
        Self {
            pitch: SequencerLane::new(16, 60), // C3
            velocity: SequencerLane::new(16, 100),
            gate: SequencerLane::new(16, 0.5),
            probability: SequencerLane::new(16, 100), // Default 100%
            
            performance_octave: SequencerLane::new(16, 0),
            performance_bend: SequencerLane::new(16, 0),
            performance_chord: SequencerLane::new(16, 0),
            performance_roll: SequencerLane::new(16, 0),
            performance_random: SequencerLane::new(16, 0), // 0% random by default
            
            random_mask_global: 0,
            modulation: SequencerLane::new(16, 0),
            modulation_targets: Vec::new(),
            muted: vec![false; 16],
            active_modulation_target_index: 0,
            root_key: 60,
            scale: ScaleType::default(),
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Timestamp {
    pub samples: u64,       // Absolute position in samples
    pub fractional: f64,    // Sub-sample precision for resampling
}

impl Default for Timestamp {
    fn default() -> Self {
        Self { samples: 0, fractional: 0.0 }
    }
}

impl Timestamp {
    pub fn new(samples: u64, fractional: f64) -> Self {
        Self { samples, fractional }
    }

    pub fn zero() -> Self {
        Self::default()
    }
    
    pub fn from_seconds(seconds: f64, sample_rate: f64) -> Self {
        let total_samples = seconds * sample_rate;
        let samples = total_samples.floor() as u64;
        let fractional = total_samples - samples as f64;
        Self { samples, fractional }
    }

    pub fn as_seconds(&self, sample_rate: f64) -> f64 {
        (self.samples as f64 + self.fractional) / sample_rate
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WarpMarker {
    pub source_sample: u64,
    pub timeline_beat: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArrangementClip {
    pub start_time: Timestamp, 
    pub length: Timestamp,     
    pub start_offset: Timestamp, 
    pub source_id: u32,        // ID in Audio Pool (0 = none/midi?)
    pub name: String,
    pub selected: bool,
    pub warp_markers: Vec<WarpMarker>,
    
    // Time Stretching
    #[serde(default)]
    pub stretch: bool,
    #[serde(default)]
    pub stretch_ratio: f32, // 1.0 = normal, 0.5 = half speed, 2.0 = double speed
    #[serde(default = "default_bpm")]
    pub original_bpm: f32,
        
    #[serde(skip)]
    pub cached_id: Option<u32>, // Runtime ID of the stretched asset
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TrackArrangement {
    pub clips: Vec<ArrangementClip>,
    // Automation curves will go here later
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
    pub arrangement: TrackArrangement,
    
    #[serde(default)]
    pub plugin_state: Option<Vec<u8>>,
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
            arrangement: TrackArrangement::default(),
            plugin_state: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub name: String,
    pub bpm: f32,
    pub tracks: Vec<Track>,
    pub arrangement_mode: bool, // Added
}

impl Default for Project {
    fn default() -> Self {
        Self {
            name: "New Project".to_string(),
            bpm: 120.0,
            tracks: Vec::new(),
            arrangement_mode: false,
        }
    }
}

fn default_bpm() -> f32 { 120.0 }
