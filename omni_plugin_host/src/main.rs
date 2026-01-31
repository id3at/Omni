use anyhow::{Context, Result};
use bincode;
use omni_shared::{HostCommand, PluginEvent, ShmemConfig, OMNI_MAGIC, OmniShmemHeader};
use shared_memory::ShmemConf;
use std::io::{self, BufRead, Write};
use base64::prelude::*;

fn main() -> Result<()> {
    // Redirect stderr to avoid polluting IPC channel (stdout)
    eprintln!("[Plugin] Starting...");

    let stdin = io::stdin();
    let mut handle = stdin.lock();

    let mut stdout = io::stdout();

    let mut shmem: Option<shared_memory::Shmem> = None;

    loop {
        let mut line = String::new();
        let bytes_read = handle.read_line(&mut line)?;
        if bytes_read == 0 {
            break; // EOF
        }

        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let command_bytes = BASE64_STANDARD.decode(line).context("Failed to decode base64")?;
        let command: HostCommand = bincode::deserialize(&command_bytes).context("Failed to deserialize command")?;

        match command {
            HostCommand::Initialize { plugin_id, shmem_config } => {
                eprintln!("[Plugin] Initializing ID: {}", plugin_id);
                // Open Shared Memory
                let mapping = ShmemConf::new()
                    .os_id(&shmem_config.os_id)
                    .open()
                    .context("Failed to open shared memory")?;
                
                shmem = Some(mapping);
                send_event(&mut stdout, PluginEvent::Initialized)?;
            }
            HostCommand::ProcessFrame => {
                if let Some(ref mapping) = shmem {
                    // Unsafe access to raw memory
                    unsafe {
                        let ptr = mapping.as_ptr();
                        let header = &*(ptr as *const OmniShmemHeader);
                        
                        // Verification (simple magic check)
                        if header.magic != OMNI_MAGIC {
                            eprintln!("[Plugin] Layout mismatch/corruption!");
                            send_event(&mut stdout, PluginEvent::Error("Shmem Magic Mismatch".into()))?;
                            continue;
                        }

                        // Simulate processing
                    }
                    send_event(&mut stdout, PluginEvent::FrameProcessed)?;
                } else {
                    send_event(&mut stdout, PluginEvent::Error("Not Initialized".into()))?;
                }
            }
            HostCommand::SetParameter { param_id, value } => {
                eprintln!("[Plugin] Set Param {} to {}", param_id, value);
            }
            HostCommand::Shutdown => {
                eprintln!("[Plugin] Shutting down.");
                break;
            }
        }
    }

    Ok(())
}

fn send_event(stdout: &mut std::io::Stdout, event: PluginEvent) -> Result<()> {
    let bytes = bincode::serialize(&event)?;
    let encoded = BASE64_STANDARD.encode(bytes);
    writeln!(stdout, "{}", encoded)?;
    stdout.flush()?;
    Ok(())
}
