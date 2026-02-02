use anyhow::{Context, Result};
use bincode;
use omni_shared::{HostCommand, PluginEvent, OmniShmemHeader, OMNI_MAGIC};
use shared_memory::{ShmemConf, Shmem};
use uuid::Uuid; // Added Uuid
use std::io::{self, BufRead, Write};
use base64::prelude::BASE64_STANDARD as BASE64;
use base64::Engine;

mod vst3_defs;
mod vst3_wrapper;
// mod clap_defs;
mod clap_wrapper;

// use vst3_wrapper::Vst3Plugin;
// use vst3_defs::VstPtr;

use clap_wrapper::ClapPlugin;

use winit::event_loop::{ControlFlow, EventLoop, EventLoopBuilder};
use winit::event::{Event, WindowEvent};
use winit::window::WindowBuilder;
use winit::platform::x11::EventLoopBuilderExtX11;
use raw_window_handle::{HasWindowHandle, RawWindowHandle, HasRawWindowHandle}; // Added HasRawWindowHandle

use std::sync::{Arc, Mutex};
use std::thread;
use std::collections::HashMap;
use std::fs::OpenOptions; // Added
use std::panic; // Added

#[derive(Debug)]
enum CustomEvent {
    OpenEditor,
    Initialize(Uuid, omni_shared::ShmemConfig),
    LoadPlugin(String),
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
    
    let mut log_file = std::sync::Arc::new(std::sync::Mutex::new(file));
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
    let plugin: Arc<Mutex<Option<ClapPlugin>>> = Arc::new(Mutex::new(None));
    
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
                HostCommand::LoadPlugin { path } => {
                    let mut f = log_file.lock().unwrap();
                    let _ = writeln!(f, "[CMD] LoadPlugin (Dispatching to Main Thread): {}", path);
                    let _ = event_loop_proxy.send_event(CustomEvent::LoadPlugin(path));
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
                                
                                let guard = plugin_for_ipc.lock().unwrap();
                                if let Some(ref p) = *guard {
                                    p.process_audio(slice, 44100.0, &[]);
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
                                
                                let guard = plugin_for_ipc.lock().unwrap();
                                if let Some(ref p) = *guard {
                                    p.process_audio(slice, 44100.0, &events);
                                }
                            }
                         }
                         let _ = send_event(PluginEvent::FrameProcessed);
                     }
                }
                HostCommand::SetParameter { param_id, value } => {
                     let guard = plugin_for_ipc.lock().unwrap();
                     if let Some(ref p) = *guard {
                         p.set_parameter(param_id, value as f64);
                     }
                }
                HostCommand::GetParamInfo => {
                     let guard = plugin_for_ipc.lock().unwrap();
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
                HostCommand::Shutdown => {
                    std::process::exit(0);
                }
            }
        }
    });

    // GUI Logic (Main Thread)
    // We keep track of open windows if needed (for now just one)
    let mut _window: Option<winit::window::Window> = None;

    event_loop.run(move |event, target| { 
        target.set_control_flow(ControlFlow::Wait);

        match event {
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
            Event::UserEvent(CustomEvent::LoadPlugin(path)) => {
                 let mut f = log_file.lock().unwrap();
                 let _ = writeln!(f, "[Main] Processing LoadPlugin: {}", path);
                 match unsafe { ClapPlugin::load(&path) } {
                    Ok(p) => {
                        let mut guard = plugin.lock().unwrap();
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
                 // ... rest of OpenEditor code stays same, just re-indented potentially ...
                 let _ = writeln!(f, "[GUI] OpenEditor received.");

                 // Create Window
                 match WindowBuilder::new()
                     .with_title("Omni Plugin Editor")
                     .with_inner_size(winit::dpi::LogicalSize::new(800.0, 600.0))
                     .build(target) 
                 {
                     Ok(win) => {
                         let raw_window_handle = win.raw_window_handle();
                         
                         // Attach Plugin
                         let guard = plugin.lock().unwrap();
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
                // Cleanup Plugin GUI
                let guard = plugin.lock().unwrap();
                if let Some(ref p) = *guard {
                    unsafe {
                        p.destroy_editor();
                    }
                }
                
                _window = None; // Drop window closes it
            }
            _ => {}
        }
    })?;

    Ok(())
}

