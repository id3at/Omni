use ringbuf::HeapProd;
use omni_shared::{MidiNoteEvent, ExpressionEvent, ParameterEvent, MAX_EXPRESSION_EVENTS, MAX_PARAM_EVENTS};
use std::sync::atomic::{AtomicU32, Ordering};

// ───────────────────────────── Constants ──────────────────────────────
/// TPDF dither amplitude for 24-bit output (±0.5 LSB at 24-bit)
const DITHER_SCALE_24: f32 = 1.0 / (1 << 23) as f32;

// ───────────────────────────── Metering ──────────────────────────────
/// Per-track peak meter values shared with UI thread (atomic f32 as bits)
pub struct PeakMeters {
    /// Peak values per track [left, right, left, right, ...]
    pub peaks: Vec<AtomicU32>,
    /// Master peak [left, right]
    pub master_peak: [AtomicU32; 2],
}

impl PeakMeters {
    pub fn new(max_tracks: usize) -> Self {
        Self {
            peaks: (0..max_tracks * 2).map(|_| AtomicU32::new(0)).collect(),
            master_peak: [AtomicU32::new(0), AtomicU32::new(0)],
        }
    }

    /// Store peak for a track (left/right). RT-safe.
    #[inline]
    pub fn store_track_peak(&self, track: usize, left: f32, right: f32) {
        let idx = track * 2;
        if idx + 1 < self.peaks.len() {
            self.peaks[idx].store(left.to_bits(), Ordering::Relaxed);
            self.peaks[idx + 1].store(right.to_bits(), Ordering::Relaxed);
        }
    }

    /// Load peak for a track. UI-safe.
    pub fn load_track_peak(&self, track: usize) -> (f32, f32) {
        let idx = track * 2;
        if idx + 1 < self.peaks.len() {
            let l = f32::from_bits(self.peaks[idx].load(Ordering::Relaxed));
            let r = f32::from_bits(self.peaks[idx + 1].load(Ordering::Relaxed));
            (l, r)
        } else {
            (0.0, 0.0)
        }
    }

    /// Store master peak. RT-safe.
    #[inline]
    pub fn store_master_peak(&self, left: f32, right: f32) {
        self.master_peak[0].store(left.to_bits(), Ordering::Relaxed);
        self.master_peak[1].store(right.to_bits(), Ordering::Relaxed);
    }

    /// Load master peak. UI-safe.
    pub fn load_master_peak(&self) -> (f32, f32) {
        let l = f32::from_bits(self.master_peak[0].load(Ordering::Relaxed));
        let r = f32::from_bits(self.master_peak[1].load(Ordering::Relaxed));
        (l, r)
    }
}

// ───────────────────── Pan Law Functions ──────────────────────
/// Equal-power (constant-power) pan law.
/// pan ∈ [-1.0, 1.0]: -1 = full left, 0 = center, 1 = full right.
/// At center: both channels ≈ -3 dB (≈ 0.707).
/// Preserves perceived loudness when panning.
#[inline]
pub fn equal_power_pan(pan: f32) -> (f32, f32) {
    // Map pan from [-1, 1] to [0, π/2]
    let angle = (pan + 1.0) * 0.25 * std::f32::consts::PI;
    let l = angle.cos();
    let r = angle.sin();
    (l, r)
}

// ─────────────────── Soft Clipping / Limiting ────────────────────
/// Fast tanh-like soft clipper (polynomial approximation).
/// Smooth saturation near ±1.0 instead of hard digital clipping.
#[inline]
pub fn soft_clip(x: f32) -> f32 {
    if x.abs() <= 1.0 {
        x - (x * x * x) / 3.0 // Cubic soft-knee
    } else {
        x.signum() * (2.0 / 3.0) // Asymptotic limit of tanh approximation
    }
}

/// Hard clip with headroom — prevents DAC overflow.
#[inline]
pub fn hard_clip(x: f32) -> f32 {
    x.clamp(-1.0, 1.0)
}

// ──────────────────────── TPDF Dither ────────────────────────
/// Triangular Probability Density Function dither for 24-bit output.
/// Eliminates quantization distortion at low signal levels.
/// Uses simple deterministic LCG to avoid heap allocation.
#[inline]
pub fn tpdf_dither(rng_state: &mut u32) -> f32 {
    // Two uniform random values → triangular distribution
    let r1 = lcg_next(rng_state);
    let r2 = lcg_next(rng_state);
    (r1 - r2) * DITHER_SCALE_24
}

/// Linear Congruential Generator — RT-safe, no heap, deterministic.
#[inline]
fn lcg_next(state: &mut u32) -> f32 {
    *state = state.wrapping_mul(1103515245).wrapping_add(12345);
    // Convert to [-1.0, 1.0]
    (*state as f32) / (u32::MAX as f32) * 2.0 - 1.0
}

pub struct AudioBuffers {
    pub track_bufs: Vec<Vec<f32>>,
    pub track_vols: Vec<f32>,
    pub track_pans: Vec<f32>,
    pub track_trims: Vec<f32>,
    pub track_events: Vec<Vec<MidiNoteEvent>>,
    pub track_expression_events: Vec<Vec<ExpressionEvent>>,
    pub track_param_events: Vec<Vec<ParameterEvent>>,
    pub master_mix: Vec<f32>,
    // Real-Time Safety: Use RingBuffer Producers instead of Vec
    pub recording_producers: Vec<Option<HeapProd<f32>>>,
    // Pre-allocated PDC latencies buffer (avoids heap alloc in audio callback)
    pub latencies: Vec<u32>,
    // Pre-allocated pitch buffer for sequencer (avoids heap alloc per step)
    pub pitch_buf: Vec<u8>,
    // Dither RNG state (per-channel to avoid correlation)
    pub dither_state_l: u32,
    pub dither_state_r: u32,
}

impl AudioBuffers {
    pub fn new(max_tracks: usize, buffer_size: usize) -> Self {
        Self {
            track_bufs: vec![vec![0.0; buffer_size]; max_tracks],
            track_vols: vec![1.0; max_tracks],
            track_pans: vec![0.0; max_tracks],
            track_trims: vec![1.0; max_tracks],
            track_events: vec![Vec::with_capacity(128); max_tracks],
            track_expression_events: vec![Vec::with_capacity(MAX_EXPRESSION_EVENTS); max_tracks],
            track_param_events: vec![Vec::with_capacity(MAX_PARAM_EVENTS); max_tracks],
            master_mix: vec![0.0; buffer_size],
            recording_producers: (0..max_tracks).map(|_| None).collect(),
            latencies: vec![0; max_tracks],
            pitch_buf: vec![0; 16],
            dither_state_l: 0x12345678,
            dither_state_r: 0x87654321,
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
        self.track_trims.resize(track_count, 1.0);
        if self.latencies.len() < track_count {
            self.latencies.resize(track_count, 0);
        }
        

        
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

    /// Professional mixing pipeline:
    /// 1. Equal-power pan law (constant loudness across pan positions)
    /// 2. Track trim (pre-fader gain staging)
    /// 3. Peak metering per track (atomic, RT→UI)
    /// 4. Summation to master bus
    pub fn mix_to_master(
        track_bufs: &[Vec<f32>], 
        master_mix: &mut [f32], 
        track_vols: &[f32], 
        track_pans: &[f32], 
        track_trims: &[f32],
        frames: usize, 
        track_count: usize,
        meters: Option<&PeakMeters>,
    ) {
        for (t_idx, track_buf) in track_bufs.iter().take(track_count).enumerate() {
             let vol = track_vols[t_idx];
             let trim = track_trims[t_idx];
             let pan = track_pans[t_idx];
             
             // Equal-power pan law (professional DAW standard)
             let (l_pan, r_pan) = equal_power_pan(pan);
             let l_gain = vol * trim * l_pan;
             let r_gain = vol * trim * r_pan;
             
             let mut peak_l: f32 = 0.0;
             let mut peak_r: f32 = 0.0;
             
             for i in 0..frames {
                 let left = track_buf[i * 2] * l_gain;
                 let right = track_buf[i * 2 + 1] * r_gain;
                 
                 master_mix[i*2] += left;
                 master_mix[i*2+1] += right;
                 
                 // Track peak metering
                 peak_l = peak_l.max(left.abs());
                 peak_r = peak_r.max(right.abs());
             }
             
             // Store peak atomically for UI
             if let Some(m) = meters {
                 m.store_track_peak(t_idx, peak_l, peak_r);
             }
         }
    }

    /// Apply master bus processing: soft-clip + dither + metering.
    /// Called after mix_to_master, before writing to output buffer.
    pub fn master_finalize(
        master_mix: &mut [f32],
        frames: usize,
        gain: f32,
        dither_state_l: &mut u32,
        dither_state_r: &mut u32,
        meters: Option<&PeakMeters>,
    ) {
        let mut peak_l: f32 = 0.0;
        let mut peak_r: f32 = 0.0;

        for i in 0..frames {
            let mut left = master_mix[i * 2] * gain;
            let mut right = master_mix[i * 2 + 1] * gain;

            // Soft-clip to prevent harsh digital distortion
            left = soft_clip(left);
            right = soft_clip(right);

            // TPDF dither for 24-bit output quality
            left += tpdf_dither(dither_state_l);
            right += tpdf_dither(dither_state_r);

            // Final hard clip to prevent DAC overflow
            left = hard_clip(left);
            right = hard_clip(right);

            master_mix[i * 2] = left;
            master_mix[i * 2 + 1] = right;

            peak_l = peak_l.max(left.abs());
            peak_r = peak_r.max(right.abs());
        }

        if let Some(m) = meters {
            m.store_master_peak(peak_l, peak_r);
        }
    }
}
