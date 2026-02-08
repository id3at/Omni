

#[cfg(test)]
mod tests {
    use crate::{AudioEngine, EngineCommand};
    use crossbeam_channel::{unbounded, bounded};
    use std::thread;
    use std::time::Duration;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    #[test]
    fn test_recording_integration() {
        // 1. Setup Engine
        let (cmd_tx, cmd_rx) = unbounded();
        let (drop_tx, drop_rx) = unbounded();
        
        // Spawn GC Thread (mock)
        thread::spawn(move || {
            for _ in drop_rx {}
        });

        // Initialize Engine
        // Note: AudioEngine::new spawns the audio thread and recorder thread.
        let engine = AudioEngine::new(cmd_rx, drop_tx).expect("Failed to create engine");
        
        // 2. Simulate Audio Processing (Wait for init)
        thread::sleep(Duration::from_millis(100));
        
        println!("[Test] Engine Initialized.");

        // 3. Start Recording
        cmd_tx.send(EngineCommand::StartRecording).unwrap();
        println!("[Test] Started Recording...");
        
        // 4. Wait / Simulate Playback
        // Since we can't easily inject audio into the input device in this test without more mocking,
        // we rely on the fact that the engine *runs*.
        // Ideally we would mock the `cpal` stream or use a "Dummy" host, but AudioEngine uses cpal directly.
        // However, we can check if the recorder creates a clip even if silence.
        
        // But wait! The issue is "Silent/Empty".
        // If we record silence, we expect a clip with Zeros.
        // If we could inject signal -> we expect non-zeros.
        // For now, let's just verify the PIPELINE (Start -> Stop -> Clip Created).
        
        thread::sleep(Duration::from_millis(1000));
        
        // 5. Stop Recording
        let (resp_tx, resp_rx) = unbounded();
        cmd_tx.send(EngineCommand::StopRecording { response_tx: resp_tx }).unwrap();
        println!("[Test] Stopped Recording. Waiting for response...");
        
        // 6. Assert Response
        let result = resp_rx.recv_timeout(Duration::from_secs(2));
        assert!(result.is_ok(), "Did not receive recording response in time");
        
        let clips = result.unwrap();
        println!("[Test] Received {} clips", clips.len());
        
        // We expect 1 clip per track (default 8 tracks? No, Engine init default is likely 0 or 8?)
        // Mixer defaults to 32 tracks.
        // The loop in StopRecording iterates `recording_buffers`.
        // If buffer is not empty, it creates a clip.
        // "Signal present" check is not in Recorder, it records whatever is pushed.
        // Engine pushes zeros if no input?
        // Actually engine inputs are from `AudioBuffers::track_bufs`.
        // If no plugins, track bufs are zero.
        
        // So we expect clips with silence.
        // But they should EXIST.
        
        assert!(!clips.is_empty(), "Should have created at least one clip (even if silence)");
        
        // 7. Verify Data (Optional: Access AudioPool if possible)
        // engine.audio_pool is Arc<ArcSwap<AudioPool>>.
        let pool = engine.audio_pool.load();
        
        for (track_idx, clip) in clips {
            println!("[Test] Clip on Track {}: ID={}, Len={}", track_idx, clip.source_id, clip.length.samples);
            let asset = pool.get_asset(clip.source_id);
            assert!(asset.is_some(), "Asset should exist in pool");
            let data = &asset.unwrap().data;
            println!("[Test] Asset Data Len: {}", data.len());
            assert!(data.len() > 0, "Asset data should not be empty");
        }
    }
}
