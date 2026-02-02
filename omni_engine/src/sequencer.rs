use omni_shared::MidiNoteEvent;

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
    
}
