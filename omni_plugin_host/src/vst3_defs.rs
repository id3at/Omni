use std::ffi::c_void;
use std::os::raw::c_char;

pub type TUID = [u8; 16];
pub type VstPtr<T> = *mut T;
pub type ModuleEntry = unsafe extern "C" fn(handle: *mut c_void) -> bool;

#[repr(C)]
pub struct FUnknown {
    pub vtable: *const FUnknownVTable,
}

#[repr(C)]
pub struct FUnknownVTable {
    pub query_interface: unsafe extern "system" fn(this: *mut c_void, iid: *const TUID, obj: *mut *mut c_void) -> i32,
    pub add_ref: unsafe extern "system" fn(this: *mut c_void) -> u32,
    pub release: unsafe extern "system" fn(this: *mut c_void) -> u32,
}

// IPluginFactory
pub const I_PLUGIN_FACTORY_IID: TUID = [
    0x7A, 0x43, 0x81, 0x98, 0x72, 0xEE, 0x4C, 0xE6, 
    0xA3, 0x99, 0xC1, 0xA4, 0x88, 0xEE, 0x50, 0x07
];

#[repr(C)]
pub struct IPluginFactory {
    pub vtable: *const IPluginFactoryVTable,
}

#[repr(C)]
pub struct PClassInfo {
    pub cid: TUID,
    pub cardinality: i32,
    pub category: [c_char; 32],
    pub name: [c_char; 64],
}

#[repr(C)]
pub struct IPluginFactoryVTable {
    pub parent: FUnknownVTable,
    pub get_factory_info: unsafe extern "system" fn(this: *mut c_void, info: *mut c_void) -> i32, // Simplified
    pub count_classes: unsafe extern "system" fn(this: *mut c_void) -> i32,
    pub get_class_info: unsafe extern "system" fn(this: *mut c_void, index: i32, info: *mut PClassInfo) -> i32,
    pub create_instance: unsafe extern "system" fn(this: *mut c_void, cid: *const TUID, iid: *const TUID, obj: *mut *mut c_void) -> i32,
}

// IComponent
pub const I_COMPONENT_IID: TUID = [
    0xE8, 0x31, 0xFF, 0x31, 0xF2, 0xD5, 0x43, 0x03, 
    0x92, 0x8E, 0x3B, 0x9B, 0x56, 0xF3, 0xD7, 0x93
];

#[repr(C)]
pub struct IComponent {
    pub vtable: *const IComponentVTable,
}

#[repr(C)]
pub struct IComponentVTable {
    pub parent: FUnknownVTable,
    // IPluginBase
    pub initialize: unsafe extern "system" fn(this: *mut c_void, context: *mut c_void) -> i32,
    pub terminate: unsafe extern "system" fn(this: *mut c_void) -> i32,
    // IComponent
    pub get_controller_class_id: unsafe extern "system" fn(this: *mut c_void, cid: *mut TUID) -> i32,
    pub set_io_mode: unsafe extern "system" fn(this: *mut c_void, mode: i32) -> i32,
    pub get_bus_count: unsafe extern "system" fn(this: *mut c_void, media_type: i32, dir: i32) -> i32,
    pub get_bus_info: unsafe extern "system" fn(this: *mut c_void, media_type: i32, dir: i32, index: i32, info: *mut c_void) -> i32, 
    pub get_routing_info: unsafe extern "system" fn(this: *mut c_void, in_info: *mut c_void, out_info: *mut c_void) -> i32,
    pub activate_bus: unsafe extern "system" fn(this: *mut c_void, media_type: i32, dir: i32, index: i32, state: u8) -> i32,
    pub set_active: unsafe extern "system" fn(this: *mut c_void, state: u8) -> i32,
    pub set_state: unsafe extern "system" fn(this: *mut c_void, state_stream: *mut c_void) -> i32,
    pub get_state: unsafe extern "system" fn(this: *mut c_void, state_stream: *mut c_void) -> i32,
}

// IAudioProcessor
pub const I_AUDIO_PROCESSOR_IID: TUID = [
    0x42, 0x04, 0x3F, 0x99, 0xB5, 0x80, 0x45, 0x64, 
    0xA0, 0x4E, 0x17, 0xF8, 0x5C, 0xE3, 0x8D, 0x47
];

#[repr(C)]
pub struct IAudioProcessor {
    pub vtable: *const IAudioProcessorVTable,
}

#[repr(C)]
pub struct IAudioProcessorVTable {
    pub parent: FUnknownVTable,
    pub set_bus_arrangements: unsafe extern "system" fn(this: *mut c_void, inputs: *mut c_void, num_ins: i32, outputs: *mut c_void, num_outs: i32) -> i32,
    pub get_bus_arrangement: unsafe extern "system" fn(this: *mut c_void, dir: i32, index: i32, arr: *mut c_void) -> i32,
    pub can_process_sample_realtime: unsafe extern "system" fn(this: *mut c_void) -> i32,
    pub set_processing: unsafe extern "system" fn(this: *mut c_void, state: u8) -> i32,
    pub process: unsafe extern "system" fn(this: *mut c_void, data: *mut ProcessData) -> i32,
    pub get_tail_samples: unsafe extern "system" fn(this: *mut c_void) -> u32,
}

#[repr(C)]
pub struct AudioBusBuffers {
    pub num_channels: i32,
    pub silence_flags: u64,
    pub channel_buffers: *mut *mut f32, // array of channel pointers
}

#[repr(C)]
pub struct ProcessData {
    pub process_mode: i32,
    pub symbolic_sample_size: i32,
    pub num_samples: i32,
    pub num_inputs: i32,
    pub num_outputs: i32,
    pub inputs: *mut AudioBusBuffers,
    pub outputs: *mut AudioBusBuffers,
    pub param_changes: *mut c_void,
    pub event_changes: *mut c_void,
}

pub const PROCESS_MODE_REALTIME: i32 = 1;
pub const PROCESS_MODE_OFFLINE: i32 = 2; // Prefetch
pub const SAMPLE_32: i32 = 0;
pub const SAMPLE_64: i32 = 1;
