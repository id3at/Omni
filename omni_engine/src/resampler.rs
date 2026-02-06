/// Simple audio resampler for time stretching.
/// Uses linear interpolation for MVP. 
/// For higher quality, consider integrating a validated rubato version or another library.

pub struct OmniResampler;

impl OmniResampler {
    /// Resamples the input buffer by the given ratio using linear interpolation.
    /// Ratio > 1.0 means speed up (shorter duration, fewer samples).
    /// Ratio < 1.0 means slow down (longer duration, more samples).
    /// 
    /// Note: Linear interpolation introduces some aliasing. For production use,
    /// consider more sophisticated algorithms (polyphase, sinc, etc.).
    pub fn resample(input: &[f32], ratio: f64) -> Result<Vec<f32>, anyhow::Error> {
        if input.is_empty() {
            return Ok(Vec::new());
        }
        
        if ratio <= 0.0 {
            return Err(anyhow::anyhow!("Ratio must be positive"));
        }
        
        // Output length: input_len / ratio
        // Speed up (ratio 2.0): half as many samples
        // Slow down (ratio 0.5): twice as many samples
        let output_len = ((input.len() as f64) / ratio).ceil() as usize;
        
        if output_len == 0 {
            return Ok(Vec::new());
        }
        
        let mut output = Vec::with_capacity(output_len);
        
        for i in 0..output_len {
            // Map output index to input position
            let src_pos = i as f64 * ratio;
            let src_idx = src_pos.floor() as usize;
            let frac = src_pos.fract() as f32;
            
            // Get samples (with bounds checking)
            let s0 = input.get(src_idx).copied().unwrap_or(0.0);
            let s1 = input.get(src_idx + 1).copied().unwrap_or(s0); // Repeat last sample if OOB
            
            // Linear interpolation
            let sample = s0 + frac * (s1 - s0);
            output.push(sample);
        }
        
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_resample_speedup() {
        let input: Vec<f32> = (0..100).map(|i| i as f32).collect();
        let result = OmniResampler::resample(&input, 2.0).unwrap();
        assert_eq!(result.len(), 50);
    }
    
    #[test]
    fn test_resample_slowdown() {
        let input: Vec<f32> = (0..100).map(|i| i as f32).collect();
        let result = OmniResampler::resample(&input, 0.5).unwrap();
        assert_eq!(result.len(), 200);
    }
    
    #[test]
    fn test_resample_no_change() {
        let input: Vec<f32> = (0..100).map(|i| i as f32).collect();
        let result = OmniResampler::resample(&input, 1.0).unwrap();
        assert_eq!(result.len(), 100);
    }
}
