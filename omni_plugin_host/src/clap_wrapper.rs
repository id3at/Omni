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
    clap_event_transport, CLAP_EVENT_TRANSPORT,
    CLAP_TRANSPORT_HAS_TEMPO, CLAP_TRANSPORT_HAS_BEATS_TIMELINE,
    CLAP_TRANSPORT_HAS_TIME_SIGNATURE, CLAP_TRANSPORT_IS_PLAYING,
    clap_event_note_expression, CLAP_EVENT_NOTE_EXPRESSION, CLAP_NOTE_EXPRESSION_TUNING,
};
use clap_sys::fixedpoint::CLAP_BEATTIME_FACTOR;
use clap_sys::ext::params::{clap_plugin_params, CLAP_EXT_PARAMS, clap_param_info};
use clap_sys::ext::gui::{clap_plugin_gui, CLAP_EXT_GUI, clap_window, CLAP_WINDOW_API_X11};
use clap_sys::ext::timer_support::{clap_plugin_timer_support, clap_host_timer_support, CLAP_EXT_TIMER_SUPPORT};
use clap_sys::ext::note_name::{clap_plugin_note_name, clap_note_name, CLAP_EXT_NOTE_NAME};
use clap_sys::ext::latency::{clap_plugin_latency, CLAP_EXT_LATENCY};
use winit::window::Window;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use libloading::os::unix::Library as UnixLibrary;
use libloading::{Library, Symbol};
use std::ptr;
use std::ffi::{CString, CStr, c_void};
use std::sync::{Arc, Mutex};
use std::os::raw::c_char;
use anyhow::{Result, anyhow};
use std::collections::HashMap;
// use lazy_static::lazy_static; // Need lazy_static or OnceLock. Actually simple Mutex works if global.
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

static NEXT_TIMER_ID: AtomicU32 = AtomicU32::new(1);
// Timer ID -> (Period Ms, Next Run Time)
type TimerMap = HashMap<u32, (u32, std::time::Instant)>;

lazy_static::lazy_static! {
    static ref ACTIVE_TIMERS: Mutex<TimerMap> = Mutex::new(HashMap::new());
}

use omni_shared::MidiNoteEvent;

/// Transport information passed from the audio engine to plugins
#[derive(Clone, Copy, Debug, Default)]
pub struct TransportInfo {
    pub is_playing: bool,
    pub tempo: f64,
    pub song_pos_beats: f64,
    pub bar_start_beats: f64,
    pub bar_number: i32,
    pub time_sig_num: u16,
    pub time_sig_denom: u16,
}

struct AudioBuffers {
    left: Vec<f32>,
    right: Vec<f32>,
    input_events: Vec<clap_event_note>,
    expression_events: Vec<clap_event_note_expression>,
    param_events: Vec<clap_event_param_value>,
}

pub struct ClapPlugin {
    _library: Arc<Library>,
    pub plugin: *const clap_plugin,
    // Boxed host to ensure pointer stability
    _host_box: Box<clap_host>, 
    pub params: *const clap_plugin_params,
    pending_params: Arc<Mutex<Vec<(u32, f64)>>>,

    // Plugin metadata
    pub clap_id: String,

    // Interior Mutability for Audio Thread exclusive access
    // This Mutex is ONLY locked by process_audio, so it is uncontended by GUI
    audio_buffers: Mutex<AudioBuffers>,
    
    // Sample Rate for process
    pub sample_rate: f64,
}

unsafe impl Send for ClapPlugin {}
unsafe impl Sync for ClapPlugin {}

// Host Callbacks

extern "C" fn host_request_restart(_host: *const clap_host) {}
extern "C" fn host_request_process(_host: *const clap_host) {}
extern "C" fn host_request_callback(_host: *const clap_host) {}

// Timer Callbacks
unsafe extern "C" fn host_register_timer(_host: *const clap_host, period_ms: u32, timer_id: *mut u32) -> bool {
    let id = NEXT_TIMER_ID.fetch_add(1, Ordering::Relaxed);
    *timer_id = id;
    eprintln!("[Timer] Registered timer ID: {} with period: {}ms", id, period_ms);
    if let Ok(mut timers) = ACTIVE_TIMERS.lock() {
        timers.insert(id, (period_ms, std::time::Instant::now() + Duration::from_millis(period_ms as u64)));
    }
    true
}

unsafe extern "C" fn host_unregister_timer(_host: *const clap_host, timer_id: u32) -> bool {
    eprintln!("[Timer] Unregistered timer ID: {}", timer_id);
    if let Ok(mut timers) = ACTIVE_TIMERS.lock() {
        timers.remove(&timer_id);
    }
    true
}

static HOST_TIMER_SUPPORT: clap_host_timer_support = clap_host_timer_support {
    register_timer: Some(host_register_timer),
    unregister_timer: Some(host_unregister_timer),
};

extern "C" fn host_get_extension(_host: *const clap_host, extension_id: *const c_char) -> *const c_void {
    unsafe {
        let id_cstr = CStr::from_ptr(extension_id);
        // Compare bytes properly. CLAP headers define macros as C strings (null terminated).
        // CLAP_EXT_TIMER_SUPPORT is b"clap.timer-support\0" in clap-sys.
        if id_cstr == CLAP_EXT_TIMER_SUPPORT {
            eprintln!("[Host] Providing CLAP_EXT_TIMER_SUPPORT");
            return &HOST_TIMER_SUPPORT as *const _ as *const c_void;
        }
    }
    ptr::null()
}


// Event list context for input events
struct EventListContext {
    events: *const Vec<clap_event_note>,
    expression_events: *const Vec<clap_event_note_expression>,
    param_events: *const Vec<clap_event_param_value>,
}

// ...

// Input events callback: get size
unsafe extern "C" fn input_events_size(list: *const clap_input_events) -> u32 {
    let ctx = (*list).ctx as *const EventListContext;
    let events = &*(*ctx).events;
    let exprs = &*(*ctx).expression_events;
    let param_events = &*(*ctx).param_events;
    (events.len() + exprs.len() + param_events.len()) as u32
}

// Input events callback: get event
unsafe extern "C" fn input_events_get(list: *const clap_input_events, index: u32) -> *const clap_event_header {
    let ctx = (*list).ctx as *const EventListContext;
    let events = &*(*ctx).events;
    let exprs = &*(*ctx).expression_events;
    let param_events = &*(*ctx).param_events;
    
    let note_count = events.len();
    let expr_count = exprs.len();
    
    if (index as usize) < note_count {
        &events[index as usize].header as *const clap_event_header
    } else if (index as usize) < (note_count + expr_count) {
        let expr_idx = (index as usize) - note_count;
        &exprs[expr_idx].header as *const clap_event_header
    } else {
        let param_idx = (index as usize) - (note_count + expr_count);
        if param_idx < param_events.len() {
            &param_events[param_idx].header as *const clap_event_header
        } else {
            ptr::null()
        }
    }
}

// ...

// Context for output events (capturing parameter changes)
struct OutputContext {
    header: *mut omni_shared::OmniShmemHeader,
}

// Output events callback: try_push
unsafe extern "C" fn output_events_try_push(list: *const clap_output_events, event: *const clap_event_header) -> bool {
    let ctx = (*list).ctx as *mut OutputContext;
    let header = &mut *(*ctx).header;

    if (*event).type_ == CLAP_EVENT_PARAM_VALUE {
        let param_event = &*(event as *const clap_event_param_value);
        
        // Update Shared Memory Header
        std::ptr::write_volatile(&mut header.last_touched_param, param_event.param_id);
        std::ptr::write_volatile(&mut header.last_touched_value, param_event.value as f32);
        
        let gen = std::ptr::read_volatile(&header.touch_generation);
        std::ptr::write_volatile(&mut header.touch_generation, gen.wrapping_add(1));
    }

    true 
}

impl ClapPlugin {
    pub unsafe fn load(path: &str, sample_rate: f64) -> Result<Self> {
        let lib_unix = UnixLibrary::open(Some(path), libc::RTLD_NOW | libc::RTLD_LOCAL)?;
        let library = Arc::new(Library::from(lib_unix));
        
        let entry_ptr: Symbol<*const clap_plugin_entry> = 
            library.get(b"clap_entry")?;

        let entry = *entry_ptr;
        
        if entry.is_null() {
            return Err(anyhow!("clap_entry is null"));
        }

        let lib_path_c = CString::new(path)?;
        if let Some(init) = (*entry).init {
             if !init(lib_path_c.as_ptr()) {
                 return Err(anyhow!("entry.init failed"));
             }
        } else {
             return Err(anyhow!("entry.init not defined"));
        }

        let get_factory = (*entry).get_factory.ok_or(anyhow!("get_factory not defined"))?;
        let factory_id = b"clap.plugin-factory\0";
        let factory_ptr = get_factory(factory_id.as_ptr() as *const c_char);
        if factory_ptr.is_null() {
             return Err(anyhow!("Failed to get clap.plugin-factory"));
        }
        let factory = factory_ptr as *const clap_plugin_factory;

        let get_plugin_count = (*factory).get_plugin_count.ok_or(anyhow!("get_plugin_count not defined"))?;
        let count = get_plugin_count(factory);
        if count == 0 {
            return Err(anyhow!("No plugins found in factory"));
        }
        
        let get_plugin_descriptor = (*factory).get_plugin_descriptor.ok_or(anyhow!("get_plugin_descriptor not defined"))?;

        let desc = get_plugin_descriptor(factory, 0); 
        let plugin_id = (*desc).id;
        
        let name = CStr::from_ptr((*desc).name).to_string_lossy();
        eprintln!("[CLAP] Loading: {}", name);

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

        let host_ptr = &*host as *const clap_host;
        let create_plugin = (*factory).create_plugin.ok_or(anyhow!("create_plugin not defined"))?;
        
        let plugin = create_plugin(factory, host_ptr, plugin_id);
        if plugin.is_null() {
            return Err(anyhow!("create_plugin failed"));
        }

        if let Some(init) = (*plugin).init {
             if !init(plugin) {
                 return Err(anyhow!("plugin.init failed"));
             }
        }

        eprintln!("[CLAP] Loaded successfully.");

        let params = if let Some(get_ext) = (*plugin).get_extension {
            get_ext(plugin, CLAP_EXT_PARAMS.as_ptr()) as *const clap_plugin_params
        } else {
            ptr::null()
        };

        if let Some(activate) = (*plugin).activate {
             if !activate(plugin, sample_rate, 32, 4096) {
                 eprintln!("[CLAP] Warning: activate failed");
             }
        }

        if let Some(start_processing) = (*plugin).start_processing {
             if !start_processing(plugin) {
                 eprintln!("[CLAP] Warning: start_processing failed");
             }
        }

        let max_buf = 4096;

        Ok(Self {
            _library: library,
            plugin,
            _host_box: host,
            params,
            pending_params: Arc::new(Mutex::new(Vec::new())),
            clap_id: CStr::from_ptr(plugin_id).to_string_lossy().into_owned(),
            audio_buffers: Mutex::new(AudioBuffers {
                left: vec![0.0; max_buf],
                right: vec![0.0; max_buf],
                input_events: Vec::with_capacity(128),
                expression_events: Vec::with_capacity(128),
                param_events: Vec::with_capacity(32),
            }),
            sample_rate,
        })
    }

    pub unsafe fn process_audio(
        &self, 
        output_buffer: &mut [f32], 
        midi_events: &[MidiNoteEvent],
        param_events: &[omni_shared::ParameterEvent],
        expression_events: &[omni_shared::ExpressionEvent],
        shmem_header: &mut omni_shared::OmniShmemHeader,
        transport: &TransportInfo,
    ) {
        let mut bufs = self.audio_buffers.lock().unwrap();
        let AudioBuffers { left, right, input_events: clap_input_events, expression_events: clap_expr_events, param_events: clap_param_events } = &mut *bufs;
        
        let frames = output_buffer.len() / 2;
        
        if left.len() < frames {
            left.resize(frames, 0.0);
            right.resize(frames, 0.0);
        }
        
        clap_input_events.clear();
        clap_expr_events.clear();
        
        for ev in midi_events {
             let event_type = if ev.velocity == 0 { CLAP_EVENT_NOTE_OFF } else { CLAP_EVENT_NOTE_ON };
             clap_input_events.push(clap_event_note {
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
            });

            // Handle Detune
            if ev.velocity > 0 && ev.detune.abs() > 0.001 {
                clap_expr_events.push(clap_event_note_expression {
                    header: clap_event_header {
                         size: std::mem::size_of::<clap_event_note_expression>() as u32,
                         time: ev.sample_offset,
                         space_id: CLAP_CORE_EVENT_SPACE_ID,
                         type_: CLAP_EVENT_NOTE_EXPRESSION,
                         flags: 0,
                    },
                    note_id: -1,
                    port_index: 0,
                    channel: ev.channel as i16,
                    key: ev.note as i16,
                    expression_id: CLAP_NOTE_EXPRESSION_TUNING,
                    value: ev.detune as f64,
                });
            }
        }

        // Handle explicit Expression Events
        if !expression_events.is_empty() {
            eprintln!("[CLAP DEBUG] Processing {} expression events", expression_events.len());
        }
        for ev in expression_events {
             clap_expr_events.push(clap_event_note_expression {
                header: clap_event_header {
                     size: std::mem::size_of::<clap_event_note_expression>() as u32,
                     time: ev.sample_offset,
                     space_id: CLAP_CORE_EVENT_SPACE_ID,
                     type_: CLAP_EVENT_NOTE_EXPRESSION,
                     flags: 0,
                },
                note_id: -1,
                port_index: 0,
                channel: ev.channel as i16,
                key: ev.key as i16,
                expression_id: ev.expression_id as i32,
                value: ev.value,
            });
        }
        
        clap_param_events.clear();
        if let Ok(mut pending) = self.pending_params.lock() {
            if !pending.is_empty() {
                for (id, val) in pending.drain(..) {
                    clap_param_events.push(clap_event_param_value {
                        header: clap_event_header {
                            size: std::mem::size_of::<clap_event_param_value>() as u32,
                            time: 0,
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
        }

        for p_ev in param_events {
             clap_param_events.push(clap_event_param_value {
                header: clap_event_header {
                    size: std::mem::size_of::<clap_event_param_value>() as u32,
                    time: p_ev.sample_offset,
                    space_id: CLAP_CORE_EVENT_SPACE_ID,
                    type_: CLAP_EVENT_PARAM_VALUE,
                    flags: 0,
                },
                param_id: p_ev.param_id,
                cookie: ptr::null_mut(),
                note_id: -1,
                port_index: 0,
                channel: -1,
                key: -1,
                value: p_ev.value,
            });
        }

        let input_ctx = EventListContext { 
            events: clap_input_events as *const _, 
            expression_events: clap_expr_events as *const _,
            param_events: clap_param_events as *const _
        };
        
        let input_events = clap_input_events {
            ctx: &input_ctx as *const _ as *mut c_void,
            size: Some(input_events_size),
            get: Some(input_events_get),
        };
        
        let mut output_ctx_struct = OutputContext {
            header: shmem_header as *mut _,
        };

        let output_events = clap_output_events {
            ctx: &mut output_ctx_struct as *mut _ as *mut c_void,
            try_push: Some(output_events_try_push),
        };

        let mut output_channel_pointers = [
            left.as_mut_ptr(),
            right.as_mut_ptr()
        ];
        
        let mut audio_outputs = clap_audio_buffer {
            data32: output_channel_pointers.as_mut_ptr(),
            data64: ptr::null_mut(),
            channel_count: 2,
            latency: 0,
            constant_mask: 0,
        };

        let mut transport_flags: u32 = CLAP_TRANSPORT_HAS_TEMPO 
            | CLAP_TRANSPORT_HAS_BEATS_TIMELINE 
            | CLAP_TRANSPORT_HAS_TIME_SIGNATURE;
        
        if transport.is_playing {
            transport_flags |= CLAP_TRANSPORT_IS_PLAYING;
        }

        let song_pos_beats_fixed = (transport.song_pos_beats * CLAP_BEATTIME_FACTOR as f64) as i64;
        let bar_start_fixed = (transport.bar_start_beats * CLAP_BEATTIME_FACTOR as f64) as i64;

        let transport_event = clap_event_transport {
            header: clap_event_header {
                size: std::mem::size_of::<clap_event_transport>() as u32,
                time: 0,
                space_id: CLAP_CORE_EVENT_SPACE_ID,
                type_: CLAP_EVENT_TRANSPORT,
                flags: 0,
            },
            flags: transport_flags,
            song_pos_beats: song_pos_beats_fixed,
            song_pos_seconds: 0,
            tempo: transport.tempo,
            tempo_inc: 0.0,
            loop_start_beats: 0,
            loop_end_beats: 0,
            loop_start_seconds: 0,
            loop_end_seconds: 0,
            bar_start: bar_start_fixed,
            bar_number: transport.bar_number,
            tsig_num: transport.time_sig_num,
            tsig_denom: transport.time_sig_denom,
        };

        let process = clap_process {
            steady_time: -1, 
            frames_count: frames as u32,
            transport: &transport_event,
            audio_inputs: ptr::null(), 
            audio_outputs: &mut audio_outputs,
            audio_inputs_count: 0,
            audio_outputs_count: 1,
            in_events: &input_events,
            out_events: &output_events,
        };

        if let Some(process_fn) = (*self.plugin).process {
            process_fn(self.plugin, &process);
        }

        for i in 0..frames {
            let l = left.get_unchecked(i); 
            let r = right.get_unchecked(i);
            output_buffer[i * 2] = *l;
            output_buffer[i * 2 + 1] = *r;
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
        // Get Native Handle
        let raw_handle = window.window_handle()?.as_raw();
        let window_ptr = match raw_handle {
             RawWindowHandle::Xlib(handle) => handle.window as *mut c_void,
             RawWindowHandle::Xcb(handle) => handle.window.get() as *mut c_void,
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
        eprintln!("[CLAP] gui.show called successfully");

        Ok(())
    }

    pub unsafe fn destroy_editor(&self) {
        let gui_ext = if let Some(get_ext) = (*self.plugin).get_extension {
            get_ext(self.plugin, CLAP_EXT_GUI.as_ptr() as *const i8) as *const clap_plugin_gui
        } else {
            ptr::null()
        };

        if !gui_ext.is_null() {
            eprintln!("[CLAP] Calling gui.hide...");
            // Hide first
            if let Some(hide) = (*gui_ext).hide {
                hide(self.plugin);
            }
            eprintln!("[CLAP] Calling gui.destroy...");
            // Then destroy
            if let Some(destroy) = (*gui_ext).destroy {
                destroy(self.plugin);
            }
            eprintln!("[CLAP] gui.destroy finished.");
        }
    }

    pub unsafe fn check_timers(&self) {
        let now = std::time::Instant::now();
        let mut timers_to_fire = Vec::new();

        if let Ok(mut timers) = ACTIVE_TIMERS.lock() {
            for (id, (period, next_run)) in timers.iter_mut() {
                if now >= *next_run {
                    timers_to_fire.push(*id);
                    *next_run = now + Duration::from_millis(*period as u64);
                }
            }
        }

        if !timers_to_fire.is_empty() {
             if let Some(get_ext) = (*self.plugin).get_extension {
                let timer_ext = get_ext(self.plugin, CLAP_EXT_TIMER_SUPPORT.as_ptr() as *const i8) as *const clap_plugin_timer_support;
                if !timer_ext.is_null() {
                    if let Some(on_timer) = (*timer_ext).on_timer {
                        for id in timers_to_fire {
                            on_timer(self.plugin, id);
                        }
                    }
                }
            }
        }
    }

    pub unsafe fn get_note_names(&self) -> Vec<omni_shared::NoteNameInfo> {
        let note_name_ext = if let Some(get_ext) = (*self.plugin).get_extension {
            get_ext(self.plugin, CLAP_EXT_NOTE_NAME.as_ptr() as *const i8) as *const clap_plugin_note_name
        } else {
            return Vec::new();
        };

        if note_name_ext.is_null() {
            eprintln!("[CLAP] Plugin does not support note_name extension");
            return Vec::new();
        }

        let mut result = Vec::new();

        if let Some(count_fn) = (*note_name_ext).count {
            let count = count_fn(self.plugin);
            eprintln!("[CLAP] note_name extension: {} names available", count);

            if let Some(get_fn) = (*note_name_ext).get {
                for i in 0..count {
                    let mut info: clap_note_name = std::mem::zeroed();
                    if get_fn(self.plugin, i, &mut info) {
                        let name = std::ffi::CStr::from_ptr(info.name.as_ptr())
                            .to_string_lossy()
                            .into_owned();
                        result.push(omni_shared::NoteNameInfo {
                            key: info.key,
                            channel: info.channel,
                            name,
                        });
                    }
                }
            }
        }

        eprintln!("[CLAP] Retrieved {} note names", result.len());
        result
    }

    /// Get latency in samples
    pub unsafe fn get_latency(&self) -> u32 {
        let latency_ext = if let Some(get_ext) = (*self.plugin).get_extension {
            get_ext(self.plugin, CLAP_EXT_LATENCY.as_ptr() as *const i8) as *const clap_plugin_latency
        } else {
            ptr::null()
        };

        if !latency_ext.is_null() {
             if let Some(get) = (*latency_ext).get {
                 return get(self.plugin);
             }
        }
        0
    }
}

