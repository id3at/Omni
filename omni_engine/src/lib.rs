pub mod graph;
pub mod nodes;

use crate::graph::AudioGraph;
use crate::nodes::{SineNode, GainNode};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::Receiver;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

pub struct AudioEngine {
    _stream: cpal::Stream,
    is_playing: Arc<AtomicBool>,
    sample_position: Arc<AtomicU64>,
    // Graph needs to be shared/mutable. Mutex for now until we do the lock-free swap.
    _graph: Arc<Mutex<AudioGraph>>, 
}

pub enum EngineCommand {
    Play,
    Pause,
    Stop,
    SetVolume(f32),
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

        let play_flag = is_playing.clone();
        let pos_counter = sample_position.clone();
        
        // Initialize Graph (Sine -> Gain)
        let mut graph = AudioGraph::new();
        let sine = graph.add_node(Box::new(SineNode::new(440.0)));
        let gain = graph.add_node(Box::new(GainNode::new(0.1))); // Start low
        graph.add_edge(sine, gain);
        
        let graph_ref = Arc::new(Mutex::new(graph));
        let graph_in_callback = graph_ref.clone();
        
        // We need to know the gain node index to control volume via command? 
        // For this prototype, we'll cheat and assume we know it or iterate.
        // Or better yet, we just rebuild/param-set on the protected graph.
        
        // To handle commands that modify the graph (SetVolume), we'll need to lock inside the command loop. 
        // NOTE: Locking in audio thread is bad practice (not real-time safe), but acceptable for Phase 3 functional prototype.
        // The "Zero-Crash" goal will replace this with atomic swap later.

        let err_fn = |err| eprintln!("an error occurred on stream: {}", err);

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
                            }
                            EngineCommand::SetVolume(v) => {
                                // Lock and update gain
                                if let Ok(mut g) = graph_in_callback.lock() {
                                    // In a real system we'd have IDs. 
                                    // Here we just know the 2nd node added (idx 1) is gain.
                                    // Or we iterate.
                                    if let Some(node) = g.node_mut(petgraph::graph::NodeIndex::new(1)) {
                                        node.set_param(0, v);
                                    }
                                }
                            }
                        }
                    }

                    let playing = play_flag.load(Ordering::Relaxed);

                    // We process chunks. Ideally we process 1 buffer of N frames.
                    // Data comes as [L, R, L, R...] flat buffer.
                    // Our graph assumes mono processing for now? 
                    // Let's make the graph write to a mono buffer, then copy to stereo.
                    
                    if playing {
                         let frames = data.len() / channels;
                         let mut mono_buf = vec![0.0; frames]; // Allocation in audio thread -> BAD, but OK for prototype.
                         
                         if let Ok(mut g) = graph_in_callback.lock() {
                             g.process(&mut mono_buf, sample_rate as f32);
                             pos_counter.fetch_add(frames as u64, Ordering::Relaxed);
                         }
                         
                         // Interleave to output
                         for (i, frame) in data.chunks_mut(channels).enumerate() {
                             let sample = mono_buf[i];
                             for channel in frame {
                                 *channel = sample;
                             }
                         }
                    } else {
                        data.fill(0.0);
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
            _graph: graph_ref,
        })
    }
// ...

    pub fn is_playing(&self) -> bool {
        self.is_playing.load(Ordering::Relaxed)
    }

    pub fn sample_position(&self) -> u64 {
        self.sample_position.load(Ordering::Relaxed)
    }
}
