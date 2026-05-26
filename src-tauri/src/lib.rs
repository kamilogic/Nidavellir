pub mod detector;
pub mod tuner;
pub mod stress;
pub mod optimizer;
pub mod profile;
pub mod monitor;
pub mod service;

use std::sync::Mutex;

#[tauri::command]
fn detect_hardware() -> detector::HardwareInfo {
    detector::detect_all()
}

#[tauri::command]
fn read_sensors(state: tauri::State<'_, MonitorState>) -> monitor::SensorReadings {
    state.0.lock().unwrap().read_sensors()
}

struct MonitorState(Mutex<monitor::Monitor>);

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(MonitorState(Mutex::new(monitor::Monitor::new())))
        .invoke_handler(tauri::generate_handler![detect_hardware, read_sensors])
        .run(tauri::generate_context!())
        .expect("error while running nidavellir");
}
