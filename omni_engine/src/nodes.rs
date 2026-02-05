// use std::f32::consts::PI;
use omni_shared::MidiNoteEvent;

pub trait AudioNode: Send + Sync {
    /// Process a block of audio.
    /// inputs: vector of input buffers (summed before this call ideally, or handled inside).
    /// output: buffer to write to.
    /// midi_events: slice of MIDI events for this block.
    /// param_events: slice of Parameter events for this block.
    fn process(&mut self, output: &mut [f32], sample_rate: f32, midi_events: &[MidiNoteEvent], param_events: &[omni_shared::ParameterEvent]);
    
    /// Handle parameters (simplified)
    fn set_param(&mut self, _id: u32, _value: f32) {}

    /// Get plugin parameters if this is a plugin node
    fn get_plugin_params(&mut self) -> Vec<omni_shared::ParamInfo> { Vec::new() }

    /// Testing: Kill the child process
    fn simulate_crash(&mut self) {}

    /// Open Native Editor
    fn open_editor(&mut self) {}

    /// Get note names from plugin (CLAP note_name extension)
    /// Returns: (clap_id, note_names) - clap_id used for fallback mappings
    fn get_note_names(&mut self) -> (String, Vec<omni_shared::NoteNameInfo>) { (String::new(), Vec::new()) }

    /// Get last touched parameter (for learning)
    /// Returns: (param_id, value, generation)
    fn get_last_touched(&self) -> (u32, f32, u32) { (0, 0.0, 0) }
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
    fn process(&mut self, output: &mut [f32], sample_rate: f32, _midi_events: &[omni_shared::MidiNoteEvent], _param_events: &[omni_shared::ParameterEvent]) {
        let frames = output.len() / 2;
        let (l, r) = output.split_at_mut(frames);
        
        for i in 0..frames {
            let s = (self.phase * 2.0 * std::f32::consts::PI).sin();
            l[i] = s;
            r[i] = s;
            self.phase = (self.phase + self.frequency / sample_rate) % 1.0;
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
    fn process(&mut self, output: &mut [f32], _sample_rate: f32, _midi_events: &[omni_shared::MidiNoteEvent], _param_events: &[omni_shared::ParameterEvent]) {
        // Assuming the output buffer is interleaved stereo (LRLR...) for simplicity
        // If it's planar, the AudioNode trait's process signature would need to change
        // to accept multiple buffers (e.g., `&mut [&mut [f32]]`).
        // For a single `&mut [f32]`, we apply gain to all samples.
        for sample in output.iter_mut() {
            *sample *= self.gain;
        }
    }
    
    fn set_param(&mut self, _id: u32, value: f32) {
         // Assuming id 0 is gain
         self.gain = value;
    }
}
