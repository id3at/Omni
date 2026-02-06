use anyhow::Result;
use bincode;
use omni_shared::{HostCommand, PluginEvent, OmniShmemHeader, OMNI_MAGIC};
use shared_memory::{ShmemConf, Shmem};
use uuid::Uuid;
use std::io::{self, BufRead, Write};
use base64::prelude::BASE64_STANDARD as BASE64;
use base64::Engine;

// mod vst3_defs;
// mod vst3_wrapper;
// mod clap_defs;
mod clap_wrapper;

// use vst3_wrapper::Vst3Plugin;
// use vst3_defs::VstPtr;

use clap_wrapper::ClapPlugin;

use winit::event::{Event, WindowEvent};
use winit::window::WindowBuilder;
use winit::event_loop::EventLoopBuilder;

use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use std::fs::OpenOptions;
use std::panic;

#[derive(Debug)]
enum CustomEvent {
    OpenEditor,
    Initialize(Uuid, omni_shared::ShmemConfig),
    LoadPlugin(String, f64),
    Shutdown, 
}

// Wrapper for Shmem to force Send/Sync (we rely on Mutex for safety)
struct SafeShmem(Shmem);
unsafe impl Send for SafeShmem {}
unsafe impl Sync for SafeShmem {}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Setup file logging
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .append(true)
        .open("/tmp/host_debug.log")?;
    
    let log_file = std::sync::Arc::new(std::sync::Mutex::new(file));
    let log_clone = log_file.clone();

    // Panic Hook
    panic::set_hook(Box::new(move |info| {
        let mut f = log_clone.lock().unwrap();
        let _ = writeln!(f, "[PANIC] {:?}", info);
    }));

    {
        let mut f = log_file.lock().unwrap();
        writeln!(f, "[START] Plugin Host Process Started PID: {}", std::process::id())?;
    }

    let stdin = io::stdin();
    // Stdout is shared for sending events
    let stdout = Arc::new(Mutex::new(io::stdout()));

    // Shared State
    let shmem: Arc<Mutex<Option<SafeShmem>>> = Arc::new(Mutex::new(None));
    let plugin: Arc<RwLock<Option<ClapPlugin>>> = Arc::new(RwLock::new(None));
    
    // Event Loop
    // Use X11 backend explicitly if needed, or default
    let event_loop = EventLoopBuilder::<CustomEvent>::with_user_event().build()?;
    let event_loop_proxy = event_loop.create_proxy();

    let plugin_for_ipc = plugin.clone();
    let shmem_for_ipc = shmem.clone();
    let stdout_for_ipc = stdout.clone();
    let log_file_for_ipc = log_file.clone();

    // Spawn IPC Thread
    thread::spawn(move || {
        let mut lines = stdin.lock().lines();
        let log_file = log_file_for_ipc;
        
        // Helper to send events
        let send_event = |event: PluginEvent| -> Result<(), Box<dyn std::error::Error>> {
             let mut out = stdout_for_ipc.lock().unwrap();
             let serialized = bincode::serialize(&event)?;
             let encoded = BASE64.encode(serialized);
             writeln!(out, "{}", encoded)?;
             out.flush()?;
             Ok(())
        };

        while let Some(Ok(line)) = lines.next() {
            if line.trim().is_empty() { continue; }
            
            let decoded = match BASE64.decode(line.trim()) {
                Ok(d) => d,
                Err(e) => {
                    let mut f = log_file.lock().unwrap();
                    let _ = writeln!(f, "[ERROR] Base64 decode error: {}", e);
                    continue;
                }
            };
            
            let command: HostCommand = match bincode::deserialize(&decoded) {
                Ok(c) => c,
                Err(e) => {
                    let mut f = log_file.lock().unwrap();
                    let _ = writeln!(f, "[ERROR] Deserialization error: {}", e);
                    continue;
                }
            };

            match command {
                HostCommand::Initialize { plugin_id, shmem_config } => {
                    let mut f = log_file.lock().unwrap();
                    let _ = writeln!(f, "[CMD] Init (Dispatching to Main Thread) ID: {} Shmem: {}", plugin_id, shmem_config.os_id);
                    let _ = event_loop_proxy.send_event(CustomEvent::Initialize(plugin_id, shmem_config));
                    // Note: We don't send Initialized event here; main thread will do it.
                }
                HostCommand::LoadPlugin { path, sample_rate } => {
                    let mut f = log_file.lock().unwrap();
                    let _ = writeln!(f, "[CMD] LoadPlugin (Dispatching to Main Thread): {} @ {}Hz", path, sample_rate);
                    let _ = event_loop_proxy.send_event(CustomEvent::LoadPlugin(path, sample_rate));
                    // Main thread sends reply.
                }
                HostCommand::ProcessFrame { count } => {
                    if let Some(ref shmem_mapping_safe) = *shmem_for_ipc.lock().unwrap() {
                         let shmem_mapping = &shmem_mapping_safe.0;
                         unsafe {
                            let ptr = shmem_mapping.as_ptr();
                            let header = &mut *(ptr as *mut OmniShmemHeader);
                            
                            if header.magic == OMNI_MAGIC {
                                let data_ptr = shmem_mapping.as_ptr().add(std::mem::size_of::<OmniShmemHeader>()) as *mut f32;
                                let slice = std::slice::from_raw_parts_mut(data_ptr, count as usize);
                                
                                let transport = clap_wrapper::TransportInfo::default();
                                let guard = plugin_for_ipc.read().unwrap();
                                if let Some(ref p) = *guard {
                                    p.process_audio(slice, &[], &[], &[], header, &transport);
                                    
                                    // Update Latency
                                    let latency = unsafe { p.get_latency() };
                                    std::ptr::write_volatile(&mut header.latency, latency);
                                } else {
                                     // Silence or passthrough (here we assume silence if no plugin)
                                     for s in slice.iter_mut() { *s = 0.0; }
                                }
                            }
                         }
                         let _ = send_event(PluginEvent::FrameProcessed);
                    }
                }
                HostCommand::ProcessWithMidi { count, events } => {
                     // Similar logic
                     if let Some(ref shmem_mapping_safe) = *shmem_for_ipc.lock().unwrap() {
                         let shmem_mapping = &shmem_mapping_safe.0;
                         unsafe {
                            let ptr = shmem_mapping.as_ptr();
                            let header = &mut *(ptr as *mut OmniShmemHeader);
                            if header.magic == OMNI_MAGIC {
                                let data_ptr = shmem_mapping.as_ptr().add(std::mem::size_of::<OmniShmemHeader>()) as *mut f32;
                                let slice = std::slice::from_raw_parts_mut(data_ptr, count as usize);
                                
                                let transport = clap_wrapper::TransportInfo::default();
                                let guard = plugin_for_ipc.read().unwrap();
                                if let Some(ref p) = *guard {
                                    p.process_audio(slice, &events, &[], &[], header, &transport);
                                    
                                    // Update Latency
                                    let latency = unsafe { p.get_latency() };
                                    std::ptr::write_volatile(&mut header.latency, latency);
                                }
                            }
                         }
                         let _ = send_event(PluginEvent::FrameProcessed);
                     }
                }
                HostCommand::SetParameter { param_id, value } => {
                     let guard = plugin_for_ipc.read().unwrap();
                     if let Some(ref p) = *guard {
                         p.set_parameter(param_id, value as f64);
                     }
                }
                HostCommand::GetParamInfo => {
                     let guard = plugin_for_ipc.read().unwrap();
                     let mut params = Vec::new();
                     if let Some(ref p) = *guard {
                         unsafe {
                             let count = p.get_param_count();
                             for i in 0..count {
                                 if let Some(info) = p.get_param_info(i) {
                                     let name = std::ffi::CStr::from_ptr(info.name.as_ptr()).to_string_lossy().into_owned();
                                     params.push(omni_shared::ParamInfo {
                                         id: info.id,
                                         name,
                                         min_value: info.min_value,
                                         max_value: info.max_value,
                                         default_value: info.default_value,
                                         flags: info.flags,
                                     });
                                 }
                             }
                         }
                     }
                     let _ = send_event(PluginEvent::ParamInfoList(params));
                }
                HostCommand::OpenEditor => {
                    let _ = event_loop_proxy.send_event(CustomEvent::OpenEditor);
                }
                HostCommand::GetNoteNames => {
                    let guard = plugin_for_ipc.read().unwrap();
                    let (names, clap_id) = if let Some(ref p) = *guard {
                        (unsafe { p.get_note_names() }, p.clap_id.clone())
                    } else {
                        (Vec::new(), String::new())
                    };
                    let _ = send_event(PluginEvent::NoteNameList { clap_id, names });
                }
                HostCommand::Shutdown => {
                    std::process::exit(0);
                }
            }
        }
        // EOF reached (Parent likely closed stdin)
        let _ = event_loop_proxy.send_event(CustomEvent::Shutdown);
    });

    // Spawn Audio Thread (High Priority Poll)
    let plugin_for_audio = plugin.clone();
    let shmem_for_audio = shmem.clone();
    
    thread::Builder::new()
        .name("AudioThread".into())
        .spawn(move || {
            // Wait for Shmem to be available
            let mut shmem_ptr: Option<*mut u8> = None;
            
            loop {
                // 1. Acquire Shmem Pointer (Lazy Init)
                if shmem_ptr.is_none() {
                    if let Ok(guard) = shmem_for_audio.lock() {
                        if let Some(ref s) = *guard {
                            shmem_ptr = Some(s.0.as_ptr());
                        }
                    }
                    if shmem_ptr.is_none() {
                        thread::sleep(std::time::Duration::from_millis(10));
                        continue;
                    }
                    eprintln!("[AudioThread] Shmem attached. Starting poll loop.");
                }

                let base_ptr = shmem_ptr.unwrap();
                let header = unsafe { &mut *(base_ptr as *mut OmniShmemHeader) };
                
                // 2. Poll Command
                // We use relaxed load first to check
                let cmd = unsafe { std::ptr::read_volatile(&header.command) };
                
                if cmd == omni_shared::CMD_PROCESS {
                    // Process Audio!
                    let count = header.sample_count;
                    let midi_count = header.midi_event_count;
                    let midi_offset = header.midi_offset;

                    unsafe {
                        let data_ptr = base_ptr.add(std::mem::size_of::<OmniShmemHeader>()) as *mut f32;
                        let slice = std::slice::from_raw_parts_mut(data_ptr, count as usize);
                        
                        let midi_slice = if midi_count > 0 {
                            let midi_ptr = base_ptr.add(midi_offset as usize) as *const omni_shared::MidiNoteEvent;
                            std::slice::from_raw_parts(midi_ptr, midi_count as usize)
                        } else {
                            &[]
                        };
                        
                        let param_count = header.param_event_count;
                        let param_offset = header.param_event_offset;
                        let param_slice = if param_count > 0 {
                            let param_ptr = base_ptr.add(param_offset as usize) as *const omni_shared::ParameterEvent;
                            std::slice::from_raw_parts(param_ptr, param_count as usize)
                        } else {
                            &[]
                        };
                        
                        let expr_count = header.expression_event_count;
                        let expr_offset = header.expression_event_offset;
                        let expr_slice = if expr_count > 0 {
                            let expr_ptr = base_ptr.add(expr_offset as usize) as *const omni_shared::ExpressionEvent;
                            std::slice::from_raw_parts(expr_ptr, expr_count as usize)
                        } else {
                            &[]
                        };
                        
                        // Read Transport from shmem
                        let transport = clap_wrapper::TransportInfo {
                            is_playing: header.transport_is_playing != 0,
                            tempo: header.transport_tempo,
                            song_pos_beats: header.transport_song_pos_beats,
                            bar_start_beats: header.transport_bar_start_beats,
                            bar_number: header.transport_bar_number,
                            time_sig_num: header.transport_time_sig_num,
                            time_sig_denom: header.transport_time_sig_denom,
                        };
                        
                        let guard = plugin_for_audio.read().unwrap();
                        if let Some(ref p) = *guard {
                            p.process_audio(slice, midi_slice, param_slice, expr_slice, header, &transport);

                            // Update Latency
                            let latency = unsafe { p.get_latency() };
                            std::ptr::write_volatile(&mut header.latency, latency);
                        } else {
                            // Passthrough/Silence
                             for s in slice.iter_mut() { *s = 0.0; }
                        }
                        
                        // 3. Mark Done
                         std::ptr::write_volatile(&mut header.response, omni_shared::RSP_DONE);
                    }
                    
                    // Wait for Host to ack (Optional, or Host just sets command=IDLE)
                    // The protocol we decided:
                    // PluginNode sets CMD_PROCESS.
                    // Host sets RSP_DONE.
                    // PluginNode sees DONE, reads, sets CDM_IDLE.
                    // Host sees IDLE, sets RSP_IDLE.
                    
                    // So we wait for CMD to become IDLE
                     let mut spin_count = 0;
                     while unsafe { std::ptr::read_volatile(&header.command) } != omni_shared::CMD_IDLE {
                         std::hint::spin_loop();
                         spin_count += 1;
                         if spin_count > 100000 {
                             // Timeout? break
                             break;
                         }
                     }
                     unsafe { std::ptr::write_volatile(&mut header.response, omni_shared::RSP_IDLE); }

                } else {
                    // IDLE State: Spin briefly then sleep
                    // This hybrid approach reduces latency for the incoming command
                    let mut i = 0;
                    while unsafe { std::ptr::read_volatile(&header.command) } != omni_shared::CMD_PROCESS && i < 1000 {
                         std::hint::spin_loop();
                         i += 1;
                    }
                    
                    if unsafe { std::ptr::read_volatile(&header.command) } != omni_shared::CMD_PROCESS {
                         thread::sleep(std::time::Duration::from_micros(50));
                    }
                }
            }
        })?;

    // GUI Logic (Main Thread)
    // We keep track of open windows if needed (for now just one)
    let mut _window: Option<winit::window::Window> = None;

    event_loop.run(move |event, target| { 
        target.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(
            std::time::Instant::now() + std::time::Duration::from_millis(16)
        ));

        match event {
            Event::AboutToWait => {
                 // Check Timers
                let guard = plugin.read().unwrap();
                if let Some(ref p) = *guard {
                    unsafe { p.check_timers(); }
                }
            }
            Event::UserEvent(CustomEvent::Initialize(_plugin_id, shmem_config)) => {
                 let mut f = log_file.lock().unwrap();
                 let _ = writeln!(f, "[Main] Processing Initialize...");
                 match ShmemConf::new().os_id(&shmem_config.os_id).open() {
                    Ok(m) => {
                        let mut s = shmem.lock().unwrap();
                        *s = Some(SafeShmem(m));
                        
                        // Send reply
                        let mut out = stdout.lock().unwrap();
                        if let Ok(serialized) = bincode::serialize(&PluginEvent::Initialized) {
                            let _ = writeln!(out, "{}", BASE64.encode(serialized));
                            let _ = out.flush();
                        }
                    },
                    Err(e) => {
                         let _ = writeln!(f, "[Main] Shmem Error: {:?}", e);
                    }
                }
            }
            Event::UserEvent(CustomEvent::LoadPlugin(path, sample_rate)) => {
                 let mut f = log_file.lock().unwrap();
                 let _ = writeln!(f, "[Main] Processing LoadPlugin: {} @ {}Hz", path, sample_rate);
                 match unsafe { ClapPlugin::load(&path, sample_rate) } {
                    Ok(p) => {
                        let mut guard = plugin.write().unwrap();
                        *guard = Some(p);
                        
                        // Send Reply
                        let mut out = stdout.lock().unwrap();
                        if let Ok(serialized) = bincode::serialize(&PluginEvent::PluginLoaded) {
                             let _ = writeln!(out, "{}", BASE64.encode(serialized));
                             let _ = out.flush();
                             let _ = writeln!(f, "[Main] PluginLoaded Sent.");
                        }
                    },
                    Err(e) => {
                        let _ = writeln!(f, "[Main] Load Error: {:?}", e);
                         let mut out = stdout.lock().unwrap();
                         if let Ok(serialized) = bincode::serialize(&PluginEvent::Error(e.to_string())) {
                              let _ = writeln!(out, "{}", BASE64.encode(serialized));
                              let _ = out.flush();
                         }
                    }
                }
            }
            Event::UserEvent(CustomEvent::OpenEditor) => {
                 let mut f = log_file.lock().unwrap();
                 let _ = writeln!(f, "[GUI] OpenEditor received.");

                 // Check if window already open
                 if _window.is_some() {
                     let _ = writeln!(f, "[GUI] Window already open, focusing...");
                      if let Some(win) = _window.as_ref() {
                          win.focus_window();
                      }
                     return;
                 }

                 // Create Window
                 match WindowBuilder::new()
                     .with_title("Omni Plugin Editor")
                     .with_inner_size(winit::dpi::LogicalSize::new(800.0, 600.0))
                     .build(target) 
                 {
                     Ok(win) => {

                         
                         // Attach Plugin
                         let guard = plugin.read().unwrap();
                         if let Some(ref p) = *guard {
                             unsafe {
                                 // We need to implement attach_gui on ClapPlugin
                                 if let Err(e) = p.attach_to_window(&win) {
                                     let _ = writeln!(f, "[GUI] Failed to attach: {}", e);
                                 } else {
                                     let _ = writeln!(f, "[GUI] Plugin attached to window.");
                                 }
                             }
                         }
                         _window = Some(win);
                     },
                     Err(e) => {
                         let _ = writeln!(f, "[GUI] Failed to create window: {}", e);
                     }
                 }
            }
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                eprintln!("[Event] Window CloseRequested received!");
                // Cleanup Plugin GUI
                let guard = plugin.read().unwrap();
                if let Some(ref p) = *guard {
                    eprintln!("[GUI] Destroying editor...");
                    unsafe {
                        p.destroy_editor();
                    }
                }
                
                _window = None; // Drop window closes it
            }
            Event::WindowEvent { event: _event, .. } => {
               // println!("[Event] Other WindowEvent: {:?}", event); // Too noisy usually, but okay for now if commented out
            }
            Event::UserEvent(CustomEvent::Shutdown) => {
                 let mut f = log_file.lock().unwrap();
                 let _ = writeln!(f, "[Main] Shutdown received via IPC/EOF. Exiting...");
                 target.exit();
            }
            _ => {}
        }
    })?;

    Ok(())
}
