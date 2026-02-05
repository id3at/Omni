pub mod graph;
pub mod nodes;
pub mod plugin_node;
pub mod sequencer;

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
            master_mix: Vec<f32>,
        }
        
        let max_buffer_size = 2048 * 2;
        let max_tracks = 32;
        
        let mut audio_buffers = AudioBuffers {
            track_bufs: vec![vec![0.0; max_buffer_size]; max_tracks],
            track_vols: vec![1.0; max_tracks],
            track_pans: vec![0.0; max_tracks],
            track_events: vec![Vec::with_capacity(128); max_tracks],
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
                    if audio_buffers.track_vols.len() < track_count {
                        audio_buffers.track_vols.resize(track_count, 1.0);
                        audio_buffers.track_pans.resize(track_count, 0.0);
                        audio_buffers.track_events.resize(track_count, Vec::with_capacity(128));
                        audio_buffers.track_bufs.resize(track_count, vec![0.0; max_buffer_size]);
                    }
                    
                    // Clear Events
                    for i in 0..track_count {
                        audio_buffers.track_events[i].clear();
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
                                            // use ceil to ensure we catch the start of step
                                            let start_step_idx = (start_beat / step_dur_beats).ceil() as u64;
                                            let end_step_idx = (end_beat / step_dur_beats).ceil() as u64;
                                            
                                            for global_step_counter in start_step_idx..end_step_idx {
                                                // Calculate offset in samples for this step trigger
                                                let step_beat_time = global_step_counter as f64 * step_dur_beats;
                                                let offset_beats = step_beat_time - start_beat;
                                                // clamp just in case
                                                let offset_samples = ((offset_beats * samples_per_beat as f64) as i64).max(0) as u32;
                                                
                                                if offset_samples >= frames as u32 { continue; }

                                                // 1. Get Pitch
                                                let pitch_idx = StepGenerator::get_step_index(
                                                    global_step_counter, 
                                                    seq.pitch.direction, 
                                                    seq.pitch.loop_start, 
                                                    seq.pitch.loop_end
                                                );
                                                let pitch_step = seq.pitch.steps.get(pitch_idx).copied().unwrap_or(60);
                                                let pitch_muted = seq.pitch.muted.get(pitch_idx).copied().unwrap_or(false);
                                                
                                                // 2. Get Velocity
                                                let vel_idx = StepGenerator::get_step_index(
                                                    global_step_counter, 
                                                    seq.velocity.direction, 
                                                    seq.velocity.loop_start, 
                                                    seq.velocity.loop_end
                                                );
                                                let velocity = seq.velocity.steps.get(vel_idx).copied().unwrap_or(100);
                                                let vel_muted = seq.velocity.muted.get(vel_idx).copied().unwrap_or(false);
                                                
                                                if velocity == 0 || vel_muted || pitch_muted { continue; } // Muted step
                                                
                                                // 3. Get Gate
                                                let gate_idx = StepGenerator::get_step_index(
                                                    global_step_counter, 
                                                    seq.gate.direction, 
                                                    seq.gate.loop_start, 
                                                    seq.gate.loop_end
                                                );
                                                let gate_len = seq.gate.steps.get(gate_idx).copied().unwrap_or(0.5);
                                                let gate_muted = seq.gate.muted.get(gate_idx).copied().unwrap_or(false);

                                                if gate_muted { continue; }
                                                
                                                // Trigger
                                                audio_buffers.track_events[t_idx].push(MidiNoteEvent {
                                                    note: pitch_step,
                                                    velocity,
                                                    channel: 0,
                                                    sample_offset: offset_samples,
                                                });
                                                
                                                if t_idx < active_notes.len() {
                                                     let dur_beats = gate_len as f64 * step_dur_beats;
                                                     let dur_samples = (dur_beats * samples_per_beat as f64) as u64;
                                                     active_notes[t_idx].push((pitch_step, dur_samples));
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

                    // 3. Parallel Process Graph
                    // PASS SLICES OF PRE_ALLOCATED BUFFERS
                    let buf_slice = &mut audio_buffers.track_bufs[0..track_count];
                    let evt_slice = &audio_buffers.track_events[0..track_count];
                    
                    graph.process_overlay(&track_node_indices, buf_slice, evt_slice, sample_rate_val);

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
