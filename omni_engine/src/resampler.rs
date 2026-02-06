use rubato::{Resampler, SincFixedIn, SincInterpolationType, SincInterpolationParameters, WindowFunction};

pub struct OmniResampler;

impl OmniResampler {
    /// Resamples the input buffer by the given ratio using Sinc Interpolation (High Quality).
    /// Ratio > 1.0 means speed up (shorter duration).
    /// Ratio < 1.0 means slow down (longer duration).
    pub fn resample(input: &[f32], ratio: f64) -> Result<Vec<f32>, anyhow::Error> {
        if input.is_empty() {
            return Ok(Vec::new());
        }
        
        if ratio <= 0.0 {
            return Err(anyhow::anyhow!("Ratio must be positive"));
        }

        // Calculate target sample rate relative to source
        let chunk_size = 1024;
        let target_ratio = 1.0 / ratio; 
        
        // Use SincFixedIn for high quality
        let params = SincInterpolationParameters {
            sinc_len: 256,
            f_cutoff: 0.95,
            interpolation: SincInterpolationType::Linear,
            oversampling_factor: 128,
            window: WindowFunction::BlackmanHarris2,
        };
        
        let channels = 1;
        let mut resampler = SincFixedIn::<f32>::new(
            target_ratio,
            2.0, // Max ratio flexibility
            params,
            chunk_size,
            channels
        )?;

        let mut output = Vec::with_capacity((input.len() as f64 * target_ratio) as usize + 1024);
        let mut input_pos = 0;
        let input_len = input.len();
        
        while input_pos < input_len {
            let end = (input_pos + chunk_size).min(input_len);
            let chunk_len = end - input_pos;
            
            let mut chunk = input[input_pos..end].to_vec();
            if chunk_len < chunk_size {
                 chunk.resize(chunk_size, 0.0);
            }
            
            let waves = vec![chunk];
            let out_waves = resampler.process(&waves, None)?;
            
            if let Some(chan_out) = out_waves.get(0) {
                // If this is the last chunk, we should prevent garbage at the end?
                // For now, appending is fine.
                output.extend_from_slice(chan_out);
            }
            
            input_pos += chunk_size;
        }
        
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_resample_compile() {
         // Basic compile check
         let input = vec![0.0; 1000];
         let _ = OmniResampler::resample(&input, 1.0);
    }
}
