use crate::commands::EngineCommand;
use crate::graph::AudioGraph;
use crate::nodes::{GainNode}; 

use crate::sequencer::{Sequencer, StepGenerator};
use crate::assets::AudioPool; // Added
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{Receiver, Sender};
use omni_shared::project::{Project, Track};
use omni_shared::MidiNoteEvent;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU32, Ordering};
// use petgraph::graph::NodeIndex;
use std::sync::Arc;
use crate::nodes::AudioNode;
use arc_swap::ArcSwap;



pub struct AudioEngine {
    _stream: cpal::Stream,
    is_playing: Arc<AtomicBool>,
    pub is_recording: Arc<AtomicBool>, // Recording to Arrangement
    pub sample_position: Arc<AtomicU64>,
    pub recording_start_sample: Arc<AtomicU64>, // Sample position when recording started
    _sequencer: Sequencer,
    current_step: Arc<AtomicU32>,
    pub sample_rate: u32,
    pub audio_pool: Arc<ArcSwap<AudioPool>>,
    pub drop_tx: Sender<Box<dyn AudioNode>>, // Off-thread dropping
}




// EngineCommand moved to commands.rs

impl AudioEngine {
    pub fn new(command_rx: Receiver<EngineCommand>, drop_tx: Sender<Box<dyn AudioNode>>) -> Result<Self, anyhow::Error> {
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
        let record_flag = Arc::new(AtomicBool::new(false)); // Recording to Arrangement
        let pos_counter = Arc::new(AtomicU64::new(0));
        let rec_start_counter = Arc::new(AtomicU64::new(0)); // Recording start sample
        let master_gain = Arc::new(AtomicU32::new(1.0f32.to_bits()));
        let current_step = Arc::new(AtomicU32::new(0));

        let is_playing = play_flag.clone();
        let is_recording = record_flag.clone();
        let sample_position = pos_counter.clone();
        let recording_start_sample = rec_start_counter.clone();
        let master_gain_callback = master_gain.clone();
        let current_step_callback = current_step.clone();
        
        // Audio Pool
        let audio_pool = Arc::new(ArcSwap::from_pointee(AudioPool::new()));
        let pool_for_callback = audio_pool.clone(); // Clone for audio thread

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
        // Audio Buffers managed by Mixer
        let max_tracks = 32;
        let max_buffer_size = 2048 * 2;
        let mut audio_buffers = crate::mixer::AudioBuffers::new(max_tracks, max_buffer_size);

        // PDC Delays
        let mut track_delays: Vec<crate::delay::DelayLine> = Vec::new();
        
        let mut crossfade: f32 = 0.0; // 0.0 = Session, 1.0 = Arrangement
        
        // Local buffer for parameter events to persist across command loop
        // (Since audio_buffers.prepare_buffers clears the main event vector)
        let mut local_param_events: Vec<Vec<omni_shared::ParameterEvent>> = vec![vec![]; max_tracks];
        let drop_tx_struct = drop_tx.clone();

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
                            EngineCommand::SetArrangementMode(mode) => {
                                project.arrangement_mode = mode;
                                eprintln!("[Engine] Arrangement Mode: {}", mode);
                            }
                            EngineCommand::MoveClip { track_index, clip_index, new_start } => {
                                if let Some(track) = project.tracks.get_mut(track_index) {
                                    if let Some(clip) = track.arrangement.clips.get_mut(clip_index) {
                                        clip.start_time.samples = new_start;
                                    }
                                }
                            }
                            EngineCommand::StretchClip { track_index, clip_index, original_bpm } => {
                                let project_bpm = project.bpm;
                                let mut result_id = None;
                                let mut target_ratio = 1.0;
                                let source_id; // Uninitialized
                                
                                if let Some(track) = project.tracks.get(track_index) {
                                    if let Some(clip) = track.arrangement.clips.get(clip_index) {
                                        source_id = clip.source_id;
                                        if original_bpm > 0.0 {
                                            target_ratio = project_bpm / original_bpm;
                                            
                                            // RCU Update for Stretched Clip
                                            // Clone current pool
                                            let current_pool = pool_for_callback.load();
                                            let mut new_pool = (**current_pool).clone();
                                            
                                            // Modify new pool
                                            if let Ok(id) = new_pool.get_or_create_stretched(source_id, target_ratio) {
                                                result_id = Some(id);
                                                // Atomic Swap
                                                pool_for_callback.store(Arc::new(new_pool));
                                                eprintln!("[Engine] Stretched Clip {}:{} (Source {}) Ratio {:.2} -> Asset {}", track_index, clip_index, source_id, target_ratio, id);
                                            }
                                        }
                                    }
                                }
                                
                                // Apply Update
                                if let Some(id) = result_id {
                                    if let Some(track) = project.tracks.get_mut(track_index) {
                                        if let Some(clip) = track.arrangement.clips.get_mut(clip_index) {
                                            clip.cached_id = Some(id);
                                            clip.stretch = true;
                                            clip.stretch_ratio = target_ratio;
                                            // TODO: Store original_bpm in AudioAsset metadata? Or Clip?
                                            // For now assuming UI passes it. 
                                        }
                                    }
                                }
                            }
                            EngineCommand::SetClipLength { track_index, clip_index, length } => {
                                if let Some(track) = project.tracks.get_mut(track_index) {
                                    if let Some(clip) = track.clips.get_mut(clip_index) {
                                        clip.length = length;
                                        eprintln!("[Engine] Updated Clip {}:{} Length to {} beats. Seq pattern_length remains {}", track_index, clip_index, length, sequencer.pattern_length);
                                    }
                                }
                            }
                            EngineCommand::StartRecording => {
                                // Start recording to Arrangement
                                // Reset playhead to 0 for simpler UX - recordings always start at 0
                                pos_counter.store(0, Ordering::Relaxed);
                                record_flag.store(true, Ordering::Relaxed);
                                rec_start_counter.store(0, Ordering::Relaxed);
                                
                                // Clear recording buffers
                                for buf in audio_buffers.recording_bufs.iter_mut() {
                                    buf.clear();
                                }
                                
                                // Clear previous recorded clips from arrangement (optional - prevents overlap)
                                // This provides a "fresh recording" each time
                                for track in project.tracks.iter_mut() {
                                    track.arrangement.clips.retain(|clip| !clip.name.starts_with("Recorded_"));
                                }
                                
                                eprintln!("[Engine] Recording Started. Playhead reset to 0.");
                            }
                            EngineCommand::StopRecording { response_tx } => {
                                // Stop recording and finalize
                                record_flag.store(false, Ordering::Relaxed);
                                let rec_start = rec_start_counter.load(Ordering::Relaxed);
                                eprintln!("[Engine] Recording Stopped. Start: {}, Processing {} tracks...", rec_start, audio_buffers.recording_bufs.len());
                                
                                let mut created_clips: Vec<(usize, omni_shared::project::ArrangementClip)> = Vec::new();
                                
                                // RCU: Load, Clone, Modify, Store
                                // We do this ONCE for all new assets to avoid multiple atomic swaps
                                let current_pool = pool_for_callback.load();
                                let mut new_pool_map = (**current_pool).clone();
                                
                                // Finalize: Convert recording buffers to AudioAssets and ArrangementClips
                                for (track_idx, rec_buf) in audio_buffers.recording_bufs.iter_mut().enumerate() {
                                    if !rec_buf.is_empty() {
                                        // Create AudioAsset from recorded data
                                        let asset_data = std::mem::take(rec_buf);
                                        let asset_len = asset_data.len();
                                        
                                        // Add to local clone of pool
                                        let asset_id = new_pool_map.add_asset_from_data(asset_data, sample_rate as f32);
                                        
                                        eprintln!("[Engine] Track {} : Created Asset {} from {} samples", track_idx, asset_id, asset_len);
                                        
                                        // Create ArrangementClip
                                        let clip = omni_shared::project::ArrangementClip {
                                            source_id: asset_id,
                                            start_time: omni_shared::project::Timestamp { samples: rec_start, fractional: 0.0 },
                                            start_offset: omni_shared::project::Timestamp::default(),
                                            length: omni_shared::project::Timestamp { samples: asset_len as u64, fractional: 0.0 },
                                            name: format!("Recorded_{}", asset_id),
                                            selected: false,
                                            warp_markers: Vec::new(),
                                            stretch: false,
                                            stretch_ratio: 1.0,
                                            original_bpm: project.bpm,
                                            cached_id: None,
                                        };
                                        
                                        // Add to internal project
                                        if let Some(track) = project.tracks.get_mut(track_idx) {
                                            track.arrangement.clips.push(clip.clone());
                                            eprintln!("[Engine] Track {} : Created ArrangementClip starting at sample {}", track_idx, rec_start);
                                        }
                                        
                                        // Collect for response
                                        created_clips.push((track_idx, clip));
                                    }
                                }
                                
                                // Apply updates to pool atomically
                                pool_for_callback.store(Arc::new(new_pool_map));
                                
                                // Send created clips back to UI
                                let _ = response_tx.send(created_clips);
                            }
                            EngineCommand::SetPluginParam { track_index, id, value } => {
                                // 1. Update Engine Node (Cache Only)
                                if let Some(&node_idx) = track_node_indices.get(track_index) {
                                    if let Some(node) = graph.node_mut(node_idx) {
                                        node.set_param(id, value); 
                                    }
                                }
                                
                                // 2. Queue Event for Audio Processing (Lock-Free)
                                if track_index < local_param_events.len() {
                                    local_param_events[track_index].push(omni_shared::ParameterEvent {
                                        param_id: id,
                                        value: value as f64,
                                        sample_offset: 0, // Apply at start of block
                                    });
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
                            EngineCommand::GetProjectState(response_tx) => {
                                let _ = response_tx.send(project.clone());
                            }
                            EngineCommand::LoadProjectState(new_proj, nodes) => {
                                // 1 Reset Graph
                                graph = AudioGraph::new();
                                track_node_indices.clear();
                                
                                // 2 Load Project
                                project = new_proj;
                                
                                // 3 Rebuild Graph from Project & Provided Nodes
                                // We expect nodes to match tracks 1:1, but handle mismatches safely
                                let mut nodes_iter = nodes.into_iter();
                                
                                for _track in &project.tracks {
                                    // Use provided node or fallback to GainNode
                                    let node: Box<dyn AudioNode> = nodes_iter.next().unwrap_or_else(|| {
                                        Box::new(GainNode::new(1.0))
                                    });
                                
                                    let node_idx = graph.add_node(node);
                                    track_node_indices.push(node_idx);
                                }

                                
                                // 4. Restore Plugin State
                                for (t_idx, track) in project.tracks.iter().enumerate() {
                                    if let Some(state_data) = &track.plugin_state {
                                        if let Some(&node_idx) = track_node_indices.get(t_idx) {
                                            if let Some(node) = graph.node_mut(node_idx) {
                                                let _ = node.set_state(state_data.clone());
                                                // eprintln!("[Engine] Restored state for track {}", t_idx);
                                            }
                                        }
                                    }
                                }
                                eprintln!("[Engine] Loaded project state (Non-Blocking Swap)");
                            }
                            EngineCommand::ResetGraph => {
                                graph = AudioGraph::new();
                                track_node_indices.clear();
                                eprintln!("[Engine] Audio Graph Reset");
                            }
                            EngineCommand::NewProject => {
                                // 1. Stop Playback
                                play_flag.store(false, Ordering::Relaxed);
                                pos_counter.store(0, Ordering::Relaxed);
                                
                                // 2. Reset Engine State
                                graph = AudioGraph::new();
                                track_node_indices.clear();
                                project = Project::default();
                                sequencer.reset();
                                active_notes.iter_mut().for_each(|v| v.clear());
                                
                                eprintln!("[Engine] New Project Created (Engine State Reset)");
                            }
                            EngineCommand::RemoveTrack { track_index } => {
                                if track_index < track_node_indices.len() {
                                    let node_idx_to_remove = track_node_indices[track_index];
                                    
                                    // 1. Remove from Graph (Standard Remove - Swap Remove behavior of petgraph!)
                                    // graph.remove_node swaps the last node into the removed node's index to preserve compact indices.
                                    // We must update the track_node_indices for the track that owned that last node.
                                    
                                    // A. Identify the index of the node that will be moved (last node in graph)
                                    // Note: buffers are resized in graph.update_schedule(), so we don't need to manually resize here.
                                    // But we need to know WHICH track owns the last node index?
                                    // Petgraph::remove_node returns the weight of the removed node.
                                    // If we use graph.remove_node, we need to find the node index that was moved.
                                    // Actually, we can implement remove logic in `track_node_indices` carefully.
                                    
                                    // Simplified approach if we assume graph.remove_node behavior:
                                    // If we remove node N, and last node L was at index L_idx:
                                    // L is moved to N_idx.
                                    // So any track pointing to L_idx must now point to N_idx.
                                    
                                    // However, AudioGraph wraps petgraph. Let's assume we add a remove_node method to AudioGraph.
                                    // Or expose graph? `graph.graph` is field.
                                    // Wait, AudioGraph::node_mut is available. logic is inside Engine.
                                    // We need to add remove_node to AudioGraph first? 
                                    // Or use raw graph? AudioGraph implementation is in graph.rs, let's look.
                                    // It has `node_mut`. It does NOT have remove_node.
                                    // We need to add it to AudioGraph to properly clear chains etc.
                                    
                                    // SKIPPING DIRECT GRAPH ACCESS FOR NOW - WE EDIT LIB.RS FIRST
                                    // We will implement `remove_node` in AudioGraph in next step.
                                    // Call it here assuming it exists.
                                    
                                    if let Some(moved_node_old_idx) = graph.remove_node(node_idx_to_remove) {
                                         // If a node was moved (swapped), we need to update the track that pointed to it.
                                         // If None returned, it means we removed the last node (no swap needed).
                                         
                                         // Find track pointing to moved_node_old_idx
                                         if let Some(t_pos) = track_node_indices.iter().position(|&idx| idx == moved_node_old_idx) {
                                             track_node_indices[t_pos] = node_idx_to_remove; // Now points to the new location (swapped slot)
                                         }
                                    }
                                    
                                    // 2. Remove Track Metadata
                                    if track_index < project.tracks.len() {
                                        project.tracks.remove(track_index);
                                    }
                                    
                                    // 2. Remove from Graph (Prevent Memory Leak)
                                    let node_idx_to_remove = track_node_indices[track_index];
                                    
                                    // graph.remove_node performs a swap-remove, moving the last node to the removed index.
                                    // It returns the index of the node that was moved (if any).
                                    // AND it should return the removed node weight so we can drop it off-thread.
                                    
                                    if let Some((moved_node_old_idx, removed_node)) = graph.remove_node_with_return(node_idx_to_remove) {
                                         // Send removed node to drop thread
                                         if let Some(node) = removed_node {
                                             let _ = drop_tx.send(node);
                                         }
                                         
                                         if let Some(swapped_old_idx) = moved_node_old_idx {
                                             // Update index for swapped node
                                            for idx in track_node_indices.iter_mut() {
                                                if *idx == swapped_old_idx {
                                                    *idx = node_idx_to_remove;
                                                    break;
                                                }
                                            }
                                         }
                                    }
                                    
                                    // 3. Remove Node Index Mapping
                                    track_node_indices.remove(track_index);
                                    
                                    // 4. Clean up active notes
                                    active_notes.remove(track_index);
                                    
                                    eprintln!("[Engine] Removed Track {}", track_index);
                                }
                            }
                            EngineCommand::AddTrackNode { node, name, plugin_path } => {
                                let node_idx = graph.add_node(node);
                                let mut t = Track::default();
                                t.name = name;
                                if let Some(p) = plugin_path { t.plugin_path = p; }
                                
                                project.tracks.push(t);
                                track_node_indices.push(node_idx);
                            }
                            EngineCommand::ReplaceTrackNode { track_index, node, name, plugin_path } => {
                                 if let Some(&node_idx) = track_node_indices.get(track_index) {
                                      if let Some(existing_node_ref) = graph.node_mut(node_idx) {
                                          *existing_node_ref = node;
                                          
                                          // Update Project
                                          if track_index < project.tracks.len() {
                                              project.tracks[track_index].name = name;
                                              project.tracks[track_index].plugin_path = plugin_path;
                                          }
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
                            EngineCommand::GetPluginState { track_index, response_tx } => {
                                if let Some(&node_idx) = track_node_indices.get(track_index) {
                                    if let Some(node) = graph.node_mut(node_idx) {
                                        let state = node.get_state().ok();
                                        let _ = response_tx.send(state);
                                    } else {
                                        let _ = response_tx.send(None);
                                    }
                                } else {
                                    let _ = response_tx.send(None);
                                }
                            }
                            EngineCommand::SetPluginState { track_index, data } => {
                                if let Some(&node_idx) = track_node_indices.get(track_index) {
                                    if let Some(node) = graph.node_mut(node_idx) {
                                        let _ = node.set_state(data);
                                    }
                                }
                            }
                            EngineCommand::AddAsset { name, data, source_sample_rate, response_tx } => {
                                // Add directly to pool. No thread spawning, no I/O.
                                // We take the lock briefly.
                                // Add directly to pool (RCU)
                                // Clone, Modify, Store
                                let current = pool_for_callback.load();
                                let mut new_pool = (**current).clone();
                                let id = new_pool.add_asset_from_data(data, source_sample_rate);
                                pool_for_callback.store(Arc::new(new_pool));
                                
                                eprintln!("[Engine] Added Asset '{}' (ID: {})", name, id);
                                let _ = response_tx.send(Ok(id));
                            } // End of Session Mode Loop
                        } // End of Session Mode Else Block
                    } // End of Playing Block
                    
                    // Clear local events for next block (if not used/moved)
                    // Note: We move them below, but vectors remain. Clear them here?
                    // No, we cleared them at start of closure? No, let's clear at start of block logic.
                    // Actually, we append them below. So we must clear them somewhere.
                    // Doing it at start of closure (line 106) would be best, but we are inside closure.
                    // Let's clear them NOW after command loop is done?
                    // NO, we need to use them below.
                    // We must clear them at the START of the NEXT loop.
                    // OR clear them right after usage.


                    let playing = play_flag.load(Ordering::Relaxed);
                    let frames = data.len() / channels;
                    let sample_rate_val = sample_rate as f32;
                    let track_count = track_node_indices.len();

                    // Resize Buffers (Keep Capacity)
                    // Prepare Buffers (Resize & Clear)
                    audio_buffers.prepare_buffers(frames, track_count, max_buffer_size);
                    
                    // Resize local_param_events if needed
                    if local_param_events.len() < track_count {
                        local_param_events.resize(track_count, Vec::new());
                    }
                    
                    // Copy Parameter Events from Command Loop to Audio Buffers
                    for (i, events) in local_param_events.iter_mut().enumerate() {
                        if i < audio_buffers.track_param_events.len() && !events.is_empty() {
                            audio_buffers.track_param_events[i].append(events);
                            // events is now empty after append (moves content) -> Actually Vec::append moves elements.
                            // But does it clear source? Yes, "The elements are moved... source becomes empty."
                        } else {
                             events.clear(); // Ensure clear if not appended (e.g. track removed)
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
                        
                        // Optimize allocation: Use retain instead of drain/push
                        notes.retain_mut(|(note, remaining)| {
                            if *remaining > frames as u64 {
                                *remaining -= frames as u64;
                                true // Keep note
                            } else {
                                // Note Off
                                audio_buffers.track_events[t_idx].push(MidiNoteEvent {
                                    note: *note, velocity: 0, channel: 0, sample_offset: 0,
                                    detune: 0.0,
                                });
                                false // Remove note
                            }
                        });
                        // Remove old logic using drain
                        // let mut survived = Vec::new();
                        // // Explicit type annotation for drain
                        // let drained: std::vec::Drain<(u8, u64)> = notes.drain(..);
                        // for (note, remaining) in drained {
                        //     if remaining > frames as u64 {
                        //         survived.push((note, remaining - frames as u64));
                        //     } else {
                        //         audio_buffers.track_events[t_idx].push(MidiNoteEvent {
                        //             note, velocity: 0, channel: 0, sample_offset: 0,
                        //             detune: 0.0,
                        //         });
                        //     }
                        // }
                        // *notes = survived;
                    }
                    
                    // 2b. If Playing: Generate Sequence Events (Note Ons)
                    // 2b. If Playing: Generate Sequence Events (Note Ons)
                    // 2b. If Playing: Generate Sequence Events (Note Ons) or Arrangement Audio
                    // 2b. If Playing: Generate Sequence Events (Note Ons) or Arrangement Audio
                    if playing {
                         let current_sample = pos_counter.load(Ordering::Relaxed);
                         let frames_u64 = frames as u64;
                         let buffer_end_sample = current_sample + frames_u64;

                         // CALCULATE CROSSFADE
                         let target_crossfade = if project.arrangement_mode { 1.0 } else { 0.0 };
                         if (crossfade - target_crossfade).abs() > 0.001 {
                             let step = 0.1; // Fast fade (approx 10 blocks = 200ms at 2048)
                             if crossfade < target_crossfade {
                                 crossfade = (crossfade + step).min(target_crossfade);
                             } else {
                                 crossfade = (crossfade - step).max(target_crossfade);
                             }
                         } else {
                             crossfade = target_crossfade;
                         }

                         // --- ARRANGEMENT MODE (Sample-Accurate Audio) ---
                         if crossfade > 0.001 {
                              // Lock Audio Pool (Try lock to avoid blocking audio thread hard?)
                             // Unwrap is fine for now, contention should be low.
                              // Lock-Free access (RCU)
                              // Guard implicitly derefs efficiently
                              { 
                                  // Just entering scope to clarify lifetime of guard if needed, but here it's fine.
                                  // Note: We need pool available for inner loop.
                                  // Moved load inside logic or lift up? Lift up slightly.
                                  // Actually we have `pool_for_callback.load()` call in loop.
                              }
                              if true { // Dummy block to preserve structure diff match, replace logic below
                                  for (t_idx, track) in project.tracks.iter().enumerate() {
                                     if t_idx >= track_count || track.mute { continue; }
                                     
                                     let track_vol = audio_buffers.track_vols[t_idx];
                                     let track_pan = audio_buffers.track_pans[t_idx];
                                     
                                     // Iterate Arrangement Clips
                                     for clip in &track.arrangement.clips {
                                         // Check overlap with current buffer
                                         let clip_start = clip.start_time.samples;
                                         let clip_end = clip_start + clip.length.samples;
                                         
                                         if clip_end > current_sample && clip_start < buffer_end_sample {
                                             // Calculate intersection
                                             let render_start = clip_start.max(current_sample);
                                             let render_end = clip_end.min(buffer_end_sample);
                                             
                                             if render_end > render_start {
                                                 let buffer_offset = (render_start - current_sample) as usize;
                                                 let length = (render_end - render_start) as usize;
                                                 let source_offset = (render_start - clip_start + clip.start_offset.samples) as usize;
                                                 
                                                 // Get Audio Data (Lock-Free)
                                                 let pool = pool_for_callback.load();
                                                 // pool is Guard<Arc<AudioPool>>
                                                 let asset_id = if clip.stretch { clip.cached_id.unwrap_or(clip.source_id) } else { clip.source_id };
                                                 if let Some(asset) = pool.get_asset(asset_id) {
                                                     let asset_data = &asset.data;
                                                     // Safety check
                                                     if source_offset + length <= asset_data.len() {
                                                         // Mix directly to master_mix (not track_bufs which gets overwritten)
                                                         // Apply track volume, pan, and crossfade
                                                         let mut l_gain = track_vol * crossfade;
                                                         let mut r_gain = track_vol * crossfade;
                                                         
                                                         if track_pan > 0.0 {
                                                             l_gain *= 1.0 - track_pan;
                                                         } else if track_pan < 0.0 {
                                                             r_gain *= 1.0 + track_pan;
                                                         }
                                                         
                                                         for i in 0..length {
                                                             let sample = asset_data[source_offset + i];
                                                             let dst_idx = (buffer_offset + i) * 2;
                                                             // Mix to master directly (mono source -> stereo)
                                                             audio_buffers.master_mix[dst_idx] += sample * l_gain;
                                                             audio_buffers.master_mix[dst_idx + 1] += sample * r_gain;
                                                         }
                                                     }
                                                 }
                                             }
                                         }
                                     }
                                 }
                             }
                             
                             // Update UI step for visual feedback (even in Arrangement)
                             // Be precise
                             let bpm = sequencer.bpm;
                             let samples_per_beat = (sample_rate_val * 60.0) / bpm;
                             let current_beat = (current_sample as f64) / samples_per_beat as f64;
                             current_step_callback.store((current_beat * 4.0) as u32, Ordering::Relaxed);
                         }

                         // --- SESSION MODE (Loop-based Sequencer) ---
                         if crossfade < 0.999 {
                             // Calculate time range for this buffer
                             let bpm = sequencer.bpm;
                             let samples_per_beat = (sample_rate_val * 60.0) / bpm;
                             // let current_sample = pos_counter.load(Ordering::Relaxed); // Already loaded
                             
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
                     } // End Session Else
                } // End if playing

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

                    // 4a. PDC (Plugin Delay Compensation)
                    // Calculate latencies and apply delay to align tracks
                    if track_delays.len() < track_count {
                         let buffer_size_samples = sample_rate as usize * 2; // 2 seconds buffer
                         track_delays.resize_with(track_count, || crate::delay::DelayLine::new(buffer_size_samples, sample_rate as f32));
                    }

                    // Query Latencies
                    let mut latencies: Vec<u32> = vec![0; track_count]; 
                    let mut max_latency = 0;
                    
                    for (i, &node_idx) in track_node_indices.iter().enumerate() {
                        // Unsafe access hack or just use node_mut? 
                        // We have mut ref to graph.
                        if let Some(node) = graph.node_mut(node_idx) {
                             let l = node.get_latency();
                             latencies[i] = l;
                             if l > max_latency { max_latency = l; }
                        }
                    }

                    // Apply Delays
                    if max_latency > 0 {
                         for i in 0..track_count {
                              let needed_delay = max_latency - latencies[i];
                              if needed_delay > 0 {
                                  // Use buf_slice to respect borrowing
                                  let track_buf = &mut buf_slice[i];
                                  // Ensure DelayLine is ready
                                  if i < track_delays.len() {
                                      track_delays[i].process_in_place(track_buf, needed_delay);
                                  }
                              } else {
                                  // 0 Delay
                                  let track_buf = &mut buf_slice[i];
                                  if i < track_delays.len() {
                                      track_delays[i].process_in_place(track_buf, 0); 
                                  }
                              }
                         }
                    }

                    // 4b. Mix to Master

                    // 4. Mix to Master
                    // 4. Mix to Master
                    crate::mixer::AudioBuffers::mix_to_master(
                        &audio_buffers.track_bufs,
                        &mut audio_buffers.master_mix,
                        &audio_buffers.track_vols,
                        &audio_buffers.track_pans,
                        frames,
                        track_count
                    );
                     
                     // 4c. Recording Capture (Session -> Arrangement)
                     // Capture audio only when recording in Session mode (not arrangement)
                     let is_rec = record_flag.load(Ordering::Relaxed);
                     if is_rec && !project.arrangement_mode && playing {
                         // Log once per second approx
                         static mut LAST_LOG: u64 = 0;
                         let current_pos = pos_counter.load(Ordering::Relaxed);
                         if unsafe { current_pos.saturating_sub(LAST_LOG) > sample_rate as u64 } {
                             eprintln!("[Engine] Recording: capturing {} tracks, {} frames", track_count, frames);
                             unsafe { LAST_LOG = current_pos };
                         }
                         
                         for t_idx in 0..track_count {
                             // Downmix stereo to mono for recording buffer
                             for i in 0..frames {
                                 let mono = (audio_buffers.track_bufs[t_idx][i * 2] + audio_buffers.track_bufs[t_idx][i * 2 + 1]) * 0.5;
                                 audio_buffers.recording_bufs[t_idx].push(mono);
                             }
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
            is_recording,
            sample_position,
            recording_start_sample,
            _sequencer: Sequencer::new(120.0), // Placeholder
            current_step,
            sample_rate,
            audio_pool,
            drop_tx: drop_tx_struct,
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
