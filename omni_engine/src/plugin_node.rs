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
    fn find_plugin_host() -> Result<std::path::PathBuf, anyhow::Error> {
        // 1. Check same directory as executable (Production/Deployment)
        if let Ok(mut path) = std::env::current_exe() {
            path.pop(); // Remove executable name
            path.push("omni_plugin_host");
            if path.exists() {
                return Ok(path);
            }
        }

        // 2. Check Standard Cargo Target dirs (Development)
        // Check for release binary first if in release mode or if debug missing
        let release_path = std::path::Path::new("./target/release/omni_plugin_host");
        let debug_path = std::path::Path::new("./target/debug/omni_plugin_host");

        if !cfg!(debug_assertions) && release_path.exists() {
             return Ok(release_path.to_path_buf());
        }
        
        if debug_path.exists() {
            return Ok(debug_path.to_path_buf());
        }

        if release_path.exists() {
             return Ok(release_path.to_path_buf());
        }

        Err(anyhow::anyhow!("Could not find omni_plugin_host binary! Tried current dir, target/debug, and target/release."))
    }

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
        let binary_path = Self::find_plugin_host()?;
        eprintln!("[PluginNode] Spawning: {:?}", binary_path);

        let mut child = Command::new(&binary_path)
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
        let _decoded = BASE64.decode(line.trim())?;
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
            let binary_path = Self::find_plugin_host()?;
            let mut child = Command::new(binary_path)
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
        let _ = self.process.kill();
        let _ = self.process.wait(); // Clean up zombie
    }
}

impl AudioNode for PluginNode {
    fn process(&mut self, output: &mut [f32], _sample_rate: f32, midi_events: &[omni_shared::MidiNoteEvent], param_events: &[omni_shared::ParameterEvent]) {
        // Resurrection check
        let _ = self.check_resurrection();

        let count = output.len() as u32;

        unsafe {
            let ptr = self.shmem.as_ptr();
            let header = &mut *(ptr as *mut OmniShmemHeader);
            let data_ptr = ptr.add(std::mem::size_of::<OmniShmemHeader>()) as *mut f32;

            // 1. Copy Audio to Shmem
            std::ptr::copy_nonoverlapping(output.as_ptr(), data_ptr, output.len());
            
            // 2. Set Parameters and write MIDI
            header.sample_count = count;
            
            // Serialize MIDI
            let audio_size_bytes = count as usize * std::mem::size_of::<f32>();
            // Note: data_ptr is f32 ptr.
            // MIDI buffer starts after audio.
            // But strict offset calculation:
            // header.midi_offset = sizeof(Header) + audio_size_bytes?
            // Let's settle on a fixed offset logic for now or write it to header.
            
            let midi_offset_bytes = std::mem::size_of::<OmniShmemHeader>() + audio_size_bytes;
            // Pad to 4 bytes alignment if needed (f32 is 4 bytes, so likely aligned if header is aligned)
            
            header.midi_offset = midi_offset_bytes as u32;
            
            let midi_ptr = (ptr as *mut u8).add(midi_offset_bytes) as *mut omni_shared::MidiNoteEvent;
            
            let events_to_write = midi_events.len().min(omni_shared::MAX_MIDI_EVENTS);
             if events_to_write > 0 {
                std::ptr::copy_nonoverlapping(midi_events.as_ptr(), midi_ptr, events_to_write);
            }
            header.midi_event_count = events_to_write as u32;
            
            // NEW: Write Parameter Events
            let param_offset_bytes = midi_offset_bytes + (omni_shared::MAX_MIDI_EVENTS * std::mem::size_of::<omni_shared::MidiNoteEvent>());
            header.param_event_offset = param_offset_bytes as u32;
            header.param_event_count = param_events.len().min(omni_shared::MAX_PARAM_EVENTS) as u32;

            if header.param_event_count > 0 {
                let param_ptr = (ptr as *mut u8).add(param_offset_bytes) as *mut omni_shared::ParameterEvent;
                std::ptr::copy_nonoverlapping(param_events.as_ptr(), param_ptr, header.param_event_count as usize);
            }
            
            // 3. Signal Process
            // std::sync::atomic::fence(Ordering::Release); // Ensure data is visible?
            std::ptr::write_volatile(&mut header.command, omni_shared::CMD_PROCESS);
            
            // 4. Spin Wait
            let mut spin_count = 0;
            const TIMEOUT_SPINS: usize = 200000; // ~ 10ms at 2GHz?
            
            while std::ptr::read_volatile(&header.response) != omni_shared::RSP_DONE {
                spin_count += 1;
                if spin_count < 2000 {
                    std::hint::spin_loop();
                } else {
                    std::thread::yield_now();
                }
                if spin_count > TIMEOUT_SPINS {
                    // Timeout (Plugin hung or crashed)
                    // eprintln!("[PluginNode] Timeout waiting for plugin!");
                    // Detect potential crash
                    return;
                }
            }
            
            // 5. Read Audio back
            std::ptr::copy_nonoverlapping(data_ptr, output.as_mut_ptr(), output.len());
            
            // 6. Reset Handshake
            std::ptr::write_volatile(&mut header.command, omni_shared::CMD_IDLE);
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

    fn get_note_names(&mut self) -> (String, Vec<omni_shared::NoteNameInfo>) {
        match self.query_note_names() {
            Ok(result) => result,
            Err(e) => {
                eprintln!("[PluginNode] Failed to query note names from {}: {}", self.plugin_path, e);
                (String::new(), Vec::new())
            }
        }
    }

    fn get_last_touched(&self) -> (u32, f32, u32) {
        unsafe {
            let ptr = self.shmem.as_ptr();
            let header = &*(ptr as *const OmniShmemHeader);
            let p = std::ptr::read_volatile(&header.last_touched_param);
            let v = std::ptr::read_volatile(&header.last_touched_value);
            let g = std::ptr::read_volatile(&header.touch_generation);
            (p, v, g)
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

    /// Query note names from plugin, returns (clap_id, note_names)
    pub fn query_note_names(&mut self) -> Result<(String, Vec<omni_shared::NoteNameInfo>), anyhow::Error> {
        if let Some(stdin) = &mut self.stdin {
            let cmd = HostCommand::GetNoteNames;
            let serialized = bincode::serialize(&cmd)?;
            writeln!(stdin, "{}", BASE64.encode(serialized))?;
            stdin.flush()?;
        }

        if let Some(reader) = &mut self.reader {
            let mut line = String::new();
            reader.read_line(&mut line)?;
            let decoded = BASE64.decode(line.trim())?;
            let event: PluginEvent = bincode::deserialize(&decoded)?;
            if let PluginEvent::NoteNameList { clap_id, names } = event {
                return Ok((clap_id, names));
            }
        }

        Err(anyhow::anyhow!("Failed to get note names"))
    }
    pub fn get_last_touched(&self) -> (u32, f32, u32) {
        unsafe {
            let ptr = self.shmem.as_ptr();
            let header = &*(ptr as *const OmniShmemHeader);
            let p = std::ptr::read_volatile(&header.last_touched_param);
            let v = std::ptr::read_volatile(&header.last_touched_value);
            let g = std::ptr::read_volatile(&header.touch_generation);
            (p, v, g)
        }
    }
}
