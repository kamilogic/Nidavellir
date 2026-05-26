pub mod detector;
pub mod tuner;
pub mod stress;
pub mod optimizer;
pub mod profile;
pub mod monitor;
pub mod sweep;
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

#[tauri::command]
fn start_sweep(
    state: tauri::State<'_, SweepState>,
    config: sweep::SweepConfig,
) -> Result<(), String> {
    state.0.lock().unwrap().start(config)
}

#[tauri::command]
fn stop_sweep(state: tauri::State<'_, SweepState>) {
    state.0.lock().unwrap().stop();
}

#[tauri::command]
fn get_sweep_progress(
    state: tauri::State<'_, SweepState>,
) -> Result<sweep::SweepProgress, String> {
    state.0.lock().unwrap().get_progress()
}

#[tauri::command]
fn reset_sweep(state: tauri::State<'_, SweepState>) {
    state.0.lock().unwrap().reset();
}

#[tauri::command]
fn generate_profiles(
    sweep_state: tauri::State<'_, SweepState>,
) -> Result<profile::ProfileSet, String> {
    let progress = sweep_state.0.lock().unwrap().get_progress()?;
    let steps = progress.steps;
    let param = progress.param.ok_or("No sweep data available")?;
    if steps.is_empty() {
        return Err("No sweep steps recorded".into());
    }
    let set = profile::generate_profiles(&steps, &param);
    profile::save_profile_set(&set)?;
    Ok(set)
}

#[tauri::command]
fn get_profiles() -> Result<profile::ProfileSet, String> {
    profile::load_profile_set()
}

#[tauri::command]
fn apply_profile(
    _profile: profile::Profile,
) -> Result<(), String> {
    Err("Kernel driver not yet available".into())
}

struct MonitorState(Mutex<monitor::Monitor>);
struct SweepState(Mutex<sweep::SweepEngine>);

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(MonitorState(Mutex::new(monitor::Monitor::new())))
        .manage(SweepState(Mutex::new(sweep::SweepEngine::new())))
        .invoke_handler(tauri::generate_handler![
            detect_hardware,
            read_sensors,
            start_sweep,
            stop_sweep,
            get_sweep_progress,
            reset_sweep,
            generate_profiles,
            get_profiles,
            apply_profile,
        ])
        .run(tauri::generate_context!())
        .expect("error while running nidavellir");
}
