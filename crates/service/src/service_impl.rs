use std::sync::{Arc, Mutex};
use std::time::Duration;

use tracing::info;
use windows_service::service::{
    ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
    ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};

use crate::ipc_server;
use crate::AppState;
use crate::SERVICE_NAME;
use nidavellir_driver_pawnio::DriverManager;

pub fn run_service() -> windows_service::Result<()> {
    let (shutdown_tx, shutdown_rx) = std::sync::mpsc::channel();

    let event_handler = move |control_event| -> ServiceControlHandlerResult {
        match control_event {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                let _ = shutdown_tx.send(());
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        }
    };

    let status_handle =
        service_control_handler::register(SERVICE_NAME, event_handler)?;

    status_handle.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::Running,
        controls_accepted: ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    })?;

    info!("Nidavellir Core Service started");

    // Parachute first: the service boots before login, so it reads the
    // boot-flag and recovers from any prior crash before touching hardware.
    let safe_store = nidavellir_core::safe_loop::SafeLoopStore::system();
    crate::safe_loop_runtime::run_startup_recovery(&safe_store);
    crate::safe_loop_runtime::spawn_heartbeat(safe_store.clone());

    let hw = nidavellir_core::detect_hardware();
    let state = Arc::new(Mutex::new(AppState {
        driver: DriverManager::new(),
        sensor_engine: nidavellir_core::sensors::SensorEngine::new(),
        motherboard: hw.motherboard,
        safe_store,
        gpu_sweep: crate::gpu_sweep_runtime::GpuSweepHandle::default(),
    }));

    let pipe_state = Arc::clone(&state);
    std::thread::spawn(move || {
        if let Err(e) = ipc_server::run_pipe_server(pipe_state) {
            tracing::error!("Pipe server error: {e}");
        }
    });

    let _ = shutdown_rx.recv();

    status_handle.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::Stopped,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    })?;

    Ok(())
}
