pub mod graph;
pub mod nodes;
pub mod plugin_node;
pub mod sequencer;
pub mod transport;

use crate::graph::AudioGraph;
use crate::nodes::{GainNode}; // Added GainNode
use crate::plugin_node::PluginNode;
use crate::sequencer::{Sequencer, StepGenerator};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{Receiver, Sender};
use omni_shared::project::{Project, Track};
use omni_shared::MidiNoteEvent;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU32, Ordering};
// use petgraph::graph::NodeIndex;
use std::sync::{Arc};
use crate::nodes::AudioNode;
use std::fs::File;
use std::io::Write;

pub struct AudioEngine {
    _stream: cpal::Stream,
    is_playing: Arc<AtomicBool>,
    sample_position: Arc<AtomicU64>,
    _sequencer: Sequencer,
    current_step: Arc<AtomicU32>,
    sample_rate: u32,
}




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
        probability: f64,
        velocity_deviation: i8,
        condition: omni_shared::project::NoteCondition,
    },
    SetMute { track_index: usize, muted: bool },
    SetBpm(f32),
    SetPluginParam { track_index: usize, id: u32, value: f32 },
    GetPluginParams { track_index: usize, response_tx: Sender<Vec<omni_shared::ParamInfo>> },
    SimulateCrash { track_index: usize },
    TriggerClip { track_index: usize, clip_index: usize },
    SetTrackVolume { track_index: usize, volume: f32 },
    SetTrackPan { track_index: usize, pan: f32 },
    SaveProject(String),
    LoadProject(String),
    ResetGraph,
    StopTrack { track_index: usize },
    OpenPluginEditor { track_index: usize },
    SetClipLength { track_index: usize, clip_index: usize, length: f64 },
    AddTrack { plugin_path: Option<String> }, // Added AddTrack command
    LoadPluginToTrack { track_index: usize, plugin_path: String }, // Added LoadPlugin command
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
}

impl AudioEngine {
    pub fn new(command_rx: Receiver<EngineCommand>) -> Result<Self, anyhow::Error> {
        let host = cpal::default_host();
        let device = host.default_output_device().ok_or(anyhow::anyhow!("No output device available"))?;
        let config = device.default_output_config()?;
        
        let sample_rate = config.sample_rate();
        let channels = config.channels() as usize;
        let sample_format = config.sample_format();

        // Check buffer size capabilities (Informational)
        if let cpal::SupportedBufferSize::Range { min, max } = config.buffer_size() {
             eprintln!("[AudioEngine] Device Buffer Range: {}-{}", min, max);
        }
        
        // Create StreamConfig and override buffer size
        let mut stream_config: cpal::StreamConfig = config.into();
        stream_config.buffer_size = cpal::BufferSize::Fixed(2048);
        eprintln!("[AudioEngine] Using Config: {:?}", stream_config);

        // Shared Atomic State
        let play_flag = Arc::new(AtomicBool::new(false));
        let pos_counter = Arc::new(AtomicU64::new(0));
        let master_gain = Arc::new(AtomicU32::new(1.0f32.to_bits()));
        let current_step = Arc::new(AtomicU32::new(0));

        let is_playing = play_flag.clone();
        let sample_position = pos_counter.clone();
        let master_gain_callback = master_gain.clone();
        let current_step_callback = current_step.clone();
        
        // Owned State for Audio Thread
        let mut graph = AudioGraph::new();
        let mut project = Project::default();
        let mut sequencer = Sequencer::new(120.0);
        let mut track_node_indices = Vec::new();

        let err_fn = |err: cpal::StreamError| {
            let s = err.to_string();
            // Suppress common buffer under/overrun messages to avoid console spam
            if !s.contains("underrun") && !s.contains("overrun") {
                eprintln!("an error occurred on stream: {}", s);
            }
        };
        
        // Track active notes for Note Offs: TrackIndex -> Vec<(Note, RemainingSamples)>
        let mut active_notes: Vec<Vec<(u8, u64)>> = vec![vec![]; 32]; // Increase capacity

        // ZERO-ALLOCATION BUFFERS
        struct AudioBuffers {
            track_bufs: Vec<Vec<f32>>,
            track_vols: Vec<f32>,
            track_pans: Vec<f32>,
            track_events: Vec<Vec<MidiNoteEvent>>,
            track_expression_events: Vec<Vec<omni_shared::ExpressionEvent>>,
            track_param_events: Vec<Vec<omni_shared::ParameterEvent>>,
            master_mix: Vec<f32>,
        }
        
        let max_buffer_size = 2048 * 2;
        let max_tracks = 32;
        
        let mut audio_buffers = AudioBuffers {
            track_bufs: vec![vec![0.0; max_buffer_size]; max_tracks],
            track_vols: vec![1.0; max_tracks],
            track_pans: vec![0.0; max_tracks],
            track_events: vec![Vec::with_capacity(128); max_tracks],
            track_expression_events: vec![Vec::with_capacity(omni_shared::MAX_EXPRESSION_EVENTS); max_tracks],
            track_param_events: vec![Vec::with_capacity(omni_shared::MAX_PARAM_EVENTS); max_tracks],
            master_mix: vec![0.0; max_buffer_size],
        };

        let stream = match sample_format {
            cpal::SampleFormat::F32 => device.build_output_stream(
                &stream_config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    // Check for commands
                    while let Ok(cmd) = command_rx.try_recv() {
                        match cmd {
                            EngineCommand::Play => play_flag.store(true, Ordering::Relaxed),
                            EngineCommand::Pause => play_flag.store(false, Ordering::Relaxed),
                            EngineCommand::Stop => {
                                play_flag.store(false, Ordering::Relaxed);
                                pos_counter.store(0, Ordering::Relaxed);
                                sequencer.reset();
                            }
                            EngineCommand::SetVolume(v) => {
                                master_gain_callback.store(v.to_bits(), Ordering::Relaxed);
                            }
                            EngineCommand::SetBpm(bpm) => {
                                sequencer.bpm = bpm;
                            }
                            EngineCommand::SetClipLength { track_index, clip_index, length } => {
                                if let Some(track) = project.tracks.get_mut(track_index) {
                                    if let Some(clip) = track.clips.get_mut(clip_index) {
                                        clip.length = length;
                                        eprintln!("[Engine] Updated Clip {}:{} Length to {} beats. Seq pattern_length remains {}", track_index, clip_index, length, sequencer.pattern_length);
                                    }
                                }
                            }
                            EngineCommand::SetPluginParam { track_index, id, value } => {
                                // 1. Update Engine Node
                                if let Some(&node_idx) = track_node_indices.get(track_index) {
                                    if let Some(node) = graph.node_mut(node_idx) {
                                        node.set_param(id, value);
                                    }
                                }
                                // 2. Update Project State
                                if let Some(track) = project.tracks.get_mut(track_index) {
                                    track.parameters.insert(id, value);
                                }
                            }
                            EngineCommand::GetPluginParams { track_index, response_tx } => {
                                if let Some(&node_idx) = track_node_indices.get(track_index) {
                                    if let Some(node) = graph.node_mut(node_idx) {
                                        let params = node.get_plugin_params();
                                        let _ = response_tx.send(params);
                                    }
                                }
                            }
                            EngineCommand::ToggleNote { track_index, clip_index, start, duration, note, probability, velocity_deviation, condition } => {
                                if track_index < project.tracks.len() {
                                    if clip_index < project.tracks[track_index].clips.len() {
                                        let notes = &mut project.tracks[track_index].clips[clip_index].notes;
                                        // Simple exact match removal or add
                                        if let Some(idx) = notes.iter().position(|n| n.key == note && (n.start - start).abs() < 0.001) {
                                            notes.remove(idx);
                                        } else {
                                            notes.push(omni_shared::project::Note {
                                                start,
                                                duration,
                                                key: note,
                                                velocity: 100,
                                                probability,
                                                velocity_deviation,
                                                condition,
                                                selected: false,
                                            });
                                        }
                                    }
                                }
                            }
                            EngineCommand::SetMute { track_index, muted } => {
                                if track_index < project.tracks.len() {
                                    project.tracks[track_index].mute = muted;
                                }
                            }
                            EngineCommand::SimulateCrash { track_index } => {
                                if let Some(&node_idx) = track_node_indices.get(track_index) {
                                    if let Some(node) = graph.node_mut(node_idx) {
                                        node.simulate_crash();
                                    }
                                }
                            }
                            EngineCommand::TriggerClip { track_index, clip_index } => {
                                if track_index < project.tracks.len() {
                                    project.tracks[track_index].active_clip_index = Some(clip_index);
                                }
                            }
                            EngineCommand::StopTrack { track_index } => {
                                if track_index < project.tracks.len() {
                                    project.tracks[track_index].active_clip_index = None;
                                }
                            }
                            EngineCommand::OpenPluginEditor { track_index } => {
                                if let Some(&node_idx) = track_node_indices.get(track_index) {
                                    if let Some(node) = graph.node_mut(node_idx) {
                                        // Explicitly cast or handle return to satisfy type inference if needed, but here it returns ()
                                        node.open_editor(); 
                                    }
                                }
                            }
                            EngineCommand::SetTrackVolume { track_index, volume } => {
                                if track_index < project.tracks.len() {
                                    project.tracks[track_index].volume = volume;
                                }
                            }
                            EngineCommand::SetTrackPan { track_index, pan } => {
                                if track_index < project.tracks.len() {
                                    project.tracks[track_index].pan = pan;
                                }
                            }
                            EngineCommand::SaveProject(path) => {
                                if let Ok(json) = serde_json::to_string_pretty(&project) {
                                    if let Ok(mut file) = File::create(&path) {
                                        let _ = file.write_all(json.as_bytes());
                                        eprintln!("[Engine] Saved project to: {}", path);
                                    }
                                }
                            }
                            EngineCommand::LoadProject(path) => {
                                if let Ok(content) = std::fs::read_to_string(&path) {
                                    if let Ok(new_proj) = serde_json::from_str::<Project>(&content) {
                                        // 1 Reset Graph
                                        graph = AudioGraph::new();
                                        track_node_indices.clear();
                                        
                                        // 2 Load Project
                                        project = new_proj;
                                        
                                        // 3 Rebuild Graph from Project
                                        for track in &project.tracks {
                                             // Re-instantiate plugins or gain nodes
                                             let plugin_path = if track.plugin_path.is_empty() { None } else { Some(track.plugin_path.as_str()) };
                                             let node: Box<dyn AudioNode> = if let Some(path) = plugin_path {
                                                 match PluginNode::new(path) {
                                                     Ok(n) => Box::new(n),
                                                     Err(e) => {
                                                         eprintln!("Error loading plugin: {}", e);
                                                         Box::new(GainNode::new(1.0))
                                                     }
                                                 }
                                             } else {
                                                 Box::new(GainNode::new(1.0))
                                             };
                                             let node_idx = graph.add_node(node);
                                             track_node_indices.push(node_idx);
                                        }
                                        eprintln!("[Engine] Loaded project state: {}", path);
                                    }
                                }
                            }
                            EngineCommand::ResetGraph => {
                                graph = AudioGraph::new();
                                track_node_indices.clear();
                                eprintln!("[Engine] Audio Graph Reset");
                            }
                            EngineCommand::AddTrack { plugin_path } => {
                                let node: Box<dyn AudioNode> = if let Some(ref path) = plugin_path {
                                     match PluginNode::new(path) {
                                         Ok(n) => Box::new(n),
                                         Err(e) => {
                                             eprintln!("Error loading plugin: {}", e);
                                             Box::new(GainNode::new(1.0))
                                         }
                                     }
                                } else {
                                     Box::new(GainNode::new(1.0))
                                };
                                
                                let node_idx = graph.add_node(node);
                                let mut t = Track::default();
                                t.name = if plugin_path.is_some() { "Plugin" } else { "Sine" }.into();
                                if let Some(p) = plugin_path { t.plugin_path = p; }
                                
                                project.tracks.push(t);
                                track_node_indices.push(node_idx);
                            }
                            EngineCommand::LoadPluginToTrack { track_index, plugin_path } => {
                                 if let Some(&node_idx) = track_node_indices.get(track_index) {
                                     match PluginNode::new(&plugin_path) {
                                         Ok(node) => {
                                              if let Some(existing_node_ref) = graph.node_mut(node_idx) {
                                                  *existing_node_ref = Box::new(node);
                                                  
                                                  // Update Project
                                                  if track_index < project.tracks.len() {
                                                      let name = std::path::Path::new(&plugin_path)
                                                          .file_stem()
                                                          .and_then(|s| s.to_str())
                                                          .unwrap_or("Plugin")
                                                          .to_string();
                                                      project.tracks[track_index].name = name;
                                                      project.tracks[track_index].plugin_path = plugin_path;
                                                  }
                                              }
                                         },
                                         Err(e) => eprintln!("Failed to load plugin: {}", e),
                                     }
                                 }
                            }
                            EngineCommand::UpdateClipSequencer { track_index, clip_index, use_sequencer, data } => {
                                if track_index < project.tracks.len() {
                                    if clip_index < project.tracks[track_index].clips.len() {
                                        project.tracks[track_index].clips[clip_index].use_sequencer = use_sequencer;
                                        project.tracks[track_index].clips[clip_index].step_sequencer = data;
                                    }
                                }
                            }
                            EngineCommand::GetNoteNames { track_index, response_tx } => {
                                if let Some(&node_idx) = track_node_indices.get(track_index) {
                                    if let Some(node) = graph.node_mut(node_idx) {
                                        let names = node.get_note_names();
                                        let _ = response_tx.send(names);
                                    }
                                }
                            }
                            EngineCommand::GetLastTouchedParam { track_index, response_tx } => {
                                if let Some(&node_idx) = track_node_indices.get(track_index) {
                                    if let Some(node) = graph.node_mut(node_idx) {
                                        let touched = node.get_last_touched();
                                        // Filter empty (0,0,0) - assuming 0 param ID might be valid but generation 0 means never touched? 
                                        // Generation starts at 0. But incremented on touch.
                                        // Let's pass it raw.
                                        let result = if touched.2 > 0 { Some(touched) } else { None };
                                        let _ = response_tx.send(result);
                                    }
                                }
                            }
                        }
                    }

                    let playing = play_flag.load(Ordering::Relaxed);
                    let frames = data.len() / channels;
                    let sample_rate_val = sample_rate as f32;
                    let track_count = track_node_indices.len();

                    // Resize Buffers (Keep Capacity)
                    // Master Mix
                    if audio_buffers.master_mix.len() != frames * 2 {
                        audio_buffers.master_mix.resize(frames * 2, 0.0);
                    }
                    audio_buffers.master_mix.fill(0.0);

                    // Track Data
                        audio_buffers.track_vols.resize(track_count, 1.0);
                        audio_buffers.track_pans.resize(track_count, 0.0);
                        audio_buffers.track_events.resize(track_count, Vec::with_capacity(128));
                        audio_buffers.track_expression_events.resize(track_count, Vec::with_capacity(omni_shared::MAX_EXPRESSION_EVENTS));
                        audio_buffers.track_param_events.resize(track_count, Vec::with_capacity(omni_shared::MAX_PARAM_EVENTS));
                        audio_buffers.track_bufs.resize(track_count, vec![0.0; max_buffer_size]);
                    
                    // Clear Events
                    for i in 0..track_count {
                        audio_buffers.track_events[i].clear();
                        audio_buffers.track_expression_events[i].clear();
                        audio_buffers.track_param_events[i].clear();
                        // Resize buffer for this frame
                        if audio_buffers.track_bufs[i].len() != frames * 2 {
                             // This keeps capacity if it's large enough
                             audio_buffers.track_bufs[i].resize(frames * 2, 0.0);
                        }
                    }

                    // 1. Update Vol/Pan from Project (Always)
                    if active_notes.len() < track_count {
                        active_notes.resize(track_count, Vec::new());
                    }

                    for (t_idx, track) in project.tracks.iter().enumerate() {
                        if t_idx < track_count {
                            audio_buffers.track_vols[t_idx] = track.volume;
                            audio_buffers.track_pans[t_idx] = track.pan;
                        }
                    }

                    // 2a. Process Active Notes (Note Offs)
                    for (t_idx, notes) in active_notes.iter_mut().enumerate() {
                        if t_idx >= track_count { continue; }
                        
                        let mut survived = Vec::new();
                        // Explicit type annotation for drain
                        let drained: std::vec::Drain<(u8, u64)> = notes.drain(..);
                        for (note, remaining) in drained {
                            if remaining > frames as u64 {
                                survived.push((note, remaining - frames as u64));
                            } else {
                                audio_buffers.track_events[t_idx].push(MidiNoteEvent {
                                    note, velocity: 0, channel: 0, sample_offset: 0,
                                    detune: 0.0,
                                });
                            }
                        }
                        *notes = survived;
                    }
                    
                    // 2b. If Playing: Generate Sequence Events (Note Ons)
                    // 2b. If Playing: Generate Sequence Events (Note Ons)
                    if playing {
                         // Calculate time range for this buffer
                         let bpm = sequencer.bpm;
                         let samples_per_beat = (sample_rate_val * 60.0) / bpm;
                         let current_sample = pos_counter.load(Ordering::Relaxed);
                         
                         let start_beat = (current_sample as f64) / samples_per_beat as f64;
                         let end_beat = ((current_sample + frames as u64) as f64) / samples_per_beat as f64;
                         
                         // Update UI step (floored beat * 4 for 16th notes)
                         let current_16th = (start_beat * 4.0) as u32;
                         // Use dynamic pattern length from sequencer
                         let len_steps = sequencer.pattern_length.max(1);
                         current_step_callback.store(current_16th % len_steps, Ordering::Relaxed);

                         for (t_idx, track) in project.tracks.iter().enumerate() {
                             if t_idx < track_count && !track.mute {
                                 if let Some(clip_idx) = track.active_clip_index {
                                     if let Some(clip) = track.clips.get(clip_idx) {
                                         // Check notes in this clip
                                         if clip.use_sequencer {
                                            // --- THESYS STEP SEQUENCER LOGIC ---
                                            let seq = &clip.step_sequencer;
                                            
                                            // Determine which global steps we are covering in this buffer
                                            // 16th notes (0.25 beats)
                                            let step_dur_beats = 0.25;
                                            // Check steps that could have subdivisions in this buffer
                                            // Look back 1 step to catch roll subdivisions from previous step
                                            let start_step_idx = ((start_beat / step_dur_beats).floor() as i64).max(0) as u64;
                                            let end_step_idx = (end_beat / step_dur_beats).ceil() as u64;
                                            
                                            for global_step_counter in start_step_idx..end_step_idx {
                                                // Calculate offset in samples for this step trigger
                                                let step_beat_time = global_step_counter as f64 * step_dur_beats;
                                                let offset_beats = step_beat_time - start_beat;
                                                let offset_samples_raw = (offset_beats * samples_per_beat as f64) as i64;
                                                
                                                // Determine if this step's base trigger is in this buffer
                                                let step_starts_in_buffer = offset_samples_raw >= 0 && offset_samples_raw < frames as i64;
                                                let offset_samples = offset_samples_raw.max(0) as u32;
                                                
                                                // Skip if step start is past this buffer
                                                if offset_samples >= frames as u32 { continue; }


                                                // 1. Get Random/Performance Mask Early if active
                                                let rnd_idx = StepGenerator::get_step_index(
                                                    global_step_counter, 
                                                    seq.performance_random.direction, 
                                                    seq.performance_random.loop_start, 
                                                    seq.performance_random.loop_end
                                                );
                                                let rnd_probability = seq.performance_random.steps.get(rnd_idx).copied().unwrap_or(0);
                                                let rnd_muted = seq.muted.get(rnd_idx).copied().unwrap_or(false);
                                                
                                                let mut do_randomize = false;
                                                let random_mask = seq.random_mask_global;
                                                
                                                if !rnd_muted && rnd_probability > 0 {
                                                    if fastrand::u8(1..=100) <= rnd_probability {
                                                        do_randomize = true;
                                                    }
                                                }

                                                // 2. Get Pitch
                                                let pitch_idx = StepGenerator::get_step_index(
                                                    global_step_counter, 
                                                    seq.pitch.direction, 
                                                    seq.pitch.loop_start, 
                                                    seq.pitch.loop_end
                                                );
                                                let mut raw_pitch = seq.pitch.steps.get(pitch_idx).copied().unwrap_or(60);
                                                let pitch_muted = seq.muted.get(pitch_idx).copied().unwrap_or(false);
                                                
                                                if do_randomize && (random_mask & 1) != 0 {
                                                    raw_pitch = fastrand::u8(0..=127);
                                                }

                                                // 3. Get Velocity
                                                let vel_idx = StepGenerator::get_step_index(
                                                    global_step_counter, 
                                                    seq.velocity.direction, 
                                                    seq.velocity.loop_start, 
                                                    seq.velocity.loop_end
                                                );
                                                let mut velocity = seq.velocity.steps.get(vel_idx).copied().unwrap_or(100);
                                                let vel_muted = seq.muted.get(vel_idx).copied().unwrap_or(false);
                                                
                                                if do_randomize && (random_mask & 2) != 0 {
                                                    velocity = fastrand::u8(0..=127);
                                                }

                                                if velocity == 0 || vel_muted || pitch_muted { continue; } // Muted step
                                                
                                                // 4. Get Gate
                                                let gate_idx = StepGenerator::get_step_index(
                                                    global_step_counter, 
                                                    seq.gate.direction, 
                                                    seq.gate.loop_start, 
                                                    seq.gate.loop_end
                                                );
                                                let mut gate_len = seq.gate.steps.get(gate_idx).copied().unwrap_or(0.5);
                                                let gate_muted = seq.muted.get(gate_idx).copied().unwrap_or(false);
                                                
                                                if do_randomize && (random_mask & 4) != 0 {
                                                    gate_len = fastrand::f32();
                                                }

                                                if gate_muted { continue; }
                                                
                                                // 5. Get Probability
                                                // ... (Keep existing prob check) ...
                                                let prob_idx = StepGenerator::get_step_index(
                                                    global_step_counter, 
                                                    seq.probability.direction, 
                                                    seq.probability.loop_start, 
                                                    seq.probability.loop_end
                                                );
                                                let probability = seq.probability.steps.get(prob_idx).copied().unwrap_or(100);
                                                let prob_muted = seq.muted.get(prob_idx).copied().unwrap_or(false);

                                                if prob_muted { continue; }
                                                if probability < 100 {
                                                    if fastrand::u8(1..=100) > probability {
                                                        continue;
                                                    }
                                                }
                                                
                                                // --- PERFORMANCE: OCTAVE ---
                                                let oct_idx = StepGenerator::get_step_index(
                                                    global_step_counter, 
                                                    seq.performance_octave.direction, 
                                                    seq.performance_octave.loop_start, 
                                                    seq.performance_octave.loop_end
                                                );
                                                let mut octave_shift = seq.performance_octave.steps.get(oct_idx).copied().unwrap_or(0);
                                                let oct_muted = seq.muted.get(oct_idx).copied().unwrap_or(false);
                                                
                                                if !oct_muted {
                                                    if do_randomize && (random_mask & 8) != 0 {
                                                        octave_shift = fastrand::i8(-2..=2);
                                                    }
                                                    
                                                    // Apple offset (saturating)
                                                    let shift_semis = (octave_shift as i32) * 12;
                                                    raw_pitch = (raw_pitch as i32 + shift_semis).clamp(0, 127) as u8;
                                                }
                                                
                                                // Quantize AFTER Octave shift or BEFORE? 
                                                // Usually before chord, but after raw pitch generation.
                                                let quantized_pitch = omni_shared::scale::quantize(
                                                    raw_pitch,
                                                    seq.root_key,
                                                    seq.scale
                                                );

                                                // --- PERFORMANCE: CHORD ---
                                                let chd_idx = StepGenerator::get_step_index(
                                                    global_step_counter, 
                                                    seq.performance_chord.direction, 
                                                    seq.performance_chord.loop_start, 
                                                    seq.performance_chord.loop_end
                                                );
                                                let mut chord_type_id = seq.performance_chord.steps.get(chd_idx).copied().unwrap_or(0);
                                                let chd_muted = seq.muted.get(chd_idx).copied().unwrap_or(false);
                                                
                                                if do_randomize && (random_mask & 32) != 0 {
                                                    chord_type_id = fastrand::u8(0..=11); // Range of chords
                                                }

                                                // Resolve Chord Intervals
                                                let mut pitches = Vec::new();
                                                pitches.push(quantized_pitch);
                                                
                                                if !chd_muted && chord_type_id > 0 {
                                                     let types: Vec<omni_shared::scale::ChordType> = omni_shared::scale::ChordType::iter().collect();
                                                     if let Some(ctype) = types.get(chord_type_id as usize) {
                                                         for &interval in ctype.get_intervals() {
                                                             if interval == 0 { continue; } // Skip root, added already
                                                             let p = (quantized_pitch as i32 + interval as i32).clamp(0, 127) as u8;
                                                             // Re-quantize chord notes? Usually yes to stay in scale.
                                                             // Or strict parallel intervals? 
                                                             // Thesys says "The resulting chord will be C minor" - implies STRICT intervals.
                                                             // Let's stick strictly to intervals defined in ChordType relative to root.
                                                             pitches.push(p);
                                                         }
                                                     }
                                                }

                                                // --- PERFORMANCE: BEND ---
                                                let bend_idx = StepGenerator::get_step_index(
                                                    global_step_counter, 
                                                    seq.performance_bend.direction, 
                                                    seq.performance_bend.loop_start, 
                                                    seq.performance_bend.loop_end
                                                );
                                                let bend_val = seq.performance_bend.steps.get(bend_idx).copied().unwrap_or(0); 
                                                let bend_muted = seq.muted.get(bend_idx).copied().unwrap_or(false);
                                                
                                                // --- PERFORMANCE: ROLL (PATTERNS) ---
                                                let roll_idx = StepGenerator::get_step_index(
                                                    global_step_counter, 
                                                    seq.performance_roll.direction, 
                                                    seq.performance_roll.loop_start, 
                                                    seq.performance_roll.loop_end
                                                );
                                                let roll_type = seq.performance_roll.steps.get(roll_idx).copied().unwrap_or(0);
                                                let roll_muted = seq.muted.get(roll_idx).copied().unwrap_or(false);

                                                // Determine if roll is active
                                                let roll_active = !roll_muted && roll_type > 0;
                                                
                                                // Skip entire step processing if step doesn't start in this buffer
                                                // (except for Roll subdivisions which are handled below)
                                                if !step_starts_in_buffer && !roll_active {
                                                    continue;
                                                }
                                                
                                                if roll_active {
                                                    // ROLL ACTIVE: Apply pattern with subdivisions
                                                    let roll_pattern = omni_shared::performance::RollPattern::get(roll_type);
                                                    let num_subdivisions = 4;
                                                    let mut pitch_accumulator: i32 = 0; // Cumulative pitch offset
                                                
                                                for sub_i in 0..num_subdivisions {
                                                    let sub_step = roll_pattern.steps[sub_i];
                                                    
                                                    // Update pitch accumulator based on sub-step type
                                                    match sub_step {
                                                        omni_shared::performance::RollSubStep::PlayUp => {
                                                            pitch_accumulator += 1;
                                                        },
                                                        omni_shared::performance::RollSubStep::PlayDown => {
                                                            pitch_accumulator -= 1;
                                                        },
                                                        _ => {}
                                                    }
                                                    
                                                    if sub_step == omni_shared::performance::RollSubStep::Rest { continue; }

                                                    // Calculate sub-offsets
                                                    let sub_offset_beats = (step_dur_beats / num_subdivisions as f64) * sub_i as f64;
                                                    let event_offset_beats = offset_beats + sub_offset_beats;
                                                    
                                                    // Calculate sample offset - allow negative/future offsets
                                                    let event_offset_samples_raw = (event_offset_beats * samples_per_beat as f64) as i64;
                                                    
                                                    // If this subdivision is in the future (past buffer end), skip for now
                                                    // It will be triggered when we reach that step in a future buffer
                                                    if event_offset_samples_raw >= frames as i64 { continue; }
                                                    if event_offset_samples_raw < 0 { continue; }
                                                    
                                                    let event_offset_samples = event_offset_samples_raw as u32;
                                                    
                                                    // Effective Gate - use 80% of subdivision duration, clamped
                                                    let sub_dur_beats = step_dur_beats / num_subdivisions as f64;
                                                    let effective_gate = 0.8_f32; // 80% of subdivision
                                                    
                                                    let dur_beats = effective_gate as f64 * sub_dur_beats;
                                                    let dur_samples = (dur_beats * samples_per_beat as f64) as u64;

                                                    for &base_p in &pitches {
                                                        // Apply cumulative pitch offset
                                                        let p = (base_p as i32 + pitch_accumulator).clamp(0, 127) as u8;

                                                        // Note On
                                                        audio_buffers.track_events[t_idx].push(MidiNoteEvent {
                                                            note: p,
                                                            velocity,
                                                            channel: 0,
                                                            sample_offset: event_offset_samples,
                                                            detune: 0.0, 
                                                        });
                                                        
                                                        // Handle Note Duration
                                                        let end_offset_abs = event_offset_samples as u64 + dur_samples;
                                                        
                                                        if end_offset_abs < frames as u64 {
                                                            audio_buffers.track_events[t_idx].push(MidiNoteEvent {
                                                                note: p,
                                                                velocity: 0, 
                                                                channel: 0,
                                                                sample_offset: end_offset_abs as u32,
                                                                detune: 0.0,
                                                            });
                                                        } else {
                                                            if t_idx < active_notes.len() {
                                                                let remaining = end_offset_abs - frames as u64;
                                                                active_notes[t_idx].push((p, remaining));
                                                            }
                                                        }

                                                        // --- BEND GENERATION ---
                                                        // Only generate if bend is enabled (value > 0) AND not muted
                                                        if !bend_muted && bend_val > 0 {
                                                            let start_s = event_offset_samples;
                                                            let end_s = (event_offset_samples as u64 + dur_samples).min(frames as u64) as u32;
                                                            
                                                            let mut s = start_s;
                                                            // Generate expression events - less frequently to reduce overhead
                                                            while s < end_s {
                                                                let time_of_s_beats = start_beat + (s as f64 / samples_per_beat as f64);
                                                                let time_in_step_beats = time_of_s_beats - step_beat_time;
                                                                let phase = (time_in_step_beats / step_dur_beats) as f32;
                                                                
                                                                let detune_val = omni_shared::performance::BendShape::get_value(bend_val, phase);
                                                                
                                                                audio_buffers.track_expression_events[t_idx].push(omni_shared::ExpressionEvent {
                                                                    key: p,
                                                                    channel: 0,
                                                                    expression_id: omni_shared::EXPRESSION_TUNING,
                                                                    value: detune_val as f64,
                                                                    sample_offset: s,
                                                                });
                                                                
                                                                s += 128; // Larger granularity to reduce overhead
                                                            }
                                                        }
                                                    }
                                                }
                                                } else {
                                                    // ROLL INACTIVE: Play single normal note
                                                    let event_offset_samples = offset_samples;
                                                    let dur_beats = gate_len as f64 * step_dur_beats;
                                                    let dur_samples = (dur_beats * samples_per_beat as f64) as u64;

                                                    for &base_p in &pitches {
                                                        let p = base_p;
                                                        
                                                        // Note On
                                                        audio_buffers.track_events[t_idx].push(MidiNoteEvent {
                                                            note: p,
                                                            velocity,
                                                            channel: 0,
                                                            sample_offset: event_offset_samples,
                                                            detune: 0.0,
                                                        });
                                                        
                                                        // Handle Note Duration
                                                        let end_offset_abs = event_offset_samples as u64 + dur_samples;
                                                        
                                                        if end_offset_abs < frames as u64 {
                                                            audio_buffers.track_events[t_idx].push(MidiNoteEvent {
                                                                note: p,
                                                                velocity: 0,
                                                                channel: 0,
                                                                sample_offset: end_offset_abs as u32,
                                                                detune: 0.0,
                                                            });
                                                        } else {
                                                            if t_idx < active_notes.len() {
                                                                let remaining = end_offset_abs - frames as u64;
                                                                active_notes[t_idx].push((p, remaining));
                                                            }
                                                        }

                                                        // --- BEND GENERATION (for non-roll notes) ---
                                                        if !bend_muted && bend_val > 0 {
                                                            let start_s = event_offset_samples;
                                                            let end_s = (event_offset_samples as u64 + dur_samples).min(frames as u64) as u32;
                                                            
                                                            let mut s = start_s;
                                                            while s < end_s {
                                                                let time_of_s_beats = start_beat + (s as f64 / samples_per_beat as f64);
                                                                let time_in_step_beats = time_of_s_beats - step_beat_time;
                                                                let phase = (time_in_step_beats / step_dur_beats) as f32;
                                                                
                                                                let detune_val = omni_shared::performance::BendShape::get_value(bend_val, phase);
                                                                
                                                                audio_buffers.track_expression_events[t_idx].push(omni_shared::ExpressionEvent {
                                                                    key: p,
                                                                    channel: 0,
                                                                    expression_id: omni_shared::EXPRESSION_TUNING,
                                                                    value: detune_val as f64,
                                                                    sample_offset: s,
                                                                });
                                                                
                                                                s += 128;
                                                            }
                                                            
                                                            // Reset pitch bend at note end to avoid stale tuning
                                                            if end_s > start_s {
                                                                audio_buffers.track_expression_events[t_idx].push(omni_shared::ExpressionEvent {
                                                                    key: p,
                                                                    channel: 0,
                                                                    expression_id: omni_shared::EXPRESSION_TUNING,
                                                                    value: 0.0, // Reset to neutral pitch
                                                                    sample_offset: end_s.saturating_sub(1),
                                                                });
                                                            }
                                                        }

                                                    }
                                                }

                                                // --- MODULATION TARGETS ---

                                                // --- MODULATION TARGETS ---
                                                // Process each modulation target, get step value, generate ParameterEvent
                                                for mod_target in &seq.modulation_targets {
                                                    // Get step index for this target's lane
                                                    let mod_idx = StepGenerator::get_step_index(
                                                        global_step_counter,
                                                        mod_target.lane.direction,
                                                        mod_target.lane.loop_start,
                                                        mod_target.lane.loop_end
                                                    );
                                                    
                                                    // Get the modulation value (0-127)
                                                    let mod_value = mod_target.lane.steps.get(mod_idx).copied().unwrap_or(0);
                                                    
                                                    // Convert 0-127 to 0.0-1.0
                                                    let normalized_value = mod_value as f64 / 127.0;
                                                    
                                                    // Push parameter event
                                                    audio_buffers.track_param_events[t_idx].push(omni_shared::ParameterEvent {
                                                        param_id: mod_target.param_id,
                                                        value: normalized_value,
                                                        sample_offset: offset_samples,
                                                    });
                                                }
                                            }
                                         } else {
                                             // --- LEGACY PIANO ROLL LOGIC ---
                                             // Ideally use a spatial map, but for <1000 notes linear scan is fine
                                             let loop_len = clip.length; 
                                             
                                          for note in &clip.notes {
                                             // Handle looping: Map note start to current window
                                             // Simple approach: Check if note falls in [start_beat, end_beat) modulo loop
                                             
                                             // Allow for notes to be triggered in this window
                                             // Note start relative to loop start
                                             let relative_start = note.start % loop_len;
                                             
                                             // We need to check if 'relative_start' happens between 'start_beat' (mod loop) and 'end_beat' (mod loop)
                                             // But wrapping makes this hard.
                                             // Easier: Calculate absolute beats for potential occurrences in this window.
                                             
                                             let loop_start_beat = (start_beat / loop_len).floor() * loop_len;
                                             let mut check_beat = loop_start_beat + relative_start;
                                             
                                             // If we are just before the note in previous loop, check next
                                             if check_beat < start_beat {
                                                 check_beat += loop_len;
                                             }
                                             
                                             if check_beat >= start_beat && check_beat < end_beat {
                                                 // STOCHASTIC CHECKS
                                                 // 1. Probability
                                                 if note.probability < 1.0 && fastrand::f64() > note.probability {
                                                     continue;
                                                 }
                                                 
                                                 // 2. Condition (Loop Iteration)
                                                 if let omni_shared::project::NoteCondition::Iteration { expected, cycle } = note.condition {
                                                      let iteration = (check_beat / loop_len).floor() as u64;
                                                      // 1-based index (1..cycle)
                                                      let current_cycle_idx = (iteration % cycle as u64) as u8 + 1;
                                                      if current_cycle_idx != expected {
                                                          continue;
                                                      }
                                                 }
                                                 
                                                 // 3. Velocity Deviation
                                                 let mut velocity = note.velocity;
                                                 if note.velocity_deviation != 0 {
                                                     let dev = note.velocity_deviation as i32;
                                                     let rnd = fastrand::i32(-dev.abs()..=dev.abs());
                                                     velocity = (velocity as i32 + rnd).clamp(1, 127) as u8;
                                                 }

                                                 // Trigger!
                                                 let offset_beats = check_beat - start_beat;
                                                 let offset_samples = (offset_beats * samples_per_beat as f64) as u32;
                                                 
                                                 if offset_samples < frames as u32 {
                                                     audio_buffers.track_events[t_idx].push(MidiNoteEvent {
                                                         note: note.key,
                                                         velocity,
                                                         channel: 0,
                                                         sample_offset: offset_samples,
                                                         detune: 0.0,
                                                     });
                                                     
                                                     if t_idx < active_notes.len() {
                                                         // Calculate note duration in samples
                                                         let dur_samples = (note.duration * samples_per_beat as f64) as u64;
                                                         active_notes[t_idx].push((note.key, dur_samples));
                                                     }
                                                 }
                                             }
                                         }
                                          } // End Else added by script
                                     }
                                 }
                             }
                         }
                    }

                    // 3. Update Global Transport for PluginNodes
                    {
                        let bpm = sequencer.bpm;
                        let samples_per_beat = (sample_rate_val * 60.0) / bpm;
                        let current_sample = pos_counter.load(Ordering::Relaxed);
                        let song_pos_beats = (current_sample as f64) / samples_per_beat as f64;
                        
                        // Calculate bar position (assuming 4/4 time signature)
                        let beats_per_bar = 4.0;
                        let bar_number = (song_pos_beats / beats_per_bar).floor() as i32;
                        let bar_start_beats = bar_number as f64 * beats_per_bar;
                        
                        crate::transport::update_transport(crate::transport::TransportState {
                            is_playing: playing,
                            tempo: bpm as f64,
                            song_pos_beats,
                            bar_start_beats,
                            bar_number,
                            time_sig_num: 4,
                            time_sig_denom: 4,
                        });
                    }

                    // 4. Parallel Process Graph
                    // PASS SLICES OF PRE_ALLOCATED BUFFERS
                    let buf_slice = &mut audio_buffers.track_bufs[0..track_count];
                    let evt_slice = &audio_buffers.track_events[0..track_count];
                    let param_evt_slice = &audio_buffers.track_param_events[0..track_count];
                    let expr_evt_slice = &audio_buffers.track_expression_events[0..track_count];
                    
                    graph.process_overlay(&track_node_indices, buf_slice, evt_slice, param_evt_slice, expr_evt_slice, sample_rate_val);

                    // 4. Mix to Master
                    for (t_idx, track_buf) in buf_slice.iter().enumerate() {
                         let vol = audio_buffers.track_vols[t_idx];
                         let pan = audio_buffers.track_pans[t_idx];
                         
                         for i in 0..frames {
                             let left = track_buf[i * 2];
                             let right = track_buf[i * 2 + 1];
                             
                             let mut l_gain = vol;
                             let mut r_gain = vol;
                             
                             if pan > 0.0 {
                                 l_gain *= 1.0 - pan;
                             } else if pan < 0.0 {
                                 r_gain *= 1.0 + pan;
                             }
                             
                             audio_buffers.master_mix[i*2] += left * l_gain;
                             audio_buffers.master_mix[i*2+1] += right * r_gain;
                         }
                     }
                     
                     if playing {
                         pos_counter.fetch_add(frames as u64, Ordering::Relaxed);
                     }
                     
                     // Interleave back to data
                     let gain = f32::from_bits(master_gain_callback.load(Ordering::Relaxed));

                     for i in 0..frames {
                         let left = audio_buffers.master_mix[i * 2];
                         let right = audio_buffers.master_mix[i * 2 + 1];

                         if channels == 2 {
                             data[i * 2] = left * gain;
                             data[i * 2 + 1] = right * gain;
                         } else {
                             data[i] = (left + right) * 0.5 * gain;
                         }
                     }
                },
                err_fn,
                None, 
            )?,
            _ => return Err(anyhow::anyhow!("Unsupported sample format")),
        };

        stream.play()?;

        Ok(Self {
            _stream: stream,
            is_playing,
            sample_position,
            _sequencer: Sequencer::new(120.0), // Placeholder
            current_step,
            sample_rate,
        })
    }

// ...

    pub fn is_playing(&self) -> bool {
        self.is_playing.load(Ordering::Relaxed)
    }

    pub fn get_sample_position(&self) -> u64 {
        self.sample_position.load(Ordering::Relaxed)
    }

    pub fn get_current_step(&self) -> u32 {
        self.current_step.load(Ordering::Relaxed)
    }

    pub fn get_sample_rate(&self) -> u32 {
        self.sample_rate
    }
}
