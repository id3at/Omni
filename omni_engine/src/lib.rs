pub mod graph;
pub mod nodes;
pub mod plugin_node;
pub mod sequencer;

use crate::graph::AudioGraph;
use crate::nodes::{SineNode, GainNode}; // Added GainNode
use crate::plugin_node::PluginNode;
use crate::sequencer::Sequencer;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{Receiver, Sender};
use omni_shared::project::{Project, Track};
use omni_shared::MidiNoteEvent;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU32, Ordering};
use petgraph::graph::NodeIndex;
use std::sync::{Arc, Mutex};
use crate::nodes::AudioNode;
use std::fs::File;
use std::io::Write;

pub struct AudioEngine {
    _stream: cpal::Stream,
    is_playing: Arc<AtomicBool>,
    sample_position: Arc<AtomicU64>,
    graph: Arc<Mutex<AudioGraph>>, 
    _sequencer: Arc<Mutex<Sequencer>>,
    track_node_indices: Arc<Mutex<Vec<NodeIndex>>>,
    project: Arc<Mutex<Project>>,
    master_gain: Arc<AtomicU32>,
}

pub enum EngineCommand {
    Play,
    Pause,
    Stop,
    SetVolume(f32),
    ToggleNote { track_index: usize, clip_index: usize, step: usize, note: u8 },
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
}

impl AudioEngine {
    pub fn new(command_rx: Receiver<EngineCommand>) -> Result<Self, anyhow::Error> {
        let host = cpal::default_host();
        let device = host.default_output_device().ok_or(anyhow::anyhow!("No output device available"))?;
        let config = device.default_output_config()?;
        let sample_rate = config.sample_rate();
        let channels = config.channels() as usize;

        let is_playing = Arc::new(AtomicBool::new(false));
        let sample_position = Arc::new(AtomicU64::new(0));
        let master_gain = Arc::new(AtomicU32::new(f32::to_bits(1.0)));

        let play_flag = is_playing.clone();
        let pos_counter = sample_position.clone();
        let master_gain_callback = master_gain.clone();
        
        let mut graph = AudioGraph::new();
        let graph_ref = Arc::new(Mutex::new(graph));
        let graph_in_callback = graph_ref.clone();

        // Initialize Project (Empty)
        let project = Project::default();
        let project_ref = Arc::new(Mutex::new(project));
        let project_in_callback = project_ref.clone();

        // Initialize Sequencer
        let sequencer = Arc::new(Mutex::new(Sequencer::new(120.0)));
        let sequencer_in_callback = sequencer.clone();

        // Initialize Track Indices (Empty)
        let track_node_indices = Arc::new(Mutex::new(Vec::new()));
        let track_node_indices_callback = track_node_indices.clone();

        let err_fn = |err| eprintln!("an error occurred on stream: {}", err);
        
        // Track active notes for Note Offs: TrackIndex -> Vec<(Note, RemainingSamples)>
        let mut active_notes: Vec<Vec<(u8, u64)>> = vec![vec![]; 16];

        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => device.build_output_stream(
                &config.into(),
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    // Check for commands
                    while let Ok(cmd) = command_rx.try_recv() {
                        match cmd {
                            EngineCommand::Play => play_flag.store(true, Ordering::Relaxed),
                            EngineCommand::Pause => play_flag.store(false, Ordering::Relaxed),
                            EngineCommand::Stop => {
                                play_flag.store(false, Ordering::Relaxed);
                                pos_counter.store(0, Ordering::Relaxed);
                                if let Ok(mut seq) = sequencer_in_callback.lock() {
                                    seq.reset();
                                }
                            }
                            EngineCommand::SetVolume(v) => {
                                master_gain_callback.store(v.to_bits(), Ordering::Relaxed);
                            }
                            EngineCommand::SetBpm(bpm) => {
                                if let Ok(mut seq) = sequencer_in_callback.lock() {
                                    seq.bpm = bpm;
                                }
                            }
                            EngineCommand::SetPluginParam { track_index, id, value } => {
                                // 1. Update Engine Node
                                if let Ok(mut graph) = graph_in_callback.lock() {
                                    if let Some(indices) = track_node_indices_callback.lock().ok() {
                                        if let Some(&node_idx) = indices.get(track_index) {
                                            if let Some(node) = graph.node_mut(node_idx) {
                                                node.set_param(id, value);
                                            }
                                        }
                                    }
                                }
                                // 2. Update Project State for persistence
                                if let Ok(mut proj) = project_in_callback.lock() {
                                    if let Some(track) = proj.tracks.get_mut(track_index) {
                                        track.parameters.insert(id, value);
                                    }
                                }
                            }
                            EngineCommand::GetPluginParams { track_index, response_tx } => {
                                if let Ok(mut graph) = graph_in_callback.lock() {
                                    if let Some(indices) = track_node_indices_callback.lock().ok() {
                                        if let Some(&node_idx) = indices.get(track_index) {
                                            if let Some(node) = graph.node_mut(node_idx) {
                                                let params = node.get_plugin_params();
                                                let _ = response_tx.send(params);
                                            }
                                        }
                                    }
                                }
                            }
                            // Hijack SetMute/ToggleNote to update Project
                             EngineCommand::ToggleNote { track_index, clip_index, step, note } => {
                                if let Ok(mut proj) = project_in_callback.lock() {
                                    if track_index < proj.tracks.len() {
                                        if clip_index < proj.tracks[track_index].clips.len() {
                                            let notes = &mut proj.tracks[track_index].clips[clip_index].notes;
                                            if step < notes.len() {
                                                 if notes[step].contains(&note) {
                                                     notes[step].retain(|&n| n != note);
                                                 } else {
                                                     notes[step].push(note);
                                                 }
                                            }
                                        }
                                    }
                                }
                            }
                            EngineCommand::SetMute { track_index, muted } => {
                                if let Ok(mut proj) = project_in_callback.lock() {
                                    if track_index < proj.tracks.len() {
                                        proj.tracks[track_index].mute = muted;
                                    }
                                }
                            }
                            EngineCommand::SimulateCrash { track_index } => {
                                if let Ok(mut graph) = graph_in_callback.lock() {
                                    if let Some(indices) = track_node_indices_callback.lock().ok() {
                                        if let Some(&node_idx) = indices.get(track_index) {
                                            if let Some(node) = graph.node_mut(node_idx) {
                                                node.simulate_crash();
                                            }
                                        }
                                    }
                                }
                            }
                            EngineCommand::TriggerClip { track_index, clip_index } => {
                                if let Ok(mut proj) = project_in_callback.lock() {
                                    if track_index < proj.tracks.len() {
                                        // Simple immediate trigger for now. 
                                        // Future: Quantization logic (wait for next bar)
                                        proj.tracks[track_index].active_clip_index = Some(clip_index);
                                    }
                                }
                            }
                            EngineCommand::StopTrack { track_index } => {
                                if let Ok(mut proj) = project_in_callback.lock() {
                                    if track_index < proj.tracks.len() {
                                        proj.tracks[track_index].active_clip_index = None;
                                    }
                                }
                            }
                            EngineCommand::OpenPluginEditor { track_index } => {
                                // Find the plugin node for this track
                                if let Some(indices) = track_node_indices_callback.lock().ok() {
                                    if let Some(&node_idx) = indices.get(track_index) {
                                        if let Ok(mut graph) = graph_in_callback.lock() {
                                            if let Some(node) = graph.node_mut(node_idx) {
                                                node.open_editor();
                                            }
                                        }
                                    }
                                }
                            }
                            EngineCommand::SetTrackVolume { track_index, volume } => {
                                if let Ok(mut proj) = project_in_callback.lock() {
                                    if track_index < proj.tracks.len() {
                                        proj.tracks[track_index].volume = volume;
                                    }
                                }
                            }
                            EngineCommand::SetTrackPan { track_index, pan } => {
                                if let Ok(mut proj) = project_in_callback.lock() {
                                    if track_index < proj.tracks.len() {
                                        proj.tracks[track_index].pan = pan;
                                    }
                                }
                            }
                            EngineCommand::SaveProject(path) => {
                                if let Ok(proj) = project_in_callback.lock() {
                                    if let Ok(json) = serde_json::to_string_pretty(&*proj) {
                                        if let Ok(mut file) = File::create(&path) {
                                            let _ = file.write_all(json.as_bytes());
                                            eprintln!("[Engine] Saved project to: {}", path);
                                        }
                                    }
                                }
                            }
                            EngineCommand::LoadProject(path) => {
                                if let Ok(content) = std::fs::read_to_string(&path) {
                                    if let Ok(new_proj) = serde_json::from_str::<Project>(&content) {
                                        if let Ok(mut proj) = project_in_callback.lock() {
                                            *proj = new_proj;
                                            eprintln!("[Engine] Loaded project state: {}", path);
                                        }
                                    }
                                }
                            }
                            EngineCommand::ResetGraph => {
                                if let Ok(mut graph) = graph_in_callback.lock() {
                                    *graph = AudioGraph::new();
                                }
                                if let Ok(mut indices) = track_node_indices_callback.lock() {
                                    indices.clear();
                                }
                                eprintln!("[Engine] Audio Graph Reset");
                            }
                        }
                    }

                    let playing = play_flag.load(Ordering::Relaxed);
                    let frames = data.len() / channels;
                    let sample_rate_val = sample_rate as f32;

                    // Master Mix Buffer
                    let mut master_mix = vec![0.0; frames * 2]; 

                    // Acquire track indices
                    let track_indices = track_node_indices_callback.lock().unwrap();
                    let track_count = track_indices.len();

                    // Prepare track data (Vol/Pan/Events)
                    // We must init this ALWAYS
                    let mut track_data: Vec<(f32, f32, Vec<MidiNoteEvent>)> = vec![(1.0, 0.0, Vec::new()); track_count];
                    
                    
                    // 1. Update Vol/Pan from Project (Always)
                    if let Ok(proj) = project_in_callback.lock() {
                        // Resize active_notes if needed
                        if active_notes.len() < proj.tracks.len() {
                            active_notes.resize(proj.tracks.len(), Vec::new());
                        }

                        for (t_idx, track) in proj.tracks.iter().enumerate() {
                            if t_idx < track_data.len() {
                                track_data[t_idx].0 = track.volume;
                                track_data[t_idx].1 = track.pan;
                            }
                        }

                        // 2a. Process Active Notes (Note Offs)
                        for (t_idx, notes) in active_notes.iter_mut().enumerate() {
                            if t_idx >= track_data.len() { continue; }
                            
                            // Decrement and filter
                            let mut survived = Vec::new();
                            for (note, remaining) in notes.drain(..) {
                                if remaining > frames as u64 {
                                    survived.push((note, remaining - frames as u64));
                                } else {
                                    // Send Note Off
                                    // eprintln!("Note Off: Track {} Note {}", t_idx, note);
                                    track_data[t_idx].2.push(MidiNoteEvent {
                                        note,
                                        velocity: 0, // Note Off
                                        channel: 0,
                                        sample_offset: 0,
                                    });
                                }
                            }
                            *notes = survived;
                        }
                        
                        // 2b. If Playing: Generate Sequence Events (Note Ons)
                        if playing {
                             if let Ok(mut seq) = sequencer_in_callback.lock() {
                                 if let Some((step, _offset)) = seq.advance(frames, sample_rate as f32) {
                                     for (t_idx, track) in proj.tracks.iter().enumerate() {
                                         if t_idx < track_data.len() && !track.mute {
                                             if let Some(clip_idx) = track.active_clip_index {
                                                 if let Some(notes) = track.clips.get(clip_idx).and_then(|c| c.notes.get(step as usize)) {
                                                     for &note in notes {
                                                         // eprintln!("Sequence: Track {} Note {}", t_idx, note); // Keep debug for now
                                                         // Send Note On
                                                         track_data[t_idx].2.push(MidiNoteEvent {
                                                             note,
                                                             velocity: 100,
                                                             channel: 0,
                                                             sample_offset: 0,
                                                         });
                                                         // Register for Note Off (Gate Length ~ 200ms = 8800 samples)
                                                         if t_idx < active_notes.len() {
                                                             active_notes[t_idx].push((note, 8800));
                                                         }
                                                     }
                                                 }
                                             }
                                         }
                                     }
                                 }
                             }
                        }
                    }

                    // 3. Process Graph (Always)
                    if let Ok(mut g) = graph_in_callback.lock() {
                         for (t_idx, &node_idx) in track_indices.iter().enumerate() {
                             // Create temp buffer for this track
                             let mut track_buf = vec![0.0; frames * 2];
                             
                             // Unpack data
                             let (vol, pan, ref events) = track_data[t_idx];

                             if let Some(node) = g.node_mut(node_idx) {
                                 node.process(&mut track_buf, sample_rate_val, events.as_slice());
                             }
                             
                             // Apply Volume and Pan
                             for i in 0..frames {
                                 let left = track_buf[i * 2];
                                 let right = track_buf[i * 2 + 1];
                                 
                                 let mut l_gain = vol;
                                 let mut r_gain = vol;
                                 
                                 // Simple Linear Pan
                                 if pan > 0.0 {
                                     l_gain *= 1.0 - pan;
                                 } else if pan < 0.0 {
                                     r_gain *= 1.0 + pan;
                                 }
                                 
                                 track_buf[i*2] = left * l_gain;
                                 track_buf[i*2+1] = right * r_gain;
                             }

                             // Add to Master Mix
                             for i in 0..master_mix.len() {
                                 master_mix[i] += track_buf[i];
                             }
                         }
                         
                         if playing {
                             pos_counter.fetch_add(frames as u64, Ordering::Relaxed);
                         }
                    }
                     
                     // Interleave back to data
                     // master_mix is already Interleaved (L, R, L, R...)
                     // cpal data buffer is also Interleaved (L, R, L, R...)
                     let gain = f32::from_bits(master_gain_callback.load(Ordering::Relaxed));

                     for i in 0..frames {
                         // Read from Interleaved Master Mix
                         let left = master_mix[i * 2];
                         let right = master_mix[i * 2 + 1];

                         // Write to Interleaved Output
                         if channels == 2 {
                             data[i * 2] = left * gain;
                             data[i * 2 + 1] = right * gain;
                         } else {
                             // Downmix for Mono output? Or just Left?
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
            graph: graph_ref,
            _sequencer: sequencer,
            track_node_indices: track_node_indices,
            project: project_ref,
            master_gain,
        })
    }

    pub fn add_track(&self, plugin_path: Option<&str>) -> Result<(), anyhow::Error> {
        let node: Box<dyn crate::nodes::AudioNode> = if let Some(path) = plugin_path {
             match PluginNode::new(path) {
                 Ok(n) => Box::new(n),
                 Err(e) => {
                     eprintln!("Error loading plugin: {}", e);
                     // Fallback to GainNode (Silent) instead of SineNode
                     Box::new(GainNode::new(1.0))
                 }
             }
        } else {
             // Fallback to GainNode (Silent) instead of SineNode
             Box::new(GainNode::new(1.0))
        };
        
        let node_idx = {
            self.graph.lock().unwrap().add_node(node)
        };
        
        let mut t = Track::default();
        t.name = if plugin_path.is_some() { "Plugin" } else { "Sine" }.into();
        
        self.project.lock().unwrap().tracks.push(t);
        self.track_node_indices.lock().unwrap().push(node_idx);
        
        Ok(())
    }

    pub fn load_plugin_to_track(&self, track_index: usize, plugin_path: &str) -> Result<(), anyhow::Error> {
        let node = match PluginNode::new(plugin_path) {
            Ok(n) => Box::new(n),
            Err(e) => return Err(anyhow::anyhow!("Failed to load plugin: {}", e)),
        };
        
        let mut graph = self.graph.lock().unwrap();
        let indices = self.track_node_indices.lock().unwrap();
        
        if let Some(&node_idx) = indices.get(track_index) {
            // Replace the node in the graph
            // Note: AudioGraph likely uses petgraph. 
            // We need to access the node weight and replace it.
            // Since we can't easily swap the Box in AudioGraph without an API,
            // we will overwrite the node data if possible, or use petgraph API via AudioGraph.
            // Assumption: AudioGraph exposes node_mut returning &mut Box<dyn AudioNode> or similar?
            // Actually AudioGraph::node_mut returns Option<&mut Box<dyn AudioNode>> usually.
            // Let's check existing code usage: g.node_mut(node_idx).
            
            if let Some(existing_node_ref) = graph.node_mut(node_idx) {
                *existing_node_ref = node;
            } else {
                return Err(anyhow::anyhow!("Node index not found in graph"));
            }
            
            // Validate: check if project track exists and update name
            let mut proj = self.project.lock().unwrap();
            if track_index < proj.tracks.len() {
                // Determine name from path (filename)
                let name = std::path::Path::new(plugin_path)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Plugin")
                    .to_string();
                proj.tracks[track_index].name = name;
            }
            
            Ok(())
        } else {
             Err(anyhow::anyhow!("Track index out of bounds"))
        }
    }
// ...

    pub fn is_playing(&self) -> bool {
        self.is_playing.load(Ordering::Relaxed)
    }

    pub fn sample_position(&self) -> u64 {
        self.sample_position.load(Ordering::Relaxed)
    }

    pub fn get_current_step(&self) -> u32 {
        if let Ok(seq) = self._sequencer.lock() {
            seq.current_step()
        } else {
            0
        }
    }
}
