
pub struct DelayLine {
    buffer: Vec<f32>,
    write_pos: usize,
}

impl DelayLine {
    pub fn new(max_delay_samples: usize, _sample_rate: f32) -> Self {
        Self {
            buffer: vec![0.0; max_delay_samples],
            write_pos: 0,
        }
    }

    #[allow(dead_code)]
    pub fn process(&mut self, input: &[f32], output: &mut [f32], delay_samples: u32) {
        let n = input.len();
        let buffer_len = self.buffer.len();
        
        let delay = delay_samples as usize;
        // Clamp delay to buffer size
        let delay = delay.min(buffer_len - 1); 
        
        // Read Pointer
        // If delay is 0, we can copy? But we still need buffering if delay changes dynamically?
        // PDC delay usually changes rarely.
        if delay == 0 {
             output.copy_from_slice(input);
             // Also update buffer for smooth transitions if needed, but for PDC 0 is common.
             // But if we jump from 0 to 100, we need history.
             // So we ALWAYS write to buffer.
        }

        for i in 0..n {
             // Write
             self.buffer[self.write_pos] = input[i];
             
             // Read
             let read_pos = (self.write_pos + buffer_len - delay) % buffer_len;
             output[i] = self.buffer[read_pos];
             
             // Advance
             self.write_pos = (self.write_pos + 1) % buffer_len;
        }
    }

    pub fn process_in_place(&mut self, buffer: &mut [f32], delay_samples: u32) {
        let n = buffer.len();
        let buffer_len = self.buffer.len();
        let delay = (delay_samples as usize).min(buffer_len - 1);

        for i in 0..n {
             let sample = buffer[i];
             self.buffer[self.write_pos] = sample;
             
             let read_pos = (self.write_pos + buffer_len - delay) % buffer_len;
             buffer[i] = self.buffer[read_pos];
             
             self.write_pos = (self.write_pos + 1) % buffer_len;
        }
    }
    
    pub fn resize(&mut self, new_size: usize) {
        if new_size > self.buffer.len() {
            self.buffer.resize(new_size, 0.0);
        }
    }
}
