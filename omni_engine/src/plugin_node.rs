use crate::nodes::AudioNode;
use omni_shared::{HostCommand, OmniShmemHeader, PluginEvent, OMNI_MAGIC};
use shared_memory::ShmemConf;
use std::process::{Child, Command, Stdio};
use std::io::{Write, BufReader, BufRead};
use bincode;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

#[allow(dead_code)]
pub struct PluginNode {
    process: Child, // Kept alive
    shmem: shared_memory::Shmem,
    stdin: Option<std::process::ChildStdin>,
    reader: Option<BufReader<std::process::ChildStdout>>,
    plugin_path: String,
    shmem_config: omni_shared::ShmemConfig,
    param_cache: std::collections::HashMap<u32, f32>,
}

unsafe impl Sync for PluginNode {}
unsafe impl Send for PluginNode {}

impl PluginNode {
    pub fn new(plugin_path: &str) -> Result<Self, anyhow::Error> {
        // 1. Setup Shared Memory
        let shmem_config = omni_shared::ShmemConfig {
            os_id: format!("/omni_{}", uuid::Uuid::new_v4()),
            size: 65536, 
        };
        // Removed mut
        let shmem = ShmemConf::new()
            .size(shmem_config.size)
            .os_id(&shmem_config.os_id)
            .create()?;

        unsafe {
            let ptr = shmem.as_ptr();
            let header = ptr as *mut OmniShmemHeader;
            (*header).magic = OMNI_MAGIC; 
            (*header).input_offset = 0;
            (*header).output_offset = 0;
        }

        // 2. Spawn Plugin Host
        let mut child = Command::new("./target/debug/omni_plugin_host")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit()) 
            .spawn()?;

        let mut stdin = child.stdin.take().ok_or(anyhow::anyhow!("Failed to open stdin"))?;
        let stdout = child.stdout.take().ok_or(anyhow::anyhow!("Failed to open stdout"))?;
        let mut reader = BufReader::new(stdout);

        // 3. Handshake: Initialize
        let init_cmd = HostCommand::Initialize { 
            plugin_id: uuid::Uuid::new_v4(),
            shmem_config: shmem_config.clone() 
        };
        let serialized = bincode::serialize(&init_cmd)?;
        let encoded = BASE64.encode(serialized);
        writeln!(stdin, "{}", encoded)?;

        // Wait for Initialized event
        let mut line = String::new();
        reader.read_line(&mut line)?;
        let decoded = BASE64.decode(line.trim())?;
        let event: PluginEvent = bincode::deserialize(&decoded)?;

        if !matches!(event, PluginEvent::Initialized) {
            return Err(anyhow::anyhow!("Plugin failed to initialize"));
        }
        
        // 4. Load Plugin
        let load_cmd = HostCommand::LoadPlugin { path: plugin_path.to_string() };
        let serialized_load = bincode::serialize(&load_cmd)?;
        writeln!(stdin, "{}", BASE64.encode(serialized_load))?;

        // Wait for PluginLoaded
        let mut line = String::new();
        reader.read_line(&mut line)?;
        let decoded = BASE64.decode(line.trim())?;
        Ok(Self {
            process: child,
            shmem,
            stdin: Some(stdin),
            reader: Some(reader),
            plugin_path: plugin_path.to_string(),
            shmem_config: shmem_config,
            param_cache: std::collections::HashMap::new(),
        })
    }

    pub fn check_resurrection(&mut self) -> Result<(), anyhow::Error> {
        if let Ok(Some(_status)) = self.process.try_wait() {
            eprintln!("[PluginNode] CRASH DETECTED! Resurrecting...");
            
            // 1. Spawn Plugin Host again
            let mut child = Command::new("./target/debug/omni_plugin_host")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit()) 
                .spawn()?;

            let mut stdin = child.stdin.take().ok_or(anyhow::anyhow!("Failed to open stdin"))?;
            let stdout = child.stdout.take().ok_or(anyhow::anyhow!("Failed to open stdout"))?;
            let mut reader = BufReader::new(stdout);

            // 2. Handshake: Initialize (reuse shmem_config)
            let init_cmd = HostCommand::Initialize { 
                plugin_id: uuid::Uuid::new_v4(),
                shmem_config: self.shmem_config.clone() 
            };
            let serialized = bincode::serialize(&init_cmd)?;
            writeln!(stdin, "{}", BASE64.encode(serialized))?;

            // Skip reading initializations for now to be fast, but ideally we wait.
            // Let's at least read one line.
            let mut line = String::new();
            reader.read_line(&mut line)?; 

            // 3. Load Plugin again
            let load_cmd = HostCommand::LoadPlugin { path: self.plugin_path.clone() };
            let serialized_load = bincode::serialize(&load_cmd)?;
            writeln!(stdin, "{}", BASE64.encode(serialized_load))?;
            reader.read_line(&mut line)?; // Wait for PluginLoaded

            self.process = child;
            self.stdin = Some(stdin);
            self.reader = Some(reader);
            eprintln!("[PluginNode] Resurrection COMPLETE.");

            // 4. Restore State (Parameter Shadowing)
            if !self.param_cache.is_empty() {
                if let Some(stdin_ref) = self.stdin.as_mut() {
                    eprintln!("[PluginNode] Restoring {} parameters...", self.param_cache.len());
                    for (&id, &val) in &self.param_cache {
                        let cmd = HostCommand::SetParameter { param_id: id, value: val };
                        if let Ok(serialized) = bincode::serialize(&cmd) {
                            let _ = writeln!(stdin_ref, "{}", BASE64.encode(serialized));
                        }
                    }
                    let _ = stdin_ref.flush();
                }
            }
        }
        Ok(())
    }
}

impl Drop for PluginNode {
    fn drop(&mut self) {
    }
}

impl AudioNode for PluginNode {
    fn process(&mut self, output: &mut [f32], _sample_rate: f32, midi_events: &[omni_shared::MidiNoteEvent]) {
        // Resurrection check
        let _ = self.check_resurrection();

        let count = output.len() as u32;

        // 1. Copy Audio to Shmem
        unsafe {
            let data_ptr = self.shmem.as_ptr().add(std::mem::size_of::<OmniShmemHeader>()) as *mut f32;
            std::ptr::copy_nonoverlapping(output.as_ptr(), data_ptr, output.len());
        }

        // 2. Send Process Command
        if let Some(stdin) = &mut self.stdin {
            let cmd = if midi_events.is_empty() {
                HostCommand::ProcessFrame { count }
            } else {
                HostCommand::ProcessWithMidi { count, events: midi_events.to_vec() }
            };

            if let Ok(serialized) = bincode::serialize(&cmd) {
                 let _ = writeln!(stdin, "{}", BASE64.encode(serialized));
                 let _ = stdin.flush();
            }
        }

        // 3. Wait for Reply (Blocking)
        if let Some(reader) = &mut self.reader {
            let mut line = String::new();
            let _ = reader.read_line(&mut line);
        }

        // 4. Read Audio back from Shmem
        unsafe {
            let data_ptr = self.shmem.as_ptr().add(std::mem::size_of::<OmniShmemHeader>()) as *const f32;
            std::ptr::copy_nonoverlapping(data_ptr, output.as_mut_ptr(), output.len());
        }
    }

    fn set_param(&mut self, id: u32, value: f32) {
        // Cache the value for resurrection
        self.param_cache.insert(id, value);

        if let Some(stdin) = &mut self.stdin {
            let cmd = HostCommand::SetParameter { param_id: id, value };
            if let Ok(serialized) = bincode::serialize(&cmd) {
                 let _ = writeln!(stdin, "{}", BASE64.encode(serialized));
                 let _ = stdin.flush();
            }
        }
    }

    fn get_plugin_params(&mut self) -> Vec<omni_shared::ParamInfo> {
        self.get_params().unwrap_or_default()
    }

    fn simulate_crash(&mut self) {
        eprintln!("[PluginNode] SIMULATING CRASH: Killing child process...");
        let _ = self.process.kill();
        // We don't wait here; we let check_resurrection handle it in next process call
    }

    fn open_editor(&mut self) {
        if let Some(stdin) = &mut self.stdin {
            let cmd = HostCommand::OpenEditor;
             if let Ok(serialized) = bincode::serialize(&cmd) {
                 let _ = writeln!(stdin, "{}", BASE64.encode(serialized));
                 let _ = stdin.flush();
            }
        }
    }
}

impl PluginNode {
    pub fn get_params(&mut self) -> Result<Vec<omni_shared::ParamInfo>, anyhow::Error> {
        if let Some(stdin) = &mut self.stdin {
            let cmd = HostCommand::GetParamInfo;
            let serialized = bincode::serialize(&cmd)?;
            writeln!(stdin, "{}", BASE64.encode(serialized))?;
            stdin.flush()?;
        }

        if let Some(reader) = &mut self.reader {
            let mut line = String::new();
            reader.read_line(&mut line)?;
            let decoded = BASE64.decode(line.trim())?;
            let event: PluginEvent = bincode::deserialize(&decoded)?;
            if let PluginEvent::ParamInfoList(params) = event {
                return Ok(params);
            }
        }
        
        Err(anyhow::anyhow!("Failed to get parameter info"))
    }
}
