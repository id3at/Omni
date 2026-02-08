use ringbuf::HeapProd;
use omni_shared::{MidiNoteEvent, ExpressionEvent, ParameterEvent, MAX_EXPRESSION_EVENTS, MAX_PARAM_EVENTS};

pub struct AudioBuffers {
    pub track_bufs: Vec<Vec<f32>>,
    pub track_vols: Vec<f32>,
    pub track_pans: Vec<f32>,
    pub track_events: Vec<Vec<MidiNoteEvent>>,
    pub track_expression_events: Vec<Vec<ExpressionEvent>>,
    pub track_param_events: Vec<Vec<ParameterEvent>>,
    pub master_mix: Vec<f32>,
    // Real-Time Safety: Use RingBuffer Producers instead of Vec
    pub recording_producers: Vec<Option<HeapProd<f32>>>,
}

impl AudioBuffers {
    pub fn new(max_tracks: usize, buffer_size: usize) -> Self {
        Self {
            track_bufs: vec![vec![0.0; buffer_size]; max_tracks],
            track_vols: vec![1.0; max_tracks],
            track_pans: vec![0.0; max_tracks],
            track_events: vec![Vec::with_capacity(128); max_tracks],
            track_expression_events: vec![Vec::with_capacity(MAX_EXPRESSION_EVENTS); max_tracks],
            track_param_events: vec![Vec::with_capacity(MAX_PARAM_EVENTS); max_tracks],
            master_mix: vec![0.0; buffer_size],
            recording_producers: (0..max_tracks).map(|_| None).collect(),
        }
    }

    pub fn prepare_buffers(&mut self, frames: usize, track_count: usize, max_buffer_size: usize) {
        // Resize Buffers (Keep Capacity)
        // Master Mix
        if self.master_mix.len() != frames * 2 {
            self.master_mix.resize(frames * 2, 0.0);
        }
        self.master_mix.fill(0.0);

        // Track Data
        self.track_vols.resize(track_count, 1.0);
        self.track_pans.resize(track_count, 0.0);
        

        
        // Resize vectors of vectors
        if self.track_events.len() < track_count {
            self.track_events.resize(track_count, Vec::with_capacity(128));
        }
        if self.track_expression_events.len() < track_count {
            self.track_expression_events.resize(track_count, Vec::with_capacity(MAX_EXPRESSION_EVENTS));
        }
        if self.track_param_events.len() < track_count {
            self.track_param_events.resize(track_count, Vec::with_capacity(MAX_PARAM_EVENTS));
        }
        
        // Resize audio buffers
        if self.track_bufs.len() < track_count {
            self.track_bufs.resize(track_count, vec![0.0; max_buffer_size]);
        }

        // Clear Events and Fill Audio
        for i in 0..track_count {
            self.track_events[i].clear();
            self.track_expression_events[i].clear();
            self.track_param_events[i].clear();
            
            // Resize buffer for this frame
            if self.track_bufs[i].len() != frames * 2 {
                 // This keeps capacity if it's large enough
                 self.track_bufs[i].resize(frames * 2, 0.0);
            }
            self.track_bufs[i].fill(0.0);
        }
    }

    /// Static method to mix tracks to master to avoid borrow checker conflicts with `self`
    pub fn mix_to_master(
        track_bufs: &[Vec<f32>], 
        master_mix: &mut [f32], 
        track_vols: &[f32], 
        track_pans: &[f32], 
        frames: usize, 
        track_count: usize
    ) {
        for (t_idx, track_buf) in track_bufs.iter().take(track_count).enumerate() {
             let vol = track_vols[t_idx];
             let pan = track_pans[t_idx];
             
             for i in 0..frames {
                 let left = track_buf[i * 2];
                 let right = track_buf[i * 2 + 1];
                 
                 let mut l_gain = vol;
                 let mut r_gain = vol;
                 
                 if pan > 0.0 {
                     l_gain *= 1.0 - pan;
                 } else if pan < 0.0 {
                     r_gain *= 1.0 + pan;
                 }
                 
                 master_mix[i*2] += left * l_gain;
                 master_mix[i*2+1] += right * r_gain;
             }
         }
    }
}
