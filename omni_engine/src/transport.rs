/// Global transport state shared between audio engine and plugin nodes
/// Lock-free implementation using atomic operations — safe for real-time audio threads.

use std::sync::atomic::{AtomicU64, AtomicI32, AtomicU32, Ordering};

/// Packed atomic transport state — zero locks, zero allocations, RT-safe.
static TRANSPORT_IS_PLAYING: AtomicU32 = AtomicU32::new(0);
static TRANSPORT_TEMPO_BITS: AtomicU64 = AtomicU64::new(0);
static TRANSPORT_SONG_POS_BITS: AtomicU64 = AtomicU64::new(0);
static TRANSPORT_BAR_START_BITS: AtomicU64 = AtomicU64::new(0);
static TRANSPORT_BAR_NUMBER: AtomicI32 = AtomicI32::new(0);
static TRANSPORT_TIME_SIG: AtomicU32 = AtomicU32::new(0x0004_0004); // packed num|denom

/// Transport state passed to plugins (value type, cheap to copy)
#[derive(Clone, Copy, Debug)]
pub struct TransportState {
    pub is_playing: bool,
    pub tempo: f64,
    pub song_pos_beats: f64,
    pub bar_start_beats: f64,
    pub bar_number: i32,
    pub time_sig_num: u16,
    pub time_sig_denom: u16,
}

impl Default for TransportState {
    fn default() -> Self {
        Self {
            is_playing: false,
            tempo: 120.0,
            song_pos_beats: 0.0,
            bar_start_beats: 0.0,
            bar_number: 0,
            time_sig_num: 4,
            time_sig_denom: 4,
        }
    }
}

/// Update the global transport state (called by AudioEngine before processing)
/// Lock-free: uses atomic stores only. Safe to call from real-time audio thread.
#[inline]
pub fn update_transport(state: TransportState) {
    TRANSPORT_IS_PLAYING.store(state.is_playing as u32, Ordering::Relaxed);
    TRANSPORT_TEMPO_BITS.store(state.tempo.to_bits(), Ordering::Relaxed);
    TRANSPORT_SONG_POS_BITS.store(state.song_pos_beats.to_bits(), Ordering::Relaxed);
    TRANSPORT_BAR_START_BITS.store(state.bar_start_beats.to_bits(), Ordering::Relaxed);
    TRANSPORT_BAR_NUMBER.store(state.bar_number, Ordering::Relaxed);
    let packed_sig = ((state.time_sig_num as u32) << 16) | (state.time_sig_denom as u32);
    TRANSPORT_TIME_SIG.store(packed_sig, Ordering::Relaxed);
}

/// Get the current transport state (called by PluginNode during process)
/// Lock-free: uses atomic loads only. Safe to call from any thread.
#[inline]
pub fn get_transport() -> TransportState {
    let packed_sig = TRANSPORT_TIME_SIG.load(Ordering::Relaxed);
    TransportState {
        is_playing: TRANSPORT_IS_PLAYING.load(Ordering::Relaxed) != 0,
        tempo: f64::from_bits(TRANSPORT_TEMPO_BITS.load(Ordering::Relaxed)),
        song_pos_beats: f64::from_bits(TRANSPORT_SONG_POS_BITS.load(Ordering::Relaxed)),
        bar_start_beats: f64::from_bits(TRANSPORT_BAR_START_BITS.load(Ordering::Relaxed)),
        bar_number: TRANSPORT_BAR_NUMBER.load(Ordering::Relaxed),
        time_sig_num: (packed_sig >> 16) as u16,
        time_sig_denom: (packed_sig & 0xFFFF) as u16,
    }
}
