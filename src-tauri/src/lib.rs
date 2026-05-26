pub mod detector;
pub mod tuner;
pub mod stress;
pub mod optimizer;
pub mod profile;
pub mod monitor;
pub mod sweep;
pub mod driver;
pub mod service;

use std::sync::Mutex;

#[tauri::command]
fn detect_hardware() -> detector::HardwareInfo {
    detector::detect_all()
}

#[tauri::command]
fn read_sensors(
    state: tauri::State<'_, MonitorState>,
    driver_state: tauri::State<'_, DriverState>,
) -> monitor::SensorReadings {
    let mut sensors = state.0.lock().unwrap().read_sensors();
    // Inject voltage reading from the shared driver
    if let Ok(dm) = driver_state.0.lock() {
        if dm.is_loaded() {
            if let Ok(msr) = dm.read_msr(driver::IA32_PERF_STATUS) {
                let vid = msr.eax & 0x1FFF;
                if vid > 0 {
                    sensors.cpu.voltage_mv = Some((vid as f64 * 5.0 + 245.0) as u32);
                }
            }
        }
    }
    sensors
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
    profile: profile::Profile,
    driver_state: tauri::State<'_, DriverState>,
) -> Result<String, String> {
    let status = driver_state.0.lock().unwrap().status().clone();
    match status {
        driver::DriverStatus::Loaded => {
            tuner::apply_tuning(&profile.tuning, &driver_state)?;
            Ok("Profile applied via kernel driver".into())
        }
        driver::DriverStatus::NotLoaded | driver::DriverStatus::Failed(_) => {
            Err(format!("Kernel driver not loaded ({:?}). Cannot apply profile.", status))
        }
    }
}

#[tauri::command]
fn get_driver_status(
    driver_state: tauri::State<'_, DriverState>,
) -> Result<String, String> {
    Ok(format!("{:?}", driver_state.0.lock().unwrap().status()))
}

struct MonitorState(Mutex<monitor::Monitor>);
struct SweepState(Mutex<sweep::SweepEngine>);
pub struct DriverState(pub Mutex<driver::DriverManager>);

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let driver_mgr = driver::DriverManager::new();
    let driver_loaded = matches!(driver_mgr.status(), driver::DriverStatus::Loaded);
    if driver_loaded {
        println!("[NIDAVELLIR] Kernel driver loaded successfully");
    } else {
        println!("[NIDAVELLIR] Kernel driver not available: {:?}", driver_mgr.status());
    }

    let mut sweep_engine = sweep::SweepEngine::new();
    sweep_engine.set_simulator(!driver_loaded);

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(MonitorState(Mutex::new(monitor::Monitor::new())))
        .manage(SweepState(Mutex::new(sweep_engine)))
        .manage(DriverState(Mutex::new(driver_mgr)))
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
            get_driver_status,
        ])
        .run(tauri::generate_context!())
        .expect("error while running nidavellir");
}
