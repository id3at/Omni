use std::sync::Arc;
use std::thread;
use std::time::Duration;
use crossbeam_channel::{Receiver, Sender};
use ringbuf::traits::Consumer;
use ringbuf::HeapCons;
use arc_swap::ArcSwap;
use crate::assets::AudioPool;

// Command messages for the Recorder thread
pub enum RecorderCommand {
    Start,
    Stop { 
        // Returns (track_idx, ArrangementClip) pairs
        response_tx: Sender<Vec<(usize, omni_shared::project::ArrangementClip)>>,
        rec_start_sample: u64,
    },
    AddTrack { track_index: usize, consumer: HeapCons<f32> },
    RemoveTrack { track_index: usize },
    Clear,
}

pub struct AudioRecorder {
    command_rx: Receiver<RecorderCommand>,
    consumers: Vec<Option<HeapCons<f32>>>,
    recording_buffers: Vec<Vec<f32>>,
    is_recording: bool,
    audio_pool: Arc<ArcSwap<AudioPool>>,
    sample_rate: f32,
}

impl AudioRecorder {
    pub fn new(command_rx: Receiver<RecorderCommand>, audio_pool: Arc<ArcSwap<AudioPool>>, sample_rate: f32) -> Self {
        Self {
            command_rx,
            consumers: Vec::with_capacity(32),
            recording_buffers: Vec::with_capacity(32),
            is_recording: false,
            audio_pool,
            sample_rate,
        }
    }

    pub fn run(&mut self) {
        loop {
            // Process all pending commands
            while let Ok(cmd) = self.command_rx.try_recv() {
                self.handle_cmd(cmd);
            }

            // Process Audio
            if self.is_recording {
                self.drain_inputs();
            } else {
                self.discard_inputs();
            }

            thread::sleep(Duration::from_millis(5));
        }
    }
    
    fn handle_cmd(&mut self, cmd: RecorderCommand) {
        match cmd {
             RecorderCommand::Start => { self.is_recording = true; eprintln!("[Recorder] Started"); },
             RecorderCommand::Stop { response_tx, rec_start_sample } => {
                self.is_recording = false;
                self.drain_inputs();
                let current_pool = self.audio_pool.load();
                let mut new_pool_map = (**current_pool).clone();
                let mut pool_modified = false;
                let mut created_clips = Vec::new();
                for (track_idx, buf) in self.recording_buffers.iter_mut().enumerate() {
                    if !buf.is_empty() {
                        let asset_data = std::mem::take(buf);
                        let asset_len = asset_data.len();
                        
                        // DEBUG: Analyze Content
                        let mut max_val = 0.0_f32;
                        let mut non_zero = 0;
                        for &s in &asset_data {
                            if s.abs() > 0.0001 { non_zero += 1; }
                            if s.abs() > max_val { max_val = s.abs(); }
                        }
                        eprintln!("[Recorder] Track {} Stats: Len={}, MaxVal={:.4}, NonZeroSamples={}", track_idx, asset_len, max_val, non_zero);
                        
                        // Add to pool
                        let asset_id = new_pool_map.add_asset_from_data(asset_data, self.sample_rate);
                        pool_modified = true;
                         let clip = omni_shared::project::ArrangementClip {
                            source_id: asset_id,
                            start_time: omni_shared::project::Timestamp { samples: rec_start_sample, fractional: 0.0 },
                            start_offset: omni_shared::project::Timestamp::default(),
                            length: omni_shared::project::Timestamp { samples: asset_len as u64, fractional: 0.0 },
                            name: format!("Recorded_{}", asset_id),
                            selected: false,
                            warp_markers: Vec::new(),
                            stretch: false,
                            stretch_ratio: 1.0,
                            original_bpm: 120.0,
                            cached_id: None,
                        };
                        created_clips.push((track_idx, clip));
                    }
                }
                if pool_modified { self.audio_pool.store(Arc::new(new_pool_map)); }
                let _ = response_tx.send(created_clips);
             },
             RecorderCommand::AddTrack { track_index, consumer } => {
                if track_index >= self.consumers.len() {
                    self.consumers.resize_with(track_index + 1, || None);
                    self.recording_buffers.resize_with(track_index + 1, || Vec::new());
                }
                self.consumers[track_index] = Some(consumer);
             },
             RecorderCommand::RemoveTrack { track_index } => {
                if track_index < self.consumers.len() { self.consumers[track_index] = None; }
             },
             RecorderCommand::Clear => {
                 for buf in &mut self.recording_buffers { buf.clear(); }
             },
        }
    }

    fn drain_inputs(&mut self) {
        for (idx, consumer_opt) in self.consumers.iter_mut().enumerate() {
            if let Some(consumer_ref) = consumer_opt {
                let consumer: &mut HeapCons<f32> = consumer_ref;
                if idx < self.recording_buffers.len() {
                    let buf: &mut Vec<f32> = &mut self.recording_buffers[idx];
                     // Explicit loop to help compiler
                     while let Some(sample) = consumer.try_pop() {
                         buf.push(sample);
                     }
                }
            }
        }
    }

    fn discard_inputs(&mut self) {
        for consumer_opt in self.consumers.iter_mut() {
            if let Some(consumer_ref) = consumer_opt {
                let consumer: &mut HeapCons<f32> = consumer_ref;
                // Discard all available samples
                while consumer.try_pop().is_some() {}
            }
        }
    }
}
