use clap_sys::entry::clap_plugin_entry;
use libc;
use clap_sys::factory::plugin_factory::clap_plugin_factory;
use clap_sys::host::clap_host;
use clap_sys::plugin::clap_plugin;
use clap_sys::version::CLAP_VERSION;
use clap_sys::process::clap_process;
use clap_sys::audio_buffer::clap_audio_buffer;
use clap_sys::events::{
    clap_input_events, clap_output_events, clap_event_header, clap_event_note,
    CLAP_CORE_EVENT_SPACE_ID, CLAP_EVENT_NOTE_ON, CLAP_EVENT_NOTE_OFF,
    clap_event_param_value, CLAP_EVENT_PARAM_VALUE,
};
use clap_sys::ext::params::{clap_plugin_params, CLAP_EXT_PARAMS, clap_param_info};
use clap_sys::ext::gui::{clap_plugin_gui, CLAP_EXT_GUI, clap_window, CLAP_WINDOW_API_X11}; // Assumes clap-sys generic enough
use winit::window::Window;
use raw_window_handle::{HasRawWindowHandle, RawWindowHandle};

use libloading::os::unix::Library as UnixLibrary;
use libloading::{Library, Symbol};
use std::ptr;
use std::ffi::{CString, CStr, c_void};
use std::sync::{Arc, Mutex};
use std::os::raw::c_char;
use anyhow::{Result, anyhow};

use omni_shared::MidiNoteEvent;

pub struct ClapPlugin {
    _library: Arc<Library>,
    pub plugin: *const clap_plugin,
    // Boxed host to ensure pointer stability
    _host_box: Box<clap_host>, 
    pub params: *const clap_plugin_params,
    pending_params: Arc<Mutex<Vec<(u32, f64)>>>,
}

unsafe impl Send for ClapPlugin {}
unsafe impl Sync for ClapPlugin {}

// Host Callbacks
extern "C" fn host_get_extension(_host: *const clap_host, _extension_id: *const c_char) -> *const c_void {
    ptr::null()
}

extern "C" fn host_request_restart(_host: *const clap_host) {}
extern "C" fn host_request_process(_host: *const clap_host) {}
extern "C" fn host_request_callback(_host: *const clap_host) {}

// Event list context for input events
struct EventListContext {
    events: Vec<clap_event_note>,
    param_events: Vec<clap_event_param_value>,
}

// Input events callback: get size
unsafe extern "C" fn input_events_size(list: *const clap_input_events) -> u32 {
    let ctx = (*list).ctx as *const EventListContext;
    ((*ctx).events.len() + (*ctx).param_events.len()) as u32
}

// Input events callback: get event
unsafe extern "C" fn input_events_get(list: *const clap_input_events, index: u32) -> *const clap_event_header {
    let ctx = (*list).ctx as *const EventListContext;
    let note_count = (*ctx).events.len();
    if (index as usize) < note_count {
        &(&(*ctx).events)[index as usize].header as *const clap_event_header
    } else {
        let param_idx = (index as usize) - note_count;
        if param_idx < (*ctx).param_events.len() {
            &(&(*ctx).param_events)[param_idx].header as *const clap_event_header
        } else {
            ptr::null()
        }
    }
}

// Output events callback: try_push (we ignore output events for now)
unsafe extern "C" fn output_events_try_push(_list: *const clap_output_events, _event: *const clap_event_header) -> bool {
    true // Accept but ignore
}

impl ClapPlugin {
    pub unsafe fn load(path: &str) -> Result<Self> {
        let lib_unix = UnixLibrary::open(Some(path), libc::RTLD_NOW | libc::RTLD_LOCAL)?;
        let library = Arc::new(Library::from(lib_unix));
        
        // Get entry point (clap_entry is a static struct, not a function)
        let entry_ptr: Symbol<*const clap_plugin_entry> = 
            library.get(b"clap_entry")?;

        let entry = *entry_ptr;
        
        if entry.is_null() {
            return Err(anyhow!("clap_entry is null"));
        }

        // 1. Init Entry
        let lib_path_c = CString::new(path)?;
        if let Some(init) = (*entry).init {
             if !init(lib_path_c.as_ptr()) {
                 return Err(anyhow!("entry.init failed"));
             }
        } else {
             return Err(anyhow!("entry.init not defined"));
        }

        // 2. Get Factory
        let get_factory = (*entry).get_factory.ok_or(anyhow!("get_factory not defined"))?;
        let factory_id = b"clap.plugin-factory\0";
        let factory_ptr = get_factory(factory_id.as_ptr() as *const c_char);
        if factory_ptr.is_null() {
             return Err(anyhow!("Failed to get clap.plugin-factory"));
        }
        let factory = factory_ptr as *const clap_plugin_factory;

        // 3. Get Plugin Count
        let get_plugin_count = (*factory).get_plugin_count.ok_or(anyhow!("get_plugin_count not defined"))?;
        let count = get_plugin_count(factory);
        if count == 0 {
            return Err(anyhow!("No plugins found in factory"));
        }
        
        let get_plugin_descriptor = (*factory).get_plugin_descriptor.ok_or(anyhow!("get_plugin_descriptor not defined"))?;

        // Just take the first one
        let desc = get_plugin_descriptor(factory, 0); 
        let plugin_id = (*desc).id;
        
        let name = CStr::from_ptr((*desc).name).to_string_lossy();
        eprintln!("[CLAP] Loading: {}", name);

        // 4. Create Host Structure
        let host_name = CString::new("OmniHost")?;
        let host_vendor = CString::new("Id3at")?;
        let host_url = CString::new("https://omni.local")?;
        let host_version = CString::new("0.1.0")?;

        let host = Box::new(clap_host {
            clap_version: CLAP_VERSION,
            host_data: ptr::null_mut(),
            name: host_name.as_ptr(),
            vendor: host_vendor.as_ptr(),
            url: host_url.as_ptr(),
            version: host_version.as_ptr(),
            get_extension: Some(host_get_extension),
            request_restart: Some(host_request_restart),
            request_process: Some(host_request_process),
            request_callback: Some(host_request_callback),
        });

        // 5. Create Plugin Instance
        let host_ptr = &*host as *const clap_host;
        let create_plugin = (*factory).create_plugin.ok_or(anyhow!("create_plugin not defined"))?;
        
        let plugin = create_plugin(factory, host_ptr, plugin_id);
        if plugin.is_null() {
            return Err(anyhow!("create_plugin failed"));
        }

        // 6. Init Plugin
        if let Some(init) = (*plugin).init {
             if !init(plugin) {
                 return Err(anyhow!("plugin.init failed"));
             }
        }

        eprintln!("[CLAP] Loaded successfully.");

        // 8. Get Params Extension
        let params = if let Some(get_ext) = (*plugin).get_extension {
            get_ext(plugin, CLAP_EXT_PARAMS.as_ptr()) as *const clap_plugin_params
        } else {
            ptr::null()
        };

        // 7. Activate
        if let Some(activate) = (*plugin).activate {
             if !activate(plugin, 44100.0, 32, 4096) {
                 eprintln!("[CLAP] Warning: activate failed");
             }
        }

        // 8. Start Processing
        if let Some(start_processing) = (*plugin).start_processing {
             if !start_processing(plugin) {
                 eprintln!("[CLAP] Warning: start_processing failed");
             }
        }

        Ok(Self {
            _library: library,
            plugin,
            _host_box: host,
            params,
            pending_params: Arc::new(Mutex::new(Vec::new())),
        })
    }

    /// Process audio with optional MIDI note events
    pub unsafe fn process_audio(
        &self, 
        output_buffer: &mut [f32], 
        _sample_rate: f32,
        midi_events: &[MidiNoteEvent]
    ) {
        let frames = output_buffer.len() / 2;
        
        let clap_events: Vec<clap_event_note> = midi_events.iter().map(|ev| {
            let event_type = if ev.velocity == 0 { CLAP_EVENT_NOTE_OFF } else { CLAP_EVENT_NOTE_ON };
            clap_event_note {
                header: clap_event_header {
                    size: std::mem::size_of::<clap_event_note>() as u32,
                    time: ev.sample_offset,
                    space_id: CLAP_CORE_EVENT_SPACE_ID,
                    type_: event_type,
                    flags: 0,
                },
                note_id: -1,
                port_index: 0,
                channel: ev.channel as i16,
                key: ev.note as i16,
                velocity: (ev.velocity as f64 / 127.0),
            }
        }).collect();

        let mut clap_params = Vec::new();
        if let Ok(mut pending) = self.pending_params.lock() {
            let pending_list: Vec<(u32, f64)> = pending.drain(..).collect();
            for (id, val) in pending_list {
                clap_params.push(clap_event_param_value {
                    header: clap_event_header {
                        size: std::mem::size_of::<clap_event_param_value>() as u32,
                        time: 0, // Instant
                        space_id: CLAP_CORE_EVENT_SPACE_ID,
                        type_: CLAP_EVENT_PARAM_VALUE,
                        flags: 0,
                    },
                    param_id: id,
                    cookie: ptr::null_mut(),
                    note_id: -1,
                    port_index: 0,
                    channel: -1,
                    key: -1,
                    value: val,
                });
            }
        }

        let input_ctx = EventListContext { events: clap_events, param_events: clap_params };
        let input_events = clap_input_events {
            ctx: &input_ctx as *const _ as *mut c_void,
            size: Some(input_events_size),
            get: Some(input_events_get),
        };

        let output_ctx = EventListContext { events: Vec::new(), param_events: Vec::new() };
        let output_events = clap_output_events {
            ctx: &output_ctx as *const _ as *mut c_void,
            try_push: Some(output_events_try_push),
        };

        // Create temporary planar buffers
        let mut left_buf = vec![0.0; frames];
        let mut right_buf = vec![0.0; frames];

        let mut output_channel_pointers = [
            left_buf.as_mut_ptr(),
            right_buf.as_mut_ptr()
        ];

        let mut output_buffers = clap_audio_buffer {
            data32: output_channel_pointers.as_mut_ptr(),
            data64: ptr::null_mut(),
            channel_count: 2,
            latency: 0,
            constant_mask: 0,
        };

        // Create process structure
        let process = clap_process {
            steady_time: -1, // Unknown
            frames_count: frames as u32,
            transport: ptr::null(), // No transport info for now
            audio_inputs: ptr::null(), // Synth - no inputs
            audio_outputs: &mut output_buffers,
            audio_inputs_count: 0,
            audio_outputs_count: 1,
            in_events: &input_events,
            out_events: &output_events,
        };


        // Call plugin process
        if let Some(process_fn) = (*self.plugin).process {
            process_fn(self.plugin, &process);
        }

        // Interleave back to output_buffer (LRLR...)
        for i in 0..frames {
            output_buffer[i * 2] = left_buf[i];
            output_buffer[i * 2 + 1] = right_buf[i];
        }
    }

    /// Set a parameter value (queued for next process call)
    pub fn set_parameter(&self, param_id: u32, value: f64) {
        if let Ok(mut pending) = self.pending_params.lock() {
            pending.push((param_id, value));
        }
    }

    pub unsafe fn get_param_count(&self) -> u32 {
        if !self.params.is_null() {
            if let Some(count) = (*self.params).count {
                return count(self.plugin);
            }
        }
        0
    }

    pub unsafe fn get_param_info(&self, index: u32) -> Option<clap_param_info> {
        if !self.params.is_null() {
            if let Some(get_info) = (*self.params).get_info {
                let mut info = std::mem::zeroed::<clap_param_info>();
                if get_info(self.plugin, index, &mut info) {
                    return Some(info);
                }
            }
        }
        None
    }

    pub unsafe fn attach_to_window(&self, window: &Window) -> Result<()> {
        let gui_ext = if let Some(get_ext) = (*self.plugin).get_extension {
            get_ext(self.plugin, CLAP_EXT_GUI.as_ptr() as *const i8) as *const clap_plugin_gui
        } else {
            ptr::null()
        };

        if gui_ext.is_null() {
            return Err(anyhow!("Plugin does not support GUI extension"));
        }

        // Check availability
        // We assume X11 for linux main target
        let api = CLAP_WINDOW_API_X11;
        let is_api_supported = (*gui_ext).is_api_supported.ok_or(anyhow!("is_api_supported not defined"))?;
        
        if !is_api_supported(self.plugin, api.as_ptr(), false) {
             return Err(anyhow!("Plugin does not support X11 Window API"));
        }

        // Create
        let create = (*gui_ext).create.ok_or(anyhow!("gui.create not defined"))?;
        if !create(self.plugin, api.as_ptr(), false) {
             return Err(anyhow!("gui.create failed"));
        }

        // Get Native Handle
        // raw_window_handle() returns Result<RawWindowHandle, HandleError> in 0.6
        let raw_handle = window.raw_window_handle();
        let window_ptr = match raw_handle {
             Ok(RawWindowHandle::Xlib(handle)) => handle.window as *mut c_void,
             Ok(RawWindowHandle::Xcb(handle)) => handle.window.get() as *mut c_void,
             _ => return Err(anyhow!("Unsupported window handle type (Not X11 or failed to get handle)")),
        };

        // Set Parent
        let set_parent = (*gui_ext).set_parent.ok_or(anyhow!("gui.set_parent not defined"))?;
        let clap_win = clap_window {
             api: api.as_ptr(),
             specific: clap_sys::ext::gui::clap_window_handle {
                 ptr: window_ptr,
             }
        };
        
        if !set_parent(self.plugin, &clap_win) {
             eprintln!("[CLAP] gui.set_parent failed (might be floating?)");
        }

        // Show
        let show = (*gui_ext).show.ok_or(anyhow!("gui.show not defined"))?;
        if !show(self.plugin) {
             return Err(anyhow!("gui.show failed"));
        }

        Ok(())
    }

    pub unsafe fn destroy_editor(&self) {
        let gui_ext = if let Some(get_ext) = (*self.plugin).get_extension {
            get_ext(self.plugin, CLAP_EXT_GUI.as_ptr() as *const i8) as *const clap_plugin_gui
        } else {
            ptr::null()
        };

        if !gui_ext.is_null() {
            // Hide first
            if let Some(hide) = (*gui_ext).hide {
                hide(self.plugin);
            }
            // Then destroy
            if let Some(destroy) = (*gui_ext).destroy {
                destroy(self.plugin);
            }
        }
    }
}
