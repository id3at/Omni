use omni_engine::{AudioEngine, EngineCommand};
use crossbeam_channel::unbounded;
use std::thread;
use std::time::Duration;

fn main() -> Result<(), anyhow::Error> {
    println!("[Headless] Starting test runner...");
    let (tx, rx) = unbounded();
    
    // Initialize engine
    let _engine = AudioEngine::new(rx)?;
    
    println!("[Headless] Engine initialized. Sending PLAY command...");
    tx.send(EngineCommand::Play)?;
    
    // Run for 5 seconds
    println!("[Headless] Running for 5 seconds. Watch for CLAP logs...");
    thread::sleep(Duration::from_secs(5));
    
    println!("[Headless] Sending STOP command...");
    tx.send(EngineCommand::Stop)?;
    thread::sleep(Duration::from_millis(500));
    
    println!("[Headless] Done.");
    Ok(())
}
