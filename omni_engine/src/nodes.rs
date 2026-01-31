use std::f32::consts::PI;

pub trait AudioNode: Send + Sync {
    /// Process a block of audio.
    /// inputs: vector of input buffers (summed before this call ideally, or handled inside).
    /// output: buffer to write to.
    /// For simplicity in this iteration: 
    /// We assume mono/stereo generic, but let's stick to mono f32 for the prototype graph logic.
    fn process(&mut self, output: &mut [f32], sample_rate: f32);
    
    /// Handle parameters (simplified)
    fn set_param(&mut self, _id: u32, _value: f32) {}
}

pub struct SineNode {
    pub phase: f32,
    pub frequency: f32,
}

impl SineNode {
    pub fn new(frequency: f32) -> Self {
        Self { phase: 0.0, frequency }
    }
}

impl AudioNode for SineNode {
    fn process(&mut self, output: &mut [f32], sample_rate: f32) {
        let phase_inc = self.frequency * 2.0 * PI / sample_rate;
        for sample in output.iter_mut() {
            self.phase = (self.phase + phase_inc) % (2.0 * PI);
            *sample = self.phase.sin();
        }
    }
}

pub struct GainNode {
    pub gain: f32,
}

impl GainNode {
    pub fn new(gain: f32) -> Self {
        Self { gain }
    }
}

impl AudioNode for GainNode {
    fn process(&mut self, output: &mut [f32], _sample_rate: f32) {
        for sample in output.iter_mut() {
            *sample *= self.gain;
        }
    }
    
    fn set_param(&mut self, _id: u32, value: f32) {
         // Assuming id 0 is gain
         self.gain = value;
    }
}
