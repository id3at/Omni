use std::sync::Arc;
use libloading::{Library, Symbol};
use anyhow::{Result, anyhow};
use std::ffi::c_void;

use crate::vst3_defs::*;

// Function signature for loading the factory
type GetPluginFactory = unsafe extern "C" fn() -> *mut c_void;

pub struct Vst3Plugin {
    _library: Arc<Library>, 
    pub component: *mut IComponent,
    pub processor: *mut IAudioProcessor,
    pub factory: *mut IPluginFactory,
}


unsafe impl Send for Vst3Plugin {}

impl Vst3Plugin {
    pub unsafe fn load(path: &str) -> Result<Self> {
        let lib = Library::new(path)?;
        let library = Arc::new(lib);

        // 0. InitDll / ModuleEntry
        // Try to find ModuleEntry (Linux VST3 standard for initialization)
        // We use libc::dlopen to get the handle because libloading doesn't expose it easily.
        let c_path = std::ffi::CString::new(path)?;
        let handle = libc::dlopen(c_path.as_ptr(), libc::RTLD_LAZY | libc::RTLD_LOCAL);
        
        if !handle.is_null() {
             // We can use libloading to find the symbol, but we need to pass 'handle' to it.
             if let Ok(init_fn) = library.get::<ModuleEntry>(b"ModuleEntry") {
                 let res = init_fn(handle);
                 if !res {
                     eprintln!("[Plugin] ModuleEntry returned false");
                     return Err(anyhow!("ModuleEntry returned false"));
                 } else {
                     eprintln!("[Plugin] ModuleEntry called with handle {:p}, success.", handle);
                 }
             }
        } else {
            eprintln!("[Plugin] dlopen failed to get handle for ModuleEntry, using null (fallback).");
             if let Ok(init_fn) = library.get::<ModuleEntry>(b"ModuleEntry") {
                 let res = init_fn(std::ptr::null_mut());
                 if !res {
                     return Err(anyhow!("ModuleEntry returned false"));
                 }
             }
        }

        // 1. Get Factory
        let factory_fn: Symbol<GetPluginFactory> = library.get(b"GetPluginFactory")?;
        let factory_ptr = factory_fn();
        
        if factory_ptr.is_null() {
            return Err(anyhow!("GetPluginFactory returned null"));
        }

        let factory = factory_ptr as *mut IPluginFactory;
        
        // 2. Count Classes
        let count = ((*(*factory).vtable).count_classes)(factory as *mut c_void);
        if count == 0 {
             return Err(anyhow!("No classes in factory"));
        }

        // 3. Get Class Info (Index 0)
        let mut class_info: PClassInfo = std::mem::zeroed();
        ((*(*factory).vtable).get_class_info)(factory as *mut c_void, 0, &mut class_info);

        let class_name = std::ffi::CStr::from_ptr(class_info.name.as_ptr()).to_string_lossy();
        let class_category = std::ffi::CStr::from_ptr(class_info.category.as_ptr()).to_string_lossy();
        
        eprintln!("[Plugin] Class Name: {}, Category: {}", class_name, class_category);
        
        // 4. Create Instance (Component)
        let mut obj_ptr: *mut c_void = std::ptr::null_mut();
        let iid = I_COMPONENT_IID;
        
        let mut res = ((*(*factory).vtable).create_instance)(
            factory as *mut c_void, 
            class_info.cid.as_ptr() as *const TUID, 
            iid.as_ptr() as *const TUID, 
            &mut obj_ptr
        );
        
        let mut component: *mut IComponent = std::ptr::null_mut();
        let mut processor: *mut IAudioProcessor = std::ptr::null_mut();

        if res == 0 && !obj_ptr.is_null() {
            component = obj_ptr as *mut IComponent;
        } else {
             eprintln!("[Plugin] Failed to create IComponent (res: 0x{:x}). Trying FUnknown...", res);
             
             // Fallback: Try FUnknown
             // IID_FUnknown: 00000000-0000-0000-C000-000000000046
             let unknown_iid = [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46];
             res = ((*(*factory).vtable).create_instance)(
                factory as *mut c_void, 
                class_info.cid.as_ptr() as *const TUID, 
                unknown_iid.as_ptr() as *const TUID, 
                &mut obj_ptr
             );
             
             if res == 0 && !obj_ptr.is_null() {
                 let unknown = obj_ptr as *mut FUnknown;
                 let mut component_ptr: *mut c_void = std::ptr::null_mut();
                 
                 // Query IComponent
                 let query_res = ((*(*unknown).vtable).query_interface)(
                    unknown as *mut c_void,
                    iid.as_ptr() as *const TUID,
                    &mut component_ptr
                 );
                 
                 if query_res == 0 && !component_ptr.is_null() {
                     component = component_ptr as *mut IComponent;
                 } else {
                     eprintln!("[Plugin] Failed to query IComponent from FUnknown (res: {:x})", query_res);
                 }
                 ((*(*unknown).vtable).release)(unknown as *mut c_void);
             } else {
                 // Fallback 2: Try IAudioProcessor directly
                 eprintln!("[Plugin] Try IAudioProcessor directly...");
                 let proc_iid = I_AUDIO_PROCESSOR_IID;
                 res = ((*(*factory).vtable).create_instance)(
                    factory as *mut c_void, 
                    class_info.cid.as_ptr() as *const TUID, 
                    proc_iid.as_ptr() as *const TUID, 
                    &mut obj_ptr
                 );
                 
                 if res == 0 && !obj_ptr.is_null() {
                     processor = obj_ptr as *mut IAudioProcessor;
                     // Query IComponent from Processor
                     let mut component_ptr: *mut c_void = std::ptr::null_mut();
                     let comp_iid = I_COMPONENT_IID;
                     let query_res = ((*(*processor).vtable).parent.query_interface)(
                        processor as *mut c_void,
                        comp_iid.as_ptr() as *const TUID,
                        &mut component_ptr
                     );
                     if query_res == 0 && !component_ptr.is_null() {
                         component = component_ptr as *mut IComponent;
                     }
                 }
             }
        }
        
        if component.is_null() {
             return Err(anyhow!("Failed to acquire IComponent interface. Last result: 0x{:x}, CID: {:02x?}", res, class_info.cid));
        }

        // 5. Initialize
        let res = ((*(*component).vtable).initialize)(component as *mut c_void, std::ptr::null_mut());
        if res != 0 {
             return Err(anyhow!("Failed to initialize component (result: 0x{:x})", res));
        }

        // 6. Query Interface (AudioProcessor) if not already found
        if processor.is_null() {
            let mut processor_ptr: *mut c_void = std::ptr::null_mut();
            let proc_iid = I_AUDIO_PROCESSOR_IID;
            let res = ((*(*component).vtable).parent.query_interface)(
                component as *mut c_void, 
                proc_iid.as_ptr() as *const TUID, 
                &mut processor_ptr
            );

            if res != 0 || processor_ptr.is_null() {
                return Err(anyhow!("Failed to query IAudioProcessor"));
            }
            processor = processor_ptr as *mut IAudioProcessor;
        }
        
        // 7. Activate
        ((*(*component).vtable).set_active)(component as *mut c_void, 1); // 1 = true
        ((*(*processor).vtable).set_processing)(processor as *mut c_void, 1);

        Ok(Self {
            _library: library,
            component,
            processor,
            factory,
        })
    }
    
    pub unsafe fn process(&mut self, buffer: &mut [f32]) {
         let samples = buffer.len() as i32;
         
         let mut input_channel_ptrs: [*mut f32; 1] = [buffer.as_mut_ptr()];
         let mut output_channel_ptrs: [*mut f32; 1] = [buffer.as_mut_ptr()];

         let mut inputs = AudioBusBuffers {
             num_channels: 1,
             silence_flags: 0,
             channel_buffers: input_channel_ptrs.as_mut_ptr() as *mut *mut f32, 
         };

         let mut outputs = AudioBusBuffers {
             num_channels: 1,
             silence_flags: 0,
             channel_buffers: output_channel_ptrs.as_mut_ptr() as *mut *mut f32,
         };

         let mut process_data: ProcessData = std::mem::zeroed();
         process_data.process_mode = PROCESS_MODE_REALTIME;
         process_data.symbolic_sample_size = SAMPLE_32;
         process_data.num_samples = samples;
         process_data.num_inputs = 1;
         process_data.num_outputs = 1;
         process_data.inputs = &mut inputs;
         process_data.outputs = &mut outputs;
         
         ((*(*self.processor).vtable).process)(self.processor as *mut c_void, &mut process_data);
    }
}
