#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(all(target_os = "linux", target_env = "gnu"))]
mod mallinfo;

#[tokio::main]
#[tauri_runtime_cef::cef_entry_point]
async fn main() {
    if let Err(error) = koharu::run(tauri::generate_context!()).await {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}
