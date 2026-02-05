/// Global transport state shared between audio engine and plugin nodes
use std::sync::RwLock;

lazy_static::lazy_static! {
    /// Global transport state accessible by PluginNode during process
    pub static ref GLOBAL_TRANSPORT: RwLock<TransportState> = RwLock::new(TransportState::default());
}

/// Transport state passed to plugins
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
pub fn update_transport(state: TransportState) {
    if let Ok(mut transport) = GLOBAL_TRANSPORT.write() {
        *transport = state;
    }
}

/// Get the current transport state (called by PluginNode during process)
pub fn get_transport() -> TransportState {
    GLOBAL_TRANSPORT.read().map(|t| *t).unwrap_or_default()
}
