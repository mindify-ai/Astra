fn main() {
    // `tauri::generate_context!` embeds the icons (the dev Dock icon on macOS comes from icons/icon.icns),
    // but cargo doesn't track them: rebuild when they change so a new icon shows up in `tauri dev`.
    println!("cargo:rerun-if-changed=icons");
    tauri_build::build()
}
