use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RollSubStep {
    Rest,
    Play,     // Original pitch
    PlayUp,   // +1 semitone
    PlayDown, // -1 semitone
}

#[derive(Debug, Clone, Copy)]
pub struct RollPattern {
    pub steps: [RollSubStep; 4],
}

impl RollPattern {
    pub fn get(id: u8) -> Self {
        match id {
            // Rhythmic Variations (Normal Pitch)
            0 => Self::new([RollSubStep::Play, RollSubStep::Play, RollSubStep::Play, RollSubStep::Play]), // ****
            1 => Self::new([RollSubStep::Play, RollSubStep::Play, RollSubStep::Play, RollSubStep::Rest]), // ***_
            2 => Self::new([RollSubStep::Play, RollSubStep::Play, RollSubStep::Rest, RollSubStep::Play]), // **_*
            3 => Self::new([RollSubStep::Play, RollSubStep::Rest, RollSubStep::Play, RollSubStep::Play]), // *__*
            4 => Self::new([RollSubStep::Rest, RollSubStep::Play, RollSubStep::Play, RollSubStep::Play]), // _***
            5 => Self::new([RollSubStep::Play, RollSubStep::Rest, RollSubStep::Play, RollSubStep::Rest]), // *_*_
            6 => Self::new([RollSubStep::Rest, RollSubStep::Play, RollSubStep::Rest, RollSubStep::Play]), // _*_*
            7 => Self::new([RollSubStep::Play, RollSubStep::Play, RollSubStep::Rest, RollSubStep::Rest]), // **__
            8 => Self::new([RollSubStep::Rest, RollSubStep::Rest, RollSubStep::Play, RollSubStep::Play]), // __**
            9 => Self::new([RollSubStep::Play, RollSubStep::Rest, RollSubStep::Rest, RollSubStep::Rest]), // *___
            
            // Pitch Variations (Up)
            10 => Self::new([RollSubStep::Play, RollSubStep::PlayUp, RollSubStep::Play, RollSubStep::PlayUp]), // *^*^
            11 => Self::new([RollSubStep::Play, RollSubStep::PlayUp, RollSubStep::PlayUp, RollSubStep::PlayUp]), // *^^^
            12 => Self::new([RollSubStep::PlayUp, RollSubStep::Play, RollSubStep::PlayUp, RollSubStep::Play]), // ^*^*
            13 => Self::new([RollSubStep::Play, RollSubStep::Play, RollSubStep::PlayUp, RollSubStep::PlayUp]), // **^^
            14 => Self::new([RollSubStep::PlayUp, RollSubStep::PlayUp, RollSubStep::PlayUp, RollSubStep::PlayUp]), // ^^^^
            
            // Pitch Variations (Down)
            15 => Self::new([RollSubStep::Play, RollSubStep::PlayDown, RollSubStep::Play, RollSubStep::PlayDown]), // *v*v
            16 => Self::new([RollSubStep::Play, RollSubStep::PlayDown, RollSubStep::PlayDown, RollSubStep::PlayDown]), // *vvv
            17 => Self::new([RollSubStep::PlayDown, RollSubStep::Play, RollSubStep::PlayDown, RollSubStep::Play]), // v*v*
            18 => Self::new([RollSubStep::Play, RollSubStep::Play, RollSubStep::PlayDown, RollSubStep::PlayDown]), // **vv
            19 => Self::new([RollSubStep::PlayDown, RollSubStep::PlayDown, RollSubStep::PlayDown, RollSubStep::PlayDown]), // vvvv

            _ => Self::new([RollSubStep::Play; 4]),
        }
    }

    fn new(steps: [RollSubStep; 4]) -> Self {
        Self { steps }
    }
}

pub struct BendShape;

impl BendShape {
    /// Returns pitch offset in semitones for a given shape ID and phase (0.0 to 1.0)
    pub fn get_value(id: u8, phase: f32) -> f32 {
        let p = phase.clamp(0.0, 1.0);
        match id {
            // Basic Ramps (Unipolar / Bipolar)
            0 => p * 2.0,           // Ramp Up (+2st)
            1 => -p * 2.0,          // Ramp Down (-2st)
            2 => (p - 0.5) * 4.0,   // Ramp Up Bipolar (-2 to +2)
            3 => (0.5 - p) * 4.0,   // Ramp Down Bipolar (+2 to -2)
            
            // Log/Exp Curves
            4 => p.powf(2.0) * 2.0, // Slow Start Up
            5 => p.powf(0.5) * 2.0, // Fast Start Up
            
            // Triangle / Spikes
            6 => if p < 0.5 { p * 4.0 } else { 2.0 - (p - 0.5) * 4.0 }, // Triangle Up (0 -> 2 -> 0)
            7 => if p < 0.5 { -p * 4.0 } else { -2.0 + (p - 0.5) * 4.0 }, // Triangle Down (0 -> -2 -> 0)
            
            // Vibrato (Sine)
            8 => (p * std::f32::consts::TAU).sin() * 0.5, // 1 Cycle Vibrato (+/- 0.5st)
            9 => (p * std::f32::consts::TAU * 2.0).sin() * 0.5, // 2 Cycle Vibrato (+/- 0.5st)
            10 => (p * std::f32::consts::TAU * 4.0).sin() * 0.25, // 4 Cycle Vibrato Fast (+/- 0.25st)
            
            // Shapes
            11 => if p < 0.5 { 1.0 } else { 0.0 }, // Square (Up then return)
            12 => if p < 0.25 { p * 8.0 } else if p < 0.75 { 2.0 } else { 2.0 - (p - 0.75) * 8.0 }, // Trap Up
            
            // Wiggles
            13 => (p * std::f32::consts::TAU * 3.0).sin() * p, // Growing Sine
            14 => (p * std::f32::consts::TAU * 3.0).sin() * (1.0 - p), // Decaying Sine
            
            // Random-ish / Complex
            15 => if p < 0.3 { p * 3.33 } else if p < 0.6 { 1.0 - (p - 0.3) * 6.66 } else { -1.0 + (p - 0.6) * 2.5 }, 
            
            // Extremes
            16 => p * 12.0, // Full Octave Up
            17 => -p * 12.0, // Full Octave Down
            18 => if p < 0.5 { p * 24.0 } else { 12.0 - (p - 0.5) * 24.0 }, // Octave Spike
            
            19 => 0.0,
            _ => 0.0,
        }
    }
}
