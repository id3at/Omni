use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Commands sent from Host to Plugin Process via IPC (e.g., Stdin/Pipe)
#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum HostCommand {
    /// Initialize the plugin with a unique ID and shared memory identifier
    Initialize {
        plugin_id: Uuid,
        shmem_config: ShmemConfig,
    },
    /// Request the plugin to process audio (trigger for test)
    ProcessFrame,
    /// Graceful shutdown
    Shutdown,
    /// Set a parameter value
    SetParameter {
        param_id: u32,
        value: f32,
    },
}

/// Events sent from Plugin Process to Host via IPC (e.g., Stdout/Pipe)
#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum PluginEvent {
    /// Initialization successful
    Initialized,
    /// Heartbeat signal
    Heartbeat,
    /// Error occurred
    Error(String),
    /// Processed frame completed
    FrameProcessed,
}

/// Configuration for Shared Memory Region
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ShmemConfig {
    /// OS-specific name/identifier for the shared memory region
    pub os_id: String,
    /// Size of the region in bytes
    pub size: usize,
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
    /// Offset to the Parameter Bank
    pub param_offset: u32,
}

pub const OMNI_MAGIC: u32 = 0x01131109;

// Helper to calculate buffer sizes for fixed latency
pub const BUFFER_SIZE: usize = 512;
pub const CHANNEL_COUNT: usize = 2;
