import { invoke } from "@tauri-apps/api/core";

/** True during `tauri dev` / Vite dev server; false in release installer builds. */
const isDev = import.meta.env.DEV;

export async function serviceCall(method, params = null) {
  return params == null
    ? invoke("service_request", { method })
    : invoke("service_request", { method, params });
}

export async function pingService() {
  return invoke("service_ping");
}

/** Human-readable driver line for UI (avoids duplicating status + detail). */
export function formatDriverStatus(payload) {
  if (!payload) return "Unknown";
  switch (payload.status) {
    case "loaded":
      return "PawnIO connected";
    case "not_installed":
      return isDev
        ? "PawnIO not installed (optional in v0.1)"
        : "PawnIO not installed";
    case "error":
      return payload.detail ?? "Driver error";
    default:
      return payload.status;
  }
}

/** Extra hint shown below the driver line when useful. */
export function driverStatusHint(payload) {
  if (!payload) return null;
  if (payload.status === "not_installed") {
    if (isDev) {
      return "Hardware detection and sensors work without PawnIO. Install from https://pawnio.eu/ when you need MSR access.";
    }
    return "Re-run the Nidavellir installer and accept the bundled PawnIO driver step for CPU MSR access.";
  }
  if (payload.status === "error" && payload.detail) {
    return payload.detail;
  }
  return null;
}

/** Hint when IPC to Core Service fails. */
export function serviceUnavailableHint() {
  if (isDev) {
    return "Start the Core Service: cargo run -p nidavellir-service -- console";
  }
  return "Ensure Nidavellir Core Service is running (Services → NidavellirCore). Reinstall from the setup package if missing.";
}
