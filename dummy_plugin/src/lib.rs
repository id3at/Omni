use clap_sys::entry::clap_plugin_entry;
use std::ffi::CStr;
use std::os::raw::c_char;
use std::ptr;

#[no_mangle]
pub unsafe extern "C" fn clap_entry(_path: *const c_char) -> *const clap_plugin_entry {
    eprintln!("[DUMMY] clap_entry called!");
    &ENTRY
}

static ENTRY: clap_plugin_entry = clap_plugin_entry {
    clap_version: clap_sys::version::CLAP_VERSION,
    init: Some(init),
    deinit: Some(deinit),
    get_factory: Some(get_factory),
};

unsafe extern "C" fn init(_plugin_path: *const c_char) -> bool {
    eprintln!("[DUMMY] init called!");
    true
}

unsafe extern "C" fn deinit() {
    eprintln!("[DUMMY] deinit called!");
}

unsafe extern "C" fn get_factory(_factory_id: *const c_char) -> *const c_void {
    eprintln!("[DUMMY] get_factory called!");
    ptr::null() // Just return null for test
}

use std::os::raw::c_void;
