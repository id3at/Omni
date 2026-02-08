//! Offline audio export/bounce module.
//! Renders the arrangement to WAV files without real-time constraints.

use hound::{WavSpec, WavWriter, SampleFormat};
use std::path::Path;

/// Export format options
#[derive(Debug, Clone, Copy)]
pub enum ExportBitDepth {
    Int16,
    Int24,
    Float32,
}

/// Export configuration
#[derive(Debug, Clone)]
pub struct ExportConfig {
    pub sample_rate: u32,
    pub channels: u16,
    pub bit_depth: ExportBitDepth,
    pub normalize: bool,
    pub dither: bool,          // Apply TPDF dither when converting to int
    pub tail_seconds: f64,     // Extra tail for reverb/delay
}

impl Default for ExportConfig {
    fn default() -> Self {
        Self {
            sample_rate: 48000,
            channels: 2,
            bit_depth: ExportBitDepth::Int24,
            normalize: false,
            dither: true,
            tail_seconds: 2.0,
        }
    }
}

/// Write interleaved f32 audio data to a WAV file.
/// Handles bit-depth conversion, normalization, and dithering.
pub fn write_wav(
    path: &Path,
    data: &[f32],       // Interleaved stereo
    config: &ExportConfig,
) -> Result<(), anyhow::Error> {
    let (bits_per_sample, sample_format) = match config.bit_depth {
        ExportBitDepth::Int16 => (16, SampleFormat::Int),
        ExportBitDepth::Int24 => (24, SampleFormat::Int),
        ExportBitDepth::Float32 => (32, SampleFormat::Float),
    };

    let spec = WavSpec {
        channels: config.channels,
        sample_rate: config.sample_rate,
        bits_per_sample,
        sample_format,
    };

    let mut writer = WavWriter::create(path, spec)?;

    // Find peak for normalization
    let peak = if config.normalize {
        data.iter().fold(0.0f32, |max, &s| max.max(s.abs())).max(1e-10)
    } else {
        1.0
    };
    let norm_gain = if config.normalize { 1.0 / peak } else { 1.0 };

    // Dither state (two independent channels for decorrelation)
    let mut dither_state_l: u32 = 0xDEADBEEF;
    let mut dither_state_r: u32 = 0xCAFEBABE;

    match config.bit_depth {
        ExportBitDepth::Float32 => {
            for &sample in data {
                writer.write_sample(sample * norm_gain)?;
            }
        }
        ExportBitDepth::Int16 => {
            let scale = (1 << 15) as f32 - 1.0;
            for (i, &sample) in data.iter().enumerate() {
                let mut s = sample * norm_gain;
                if config.dither {
                    let state = if i % 2 == 0 { &mut dither_state_l } else { &mut dither_state_r };
                    s += tpdf_dither(state, 16);
                }
                let quantized = (s * scale).round().clamp(-(scale + 1.0), scale) as i32;
                writer.write_sample(quantized as i16)?;
            }
        }
        ExportBitDepth::Int24 => {
            let scale = (1 << 23) as f32 - 1.0;
            for (i, &sample) in data.iter().enumerate() {
                let mut s = sample * norm_gain;
                if config.dither {
                    let state = if i % 2 == 0 { &mut dither_state_l } else { &mut dither_state_r };
                    s += tpdf_dither(state, 24);
                }
                let quantized = (s * scale).round().clamp(-(scale + 1.0), scale) as i32;
                writer.write_sample(quantized)?;
            }
        }
    }

    writer.finalize()?;
    Ok(())
}

/// TPDF dither for target bit depth
#[inline]
fn tpdf_dither(state: &mut u32, bits: u32) -> f32 {
    let r1 = lcg_next(state);
    let r2 = lcg_next(state);
    let lsb = 1.0 / (1u64 << (bits - 1)) as f32;
    (r1 - r2) * lsb
}

#[inline]
fn lcg_next(state: &mut u32) -> f32 {
    *state = state.wrapping_mul(1103515245).wrapping_add(12345);
    (*state as f32) / (u32::MAX as f32) * 2.0 - 1.0
}

/// Stem export: writes individual track buffers as separate WAV files.
pub fn write_stems(
    output_dir: &Path,
    track_names: &[String],
    track_data: &[Vec<f32>],   // Per-track interleaved stereo
    config: &ExportConfig,
) -> Result<Vec<std::path::PathBuf>, anyhow::Error> {
    std::fs::create_dir_all(output_dir)?;
    let mut paths = Vec::new();

    for (i, (name, data)) in track_names.iter().zip(track_data.iter()).enumerate() {
        let safe_name = name.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
        let filename = if safe_name.is_empty() {
            format!("track_{:02}.wav", i + 1)
        } else {
            format!("{}_{:02}.wav", safe_name, i + 1)
        };
        let path = output_dir.join(filename);
        write_wav(&path, data, config)?;
        paths.push(path);
    }

    Ok(paths)
}
