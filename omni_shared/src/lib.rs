pub mod project;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// MIDI Note Event for triggering synth voices
#[repr(C)]
#[derive(Debug, Serialize, Deserialize, Clone, Copy)]
pub struct MidiNoteEvent {
    /// MIDI note number (0-127)
    pub note: u8,
    /// Velocity (0 = note off, 1-127 = note on)
    pub velocity: u8,
    /// MIDI channel (0-15)
    pub channel: u8,
    /// Sample offset within the current buffer
    pub sample_offset: u32,
}

/// Commands sent from Host to Plugin Process via IPC (e.g., Stdin/Pipe)
#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum HostCommand {
    /// Initialize the plugin with a unique ID and shared memory identifier
    Initialize {
        plugin_id: Uuid,
        shmem_config: ShmemConfig,
    },
    /// Load a plugin from a given path
    LoadPlugin {
        path: String,
    },
    /// Request the plugin to process audio with optional MIDI events
    ProcessFrame {
        count: u32,
    },
    /// Process audio with MIDI note events
    ProcessWithMidi {
        count: u32,
        events: Vec<MidiNoteEvent>,
    },
    /// Graceful shutdown
    Shutdown,
    /// Set a parameter value
    SetParameter {
        param_id: u32,
        value: f32,
    },
    /// Request parameter info
    GetParamInfo,
    /// Open the plugin's native editor window
    OpenEditor,
    /// Request note names from plugin (CLAP note_name extension)
    GetNoteNames,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ParamInfo {
    pub id: u32,
    pub name: String,
    pub min_value: f64,
    pub max_value: f64,
    pub default_value: f64,
    pub flags: u32,
}

/// Note name information from CLAP plugin's note_name extension
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct NoteNameInfo {
    /// MIDI key (-1 means all keys)
    pub key: i16,
    /// MIDI channel (-1 means all channels)
    pub channel: i16,
    /// Human-readable name for this note
    pub name: String,
}

/// Events sent from Plugin Process to Host via IPC (e.g., Stdout/Pipe)
#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum PluginEvent {
    /// Initialization successful
    Initialized,
    /// Plugin loaded successfully
    PluginLoaded,
    /// Heartbeat signal
    Heartbeat,
    /// Error occurred
    Error(String),
    /// Processed frame completed
    FrameProcessed,
    /// Parameter information list
    ParamInfoList(Vec<ParamInfo>),
    /// Note name information list (from CLAP note_name extension or fallback)
    /// Contains: (clap_id, note_names) - clap_id allows host to apply hardcoded mappings
    NoteNameList { clap_id: String, names: Vec<NoteNameInfo> },
}

/// Configuration for Shared Memory Region
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ShmemConfig {
    /// OS-specific name/identifier for the shared memory region
    pub os_id: String,
    /// Size of the region in bytes
    pub size: usize,
}

/// Parameter Automation Event
#[repr(C)]
#[derive(Debug, Serialize, Deserialize, Clone, Copy)]
pub struct ParameterEvent {
    pub param_id: u32,
    pub value: f64,
    pub sample_offset: u32,
}

/// Layout of the Shared Memory Header
/// This sits at the very beginning of the shared memory region.
#[repr(C)]
pub struct OmniShmemHeader {
    /// Protocol version/Magic
    pub magic: u32,
    /// Plugin status flags (Atomic in practice)
    pub status: u32,
    /// Offset to the Audio Input Buffer
    pub input_offset: u32,
    /// Offset to the Audio Output Buffer
    pub output_offset: u32,
    /// Offset to the Parameter Bank (Static)
    pub param_bank_offset: u32,
    
    // Atomic Command/Response Protocol
    pub command: u32, // Host -> Plugin (0=Idle, 1=Process)
    pub response: u32, // Plugin -> Host (0=Idle, 1=Done)
    pub sample_count: u32,
    
    pub midi_event_count: u32,
    /// Offset to the MIDI Event Buffer
    pub midi_offset: u32,

    pub param_event_count: u32,
    /// Offset to the Parameter Event Buffer
    pub param_event_offset: u32,

    // Parameter Learn / Touch Feedback
    pub last_touched_param: u32,
    pub last_touched_value: f32,
    pub touch_generation: u32, // Increments on touch

    // Transport Information
    pub transport_is_playing: u32,   // 0 = stopped, 1 = playing
    pub transport_tempo: f64,        // BPM
    pub transport_song_pos_beats: f64,  // Current position in beats
    pub transport_bar_start_beats: f64, // Start of current bar in beats
    pub transport_bar_number: i32,   // Current bar number
    pub transport_time_sig_num: u16, // Time signature numerator
    pub transport_time_sig_denom: u16, // Time signature denominator
}

pub const SPIN_TIMEOUT_MS: u64 = 5; // Timeout for spin loop

pub const CMD_IDLE: u32 = 0;
pub const CMD_PROCESS: u32 = 1;

pub const RSP_IDLE: u32 = 0;
pub const RSP_DONE: u32 = 1;

pub const OMNI_MAGIC: u32 = 0x01131109;

// Helper to calculate buffer sizes for fixed latency
pub const BUFFER_SIZE: usize = 512;
pub const CHANNEL_COUNT: usize = 2;
pub const MAX_MIDI_EVENTS: usize = 128;
pub const MAX_PARAM_EVENTS: usize = 256;
