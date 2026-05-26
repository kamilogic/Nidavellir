pub mod detector;
pub mod tuner;
pub mod stress;
pub mod optimizer;
pub mod profile;
pub mod monitor;
pub mod service;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![])
        .run(tauri::generate_context!())
        .expect("error while running nidavellir");
}
