mod ipc_client;

use serde_json::Value;

#[tauri::command]
fn service_request(method: String, params: Option<Value>) -> Result<Value, String> {
    ipc_client::call_service_with_params(&method, params)
}

#[tauri::command]
fn service_ping() -> Result<Value, String> {
    ipc_client::call_service("Ping")
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![service_request, service_ping])
        .run(tauri::generate_context!())
        .expect("error while running nidavellir ui");
}
