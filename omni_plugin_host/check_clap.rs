fn main() {
    println!("Checking clap-sys extensions...");
    // This will fail to compile if note_name is missing
    let _ = clap_sys::ext::note_name::CLAP_EXT_NOTE_NAME;
    println!("note_name extension found.");
}
