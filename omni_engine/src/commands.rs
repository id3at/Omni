use crossbeam_channel::Sender;
use omni_shared::project::Project;

pub enum EngineCommand {
    Play,
    Pause,
    Stop,
    SetVolume(f32),
    ToggleNote { 
        track_index: usize, 
        clip_index: usize, 
        start: f64, 
        duration: f64, 
        note: u8,
        velocity: u8,
        probability: f64,
        velocity_deviation: i8,
        condition: omni_shared::project::NoteCondition,
    },
    RemoveNote {
        track_index: usize,
        clip_index: usize,
        start: f64,
        note: u8,
    },
    UpdateNote {
        track_index: usize,
        clip_index: usize,
        old_start: f64,
        old_note: u8,
        new_start: f64,
        new_duration: f64,
        new_note: u8,
        new_velocity: u8,
        new_probability: f64,
        new_velocity_deviation: i8,
        new_condition: omni_shared::project::NoteCondition,
    },
    SetMute { track_index: usize, muted: bool },
    SetBpm(f32),
    SetPluginParam { track_index: usize, id: u32, value: f32 },
    GetPluginParams { track_index: usize, response_tx: Sender<Vec<omni_shared::ParamInfo>> },
    SimulateCrash { track_index: usize },
    TriggerClip { track_index: usize, clip_index: usize },
    SetTrackVolume { track_index: usize, volume: f32 },
    SetTrackPan { track_index: usize, pan: f32 },
    // State Management (No I/O)
    GetProjectState(Sender<Project>),
    LoadProjectState(Project, Vec<Box<dyn crate::nodes::AudioNode>>),
    ResetGraph,
    StopTrack { track_index: usize },
    RemoveTrack { track_index: usize }, 
    NewProject, 
    OpenPluginEditor { track_index: usize },
    SetClipLength { track_index: usize, clip_index: usize, length: f64 },
    AddTrackNode { node: Box<dyn crate::nodes::AudioNode>, name: String, plugin_path: Option<String> }, 
    ReplaceTrackNode { track_index: usize, node: Box<dyn crate::nodes::AudioNode>, name: String, plugin_path: String }, 
    UpdateClipSequencer {
        track_index: usize,
        clip_index: usize,
        use_sequencer: bool,
        data: omni_shared::project::StepSequencerData,
    },
    // Returns (clap_id, note_names)
    GetNoteNames { track_index: usize, response_tx: Sender<(String, Vec<omni_shared::NoteNameInfo>)> },
    // Returns (param_id, value, generation)
    GetLastTouchedParam { track_index: usize, response_tx: Sender<Option<(u32, f32, u32)>> },
    
    // Plugin State Management
    GetPluginState { track_index: usize, response_tx: Sender<Option<Vec<u8>>> },
    SetPluginState { track_index: usize, data: Vec<u8> },
    
    // Asset Management
    // UI Loads file, sends raw data. Engine adds to pool.
    AddAsset { name: String, data: Vec<f32>, source_sample_rate: f32, response_tx: Sender<Result<u32, String>> }, 
    
    // View/Mode
    SetArrangementMode(bool),
    
    // Arrangement Editing
    MoveClip { track_index: usize, clip_index: usize, new_start: u64 },
    StretchClip { track_index: usize, clip_index: usize, original_bpm: f32 },
    
    // Recording Session to Arrangement
    StartRecording,
    StopRecording { response_tx: Sender<Vec<(usize, omni_shared::project::ArrangementClip)>> }, // Returns (track_idx, clip) pairs
    
    // Sync recorded clips to engine project
    AddArrangementClips { clips: Vec<(usize, omni_shared::project::ArrangementClip)> },
}
