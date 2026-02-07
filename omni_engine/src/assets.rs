use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone)]
pub struct AudioAsset {
    pub id: u32,
    pub path: String,
    pub data: Arc<Vec<f32>>, // Mono or Interleaved Stereo? Let's assume Mono for now or handle channels
    pub channels: u16,
    pub sample_rate: u32,
    pub duration_seconds: f64,
    pub original_bpm: Option<f32>, // Metadata for stretching
}

#[derive(Clone)]
pub struct AudioPool {
    assets: HashMap<u32, AudioAsset>,
    next_id: u32,
    path_cache: HashMap<String, u32>, // Path -> ID mapping to avoid duplicates
    stretched_cache: HashMap<(u32, u32), u32>, // (Source ID, Ratio * 1000) -> Stretched ID
}

impl AudioPool {
    pub fn new() -> Self {
        Self {
            assets: HashMap::new(),
            next_id: 1, // Start from 1, 0 is reserved/null
            path_cache: HashMap::new(),
            stretched_cache: HashMap::new(),
        }
    }

    pub fn load_asset(&mut self, path: &str) -> Result<u32, anyhow::Error> {
        // Check cache
        if let Some(&id) = self.path_cache.get(path) {
            return Ok(id);
        }

        // Initialize reader to get spec
        let reader = hound::WavReader::open(path)?;
        let spec = reader.spec();
        let channels = spec.channels;
        let sample_rate = spec.sample_rate;
        // Re-open for robust loading below
            
        // Wait, hound defaults to i16/i32 usually. 
        // Let's reload reader to be sure about format or use a helper.
        // Actually, let's implement a robust loader.
        
        // Re-open for robust loading
        let reader = hound::WavReader::open(path)?;
        let spec = reader.spec();
        let raw_samples: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Float => reader.into_samples::<f32>().collect::<Result<Vec<_>, _>>()?,
            hound::SampleFormat::Int => {
                let bit_depth = spec.bits_per_sample;
                let max_val = 2.0_f32.powi(bit_depth as i32 - 1);
                reader.into_samples::<i32>()
                    .map(|s| s.map(|x| x as f32 / max_val))
                    .collect::<Result<Vec<_>, _>>()?
            }
        };

        let duration = raw_samples.len() as f64 / (channels as f64 * sample_rate as f64);
        
        let id = self.next_id;
        self.next_id += 1;

        let asset = AudioAsset {
            id,
            path: path.to_string(),
            data: Arc::new(raw_samples),
            channels,
            sample_rate,
            duration_seconds: duration,
            original_bpm: None, // TODO: Read from WAV metadata/fmt chuck if possible, or user input
        };

        self.assets.insert(id, asset);
        self.path_cache.insert(path.to_string(), id);
        
        eprintln!("[AudioPool] Loaded asset {}: {} ({}s)", id, path, duration);

        Ok(id)
    }

    pub fn get_asset(&self, id: u32) -> Option<&AudioAsset> {
        self.assets.get(&id)
    }

    pub fn get_or_create_stretched(&mut self, source_id: u32, ratio: f32) -> Result<u32, anyhow::Error> {
        let ratio_key = (ratio * 1000.0) as u32;
        
        // 1. Check Cache
        if let Some(&id) = self.stretched_cache.get(&(source_id, ratio_key)) {
            return Ok(id);
        }

        // 2. Get Source Data
        let (source_data, channels, sr, path) = {
            let asset = self.assets.get(&source_id).ok_or_else(|| anyhow::anyhow!("Asset not found"))?;
            (asset.data.clone(), asset.channels, asset.sample_rate, asset.path.clone())
        };

        // 3. Resample
        let stretched_data = crate::resampler::OmniResampler::resample(&source_data, ratio as f64)?;
        
        // 4. Create New Asset
        let id = self.next_id;
        self.next_id += 1;
        
        let duration = stretched_data.len() as f64 / (channels as f64 * sr as f64);
        
        let new_asset = AudioAsset {
            id,
            path: format!("{} [Stretched {:.2}x]", path, ratio),
            data: Arc::new(stretched_data),
            channels,
            sample_rate: sr,
            duration_seconds: duration,
            original_bpm: None, 
        };

        self.assets.insert(id, new_asset);
        self.stretched_cache.insert((source_id, ratio_key), id);
        
        Ok(id)
    }
    
    /// Create an AudioAsset from raw sample data (used for recording).
    pub fn add_asset_from_data(&mut self, data: Vec<f32>, sample_rate: f32) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        
        let duration = data.len() as f64 / sample_rate as f64;
        
        let asset = AudioAsset {
            id,
            path: format!("[Recorded {}]", id),
            data: Arc::new(data),
            channels: 1, // Mono recording
            sample_rate: sample_rate as u32,
            duration_seconds: duration,
            original_bpm: None,
        };
        
        self.assets.insert(id, asset);
        eprintln!("[AudioPool] Created recorded asset {}: {}s", id, duration);
        
        id
    }
}
