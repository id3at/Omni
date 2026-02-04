// use omni_shared::MidiNoteEvent;

#[derive(Clone)]
pub struct Sequencer {
    /// BPM (beats per minute)
    pub bpm: f32,
    /// Steps per beat (e.g., 4 = sixteenth notes)
    pub steps_per_beat: u32,
    /// Current step position (0-15)
    current_step: u32,
    /// Total steps in pattern
    pub pattern_length: u32,
    /// Sample position within current step
    samples_in_step: u64,
}

impl Sequencer {
    pub fn new(bpm: f32) -> Self {
        Self {
            bpm,
            steps_per_beat: 4, // 16th notes
            current_step: 0,
            pattern_length: 16,
            samples_in_step: 0,
        }
    }
    
    /// Calculate samples per step based on BPM and sample rate
    fn samples_per_step(&self, sample_rate: f32) -> u64 {
        let beats_per_second = self.bpm / 60.0;
        let steps_per_second = beats_per_second * self.steps_per_beat as f32;
        let samples_per_step = sample_rate / steps_per_second;
        samples_per_step as u64
    }
    
    /// Advance the sequencer by a number of samples and return the current step if a boundary was crossed
    pub fn advance(&mut self, samples: usize, sample_rate: f32) -> Option<(u32, u32)> {
        // Returns (current_step, sample_offset_within_buffer)
        // Note: For high precision, we might need multiple events per buffer.
        // For now, let's just return the LAST step boundary crossed, 
        // or simplistic logic: if we cross a boundary, we return the new step index.
        
        let samples_per_step = self.samples_per_step(sample_rate);
        let mut new_step = None;
        
        self.samples_in_step += samples as u64;
        
        while self.samples_in_step >= samples_per_step {
            self.samples_in_step -= samples_per_step;
            self.current_step = (self.current_step + 1) % self.pattern_length;
            new_step = Some((self.current_step, 0)); // Offset 0 for now (simplify)
        }
        
        new_step
    }
    
    /// Reset the sequencer to the beginning
    pub fn reset(&mut self) {
        self.current_step = 0;
        self.samples_in_step = 0;
    }
    
    /// Get current step (0-indexed)
    pub fn current_step(&self) -> u32 {
        self.current_step
    }
    
    pub fn set_length_in_beats(&mut self, beats: f64) {
        self.pattern_length = (beats * self.steps_per_beat as f64) as u32;
    }
}

use omni_shared::project::SequencerDirection;

pub struct StepGenerator;

impl StepGenerator {
    /// Calculate the next step index for a lane
    /// 
    /// # Arguments
    /// * `counter` - A monotonically increasing counter (e.g. total steps played for this lane or global step)
    /// * `direction` - The direction mode
    /// * `loop_start` - Index where loop starts
    /// * `loop_end` - Index where loop ends (exclusive, i.e. length + start)
    /// 
    /// # Returns
    /// The step index to play (0-based relative to the array).
    pub fn get_step_index(
        counter: u64, 
        direction: SequencerDirection, 
        loop_start: u32, 
        loop_end: u32
    ) -> usize {
        // Validation
        if loop_end <= loop_start {
            return loop_start as usize;
        }
        
        let len = (loop_end - loop_start) as u64;
        let start = loop_start as u64;
        
        match direction {
            SequencerDirection::Forward => {
                let offset = counter % len;
                (start + offset) as usize
            },
            SequencerDirection::Backward => {
                let offset = counter % len;
                // len - 1 - offset maps 0 -> len-1, 1 -> len-2
                (start + (len - 1 - offset)) as usize
            },
            SequencerDirection::Random => {
                // Deterministic random based on counter? Or truly random?
                // For a pure function, we need deterministic or passed state.
                // Using fastrand with seed from counter makes it repeatable per position, which is nice for freezing.
                // But "Random" usually implies evolving. 
                // Let's use a hash of counter to pick a step.
                let rnd = (counter * 48271) % len; // Simple LCG
                (start + rnd) as usize
            },
            SequencerDirection::Each2nd => {
                // 0, 2, 4... then 1, 3, 5...
                // Only if len is even? Thesys docs say: 
                // "For sequences with an even number of steps... 
                // first run: 1, 3, 5 (indices 0, 2, 4)
                // first repeat: 2, 4, 6 (indices 1, 3, 5)"
                
                // Effective length is len.
                // We want to traverse strides of 2.
                // Stride formula: (counter * step) % len? No.
                // If len=4: 0, 2, 1, 3, 0, 2...
                // This is (counter * 2) % len_if_odd?
                
                // General solution for "Each Nth" where we visit all pixels with stride N:
                // If N and Len are coprime, simple stride coverage.
                // If not (e.g. 2 and 16), we need to shift offset.
                
                let stride = 2;
                Self::calculate_interleaved_step(counter, len, stride) as usize + start as usize
            },
            SequencerDirection::Each3rd => {
                 let stride = 3;
                 Self::calculate_interleaved_step(counter, len, stride) as usize + start as usize
            },
            SequencerDirection::Each4th => {
                 let stride = 4;
                 Self::calculate_interleaved_step(counter, len, stride) as usize + start as usize
            },
        }
    }

    fn calculate_interleaved_step(counter: u64, len: u64, stride: u64) -> u64 {
        if len == 0 { return 0; }
        
        // GCD helper
        let gcd = |mut a: u64, mut b: u64| {
            while b != 0 {
                let t = b;
                b = a % b;
                a = t;
            }
            a
        };
        
        let g = gcd(len, stride);
        
        if g == 1 {
            (counter * stride) % len
        } else {
            // We have 'g' passes.
            // Each pass has 'len / g' steps.
            let steps_per_pass = len / g;
            
            // Current pass index (cycle through 0..g)
            let pass_idx = (counter / steps_per_pass) % g;
            
            // Step within current pass
            let step_in_pass = counter % steps_per_pass;
            
            ((step_in_pass * stride) + pass_idx) % len
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omni_shared::project::SequencerDirection::*;

    #[test]
    fn test_forward() {
        // Len 4: 0, 1, 2, 3, 0
        let s = Vec::from_iter((0..5).map(|i| StepGenerator::get_step_index(i, Forward, 0, 4)));
        assert_eq!(s, vec![0, 1, 2, 3, 0]);
    }

    #[test]
    fn test_backward() {
        // Len 4: 3, 2, 1, 0, 3
        let s = Vec::from_iter((0..5).map(|i| StepGenerator::get_step_index(i, Backward, 0, 4)));
        assert_eq!(s, vec![3, 2, 1, 0, 3]);
    }
    
    #[test]
    fn test_each_2nd_even_len() {
        // Len 4: 0, 2, 1, 3, 0
        // Counter 0: (0*2 + 0) % 4 = 0
        // Counter 1: (1*2 + 0) % 4 = 2
        // Counter 2: (2*2 + 1) % 4 = 1  <- Phase shift happens here because 4/4 = 1
        // Counter 3: (3*2 + 1) % 4 = 7%4 = 3
        // Counter 4: (4*2 + 2) % 4 = 10%4 = 2 -- Wait, expected 0.
        
        // My formula:
        // C=4: raw=8, phase=2. (8+2)%4 = 2.
        // It failed to wrap back to 0 perfectly at 4?
        // Let's trace Thesys: "first run... 1, 3, 5 (indices 0, 2, 4)" (Len 6?)
        // If Len 4. Run 1: 0, 2. Run 2: 1, 3.
        // Sequence: 0, 2, 1, 3.
        
        let s = Vec::from_iter((0..6).map(|i| StepGenerator::get_step_index(i, Each2nd, 0, 4)));
        // GCD(4, 2) = 2.
        // Steps per pass = 4/2 = 2.
        // C=0: p=0, s=0. (0*2+0)%4 = 0
        // C=1: p=0, s=1. (1*2+0)%4 = 2
        // C=2: p=1, s=0. (0*2+1)%4 = 1
        // C=3: p=1, s=1. (1*2+1)%4 = 3
        // C=4: p=0, s=0. 0
        // C=5: p=0, s=1. 2
        assert_eq!(s, vec![0, 2, 1, 3, 0, 2]);
    }
}

