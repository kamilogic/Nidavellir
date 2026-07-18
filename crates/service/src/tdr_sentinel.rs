//! v17 runtime TDR sentinel — Stage A (Event Log layer).
//!
//! Watches the Windows System event log for `nvlddmkm` Event ID 153 ("BusReset TDR") while a
//! Nidavellir undervolt profile is applied, and breaks the observed 5-strike hang cascade at the
//! FIRST event: reset to stock → blacklist the failed intent (durable Safe Loop knowledge the next
//! forge consumes as a boundary) → auto-fallback the profile +3 physical voltage bins at the SAME
//! clock (preserve identity) and re-apply. A SECOND event in the same service session stops the
//! ladder: stay stock, clear the applied profile (boot never re-applies the failed point) — if two
//! bumps didn't fix it, the problem is not one bin.
//!
//! Cost: one filtered `wevtutil` query every ~15 s (sub-millisecond CPU, ZERO GPU — never touches
//! the card during gameplay). The GPU canary (silent-error layer) is Stage B.
//!
//! Safety guards: acts ONLY when (a) an F2 undervolt profile is applied, (b) the Safe Loop boot
//! flag is NOT armed (during forge dwells the flag is armed — and a dwell DeviceLost RETAINS it —
//! so forge-owned TDRs are never double-handled), (c) the event is NEWER than service start /
//! the last handled event (historical log entries never trigger on boot).

#![cfg(windows)]

use tracing::{info, warn};
use nidavellir_core::safe_loop::{BlacklistRegion, SafeLoopStore, TuningPoint, DEFAULT_BLACKLIST_RADIUS};

const SENTINEL_POLL_MS: u64 = 15_000;
/// Post-action settle time (driver just recovered + we rewrote the curve).
const SENTINEL_COOLDOWN_MS: u64 = 60_000;
/// TDR is the gravest failure class → +3 physical bins at the same clock.
const SENTINEL_TDR_BUMP_BINS: usize = 3;
/// A canary-detected silent error is boundary-class → +2 bins at the same clock.
const SENTINEL_SILENT_BUMP_BINS: usize = 2;
/// Canary cadence: one owned TextureRop self-check every 20 s, and ONLY while the GPU is under
/// real load (silent errors at elastic idle voltages are meaningless and the canary must never
/// keep an idle card awake).
const SENTINEL_CANARY_POLL_MS: u64 = 20_000;
/// TextureRop self-check burst: long enough for ≥2 of the 250 ms checksum windows (reference +
/// compare). ~700 ms of shared GPU load every 20 s ≈ 3.5% duty while gaming — the price of a
/// canary that samples the BINDING failure path instead of being statistically blind.
const SENTINEL_CANARY_KERNEL_MS: u64 = 700;
const SENTINEL_CANARY_MIN_UTIL_PCT: f64 = 30.0;
/// Operator policy (2026-07-12, after the first successful field recovery): THREE strikes —
/// two automatic bumps, the third failure resets to stock and clears the profile. On the field
/// case (1815@843 → 862 exhausted at 2 strikes) the third bump would have landed 875 mV — the
/// operator's hand-validated golden voltage.
const SENTINEL_MAX_BUMPS_PER_SESSION: u32 = 2;

/// What the sentinel decided for a detected TDR. Pure decision — testable without hardware.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SentinelAction {
    /// No undervolt applied (stock/F1) — record and move on.
    Ignore,
    /// Forge owns the GPU right now (boot flag armed) — never double-handle a dwell TDR.
    ForgeOwns,
    /// First failure: bump to this anchor (same clock) and re-apply.
    Bump { target_mhz: u32, new_anchor_mv: u32 },
    /// Ladder exhausted (or no higher bin exists): stay stock, clear the applied profile.
    Stock { target_mhz: u32, failed_anchor_mv: u32 },
}

/// Only a completed canary verdict can drive the silent-error fallback. A stalled GPU call stays
/// owned by the dedicated canary thread; the Event Log layer and boot reconciliation remain the
/// authoritative recovery paths for TDRs and hard wedges.
fn canary_returned_failure(verdict: &nidavellir_core::gpu_sweep::StabilityResult) -> bool {
    !matches!(verdict, nidavellir_core::gpu_sweep::StabilityResult::Stable)
}

/// Pure fallback-ladder decision. `bins_above` are the physical VF bins strictly above the failed
/// anchor, ascending (from the live sane curve).
pub(crate) fn sentinel_decide(
    applied: Option<(u32, u32)>,
    boot_flag_armed: bool,
    bumps_this_session: u32,
    bins_above: &[u32],
    bump_bins: usize,
) -> SentinelAction {
    let Some((target_mhz, anchor_mv)) = applied else {
        return SentinelAction::Ignore;
    };
    if boot_flag_armed {
        return SentinelAction::ForgeOwns;
    }
    if bumps_this_session >= SENTINEL_MAX_BUMPS_PER_SESSION {
        return SentinelAction::Stock { target_mhz, failed_anchor_mv: anchor_mv };
    }
    // +N bins, or the highest available if the curve runs out before that (never less than +1).
    match bins_above.get(bump_bins.saturating_sub(1)).or(bins_above.last()) {
        Some(&new_anchor_mv) => SentinelAction::Bump { target_mhz, new_anchor_mv },
        None => SentinelAction::Stock { target_mhz, failed_anchor_mv: anchor_mv },
    }
}

/// Extract `SystemTime='…'` from `wevtutil /f:xml` output (locale-proof, no regex).
pub(crate) fn parse_event_system_time(xml: &str) -> Option<String> {
    let start = xml.find("SystemTime='")? + "SystemTime='".len();
    let end = xml[start..].find('\'')? + start;
    Some(xml[start..end].to_string())
}

/// Newest nvlddmkm-153 event timestamp, if any.
pub(crate) fn query_latest_tdr_event() -> Option<String> {
    let out = std::process::Command::new("wevtutil")
        .args([
            "qe",
            "System",
            "/q:*[System[Provider[@Name='nvlddmkm'] and (EventID=153)]]",
            "/c:1",
            "/rd:true",
            "/f:xml",
        ])
        .output()
        .ok()?;
    parse_event_system_time(&String::from_utf8_lossy(&out.stdout))
}

/// Physical VF bins strictly above `anchor_mv` on the live sane base curve, ascending.
fn bins_above_anchor(anchor_mv: u32) -> Vec<u32> {
    use nidavellir_gpu_nvapi as gpu;
    let mut bins: Vec<u32> = gpu::read_vf_base_curve_modern()
        .into_iter()
        .filter(|&(_, mv, f)| crate::gpu_undervolt::is_f2_sane_point(mv, f))
        .map(|(_, mv, _)| mv)
        .filter(|&mv| mv > anchor_mv)
        .collect();
    bins.sort_unstable();
    bins.dedup();
    bins
}

fn baseline_path() -> std::path::PathBuf {
    nidavellir_core::safe_loop::default_data_dir().join("sentinel_baseline.txt")
}

fn persist_baseline(ts: &str) {
    let _ = std::fs::create_dir_all(nidavellir_core::safe_loop::default_data_dir());
    let _ = std::fs::write(baseline_path(), ts);
}

/// BOOT RECONCILIATION — the hole a live sentinel cannot cover: a hard WEDGE freezes the whole
/// machine (sentinel included), the operator power-cycles, and on the next boot the crash events
/// are HISTORICAL (correctly inert for the live watcher) while `reapply_on_boot` would happily
/// re-apply the exact profile that just froze the PC. Called BEFORE `reapply_on_boot`: if a
/// nvlddmkm-153 newer than the last persisted baseline exists AND an undervolt profile is
/// persisted, the crash happened on OUR watch → blacklist the point + clear the applied profile
/// (boot comes up STOCK — a hard wedge is ladder-exhausted-grade, no auto-bump at cold boot).
/// First run (no baseline file) only initializes the baseline. Returns true when it demoted.
pub fn startup_reconcile(store: &SafeLoopStore) -> bool {
    let newest = query_latest_tdr_event();
    let baseline = std::fs::read_to_string(baseline_path()).ok();
    let Some(newest) = newest else { return false };
    let Some(baseline) = baseline else {
        persist_baseline(&newest);
        return false;
    };
    persist_baseline(&newest);
    if newest.as_str() <= baseline.trim() {
        return false;
    }
    let Some(applied) = crate::gpu_apply::load_applied().and_then(|p| p.undervolt) else {
        return false;
    };
    warn!(
        "sentinel: TDR/wedge at {} MHz @ {} mV happened while the service was down (event {newest}          > baseline) — blacklisting and clearing the profile; boot stays STOCK",
        applied.target_mhz, applied.anchor_mv
    );
    let mut rec = store.load_record();
    let intent = TuningPoint::from_axes([
        ("gpu_freq_mhz", applied.target_mhz as i64),
        ("gpu_vf_bin_mv", applied.anchor_mv as i64),
    ]);
    rec.blacklist.push(BlacklistRegion::around(intent, DEFAULT_BLACKLIST_RADIUS));
    let _ = store.save_record(&rec);
    crate::gpu_undervolt::append_condemnation(
        store.base_dir(),
        nidavellir_core::condemnation::CondemnationSeverity::Rigid,
        nidavellir_core::condemnation::KIND_FIELD_TDR,
        Some(crate::gpu_power_sweep::current_gpu_key()),
        applied.target_mhz,
        applied.anchor_mv,
        None,
        "TDR/wedge while the service was down with the profile applied; boot stays stock".into(),
    );
    crate::gpu_apply::sentinel_clear_applied();
    append_sentinel_log(&format!(
        "\"event\":\"boot-reconcile\",\"action\":\"stock\",\"target_mhz\":{},\"failed_mv\":{}",
        applied.target_mhz, applied.anchor_mv
    ));
    true
}

/// UI-facing status (overwritten each action): last sentinel event + operator recommendation.
fn write_sentinel_status(json: &str) {
    let _ = std::fs::create_dir_all(nidavellir_core::safe_loop::default_data_dir());
    let _ = std::fs::write(
        nidavellir_core::safe_loop::default_data_dir().join("sentinel_status.json"),
        json,
    );
}

fn append_sentinel_log(entry: &str) {
    let path = nidavellir_core::safe_loop::default_data_dir().join("sentinel_log.jsonl");
    let line = format!(
        "{{\"ts\":\"{}\",{entry}}}\n",
        nidavellir_core::f2_observation::now_rfc3339()
    );
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut f| std::io::Write::write_all(&mut f, line.as_bytes()));
}

/// Delete a file, treating "already absent" as success — a reset must be idempotent.
fn remove_if_exists(path: &std::path::Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

/// Wipe ALL persisted sentinel state so a full "forget everything" reset (which drops the Safe Loop
/// blacklist and every learned observation) leaves the sentinel CONSISTENT with that clean slate:
/// the boot baseline, the last-action dedup stamp, the UI status card, and the append-only event log.
/// The running watcher thread keeps its in-memory `last_handled` pinned to the newest event, so it
/// never re-handles an old TDR, and the profile it guards was just cleared to stock — nothing is lost
/// by starting the history fresh. Returns the name of any file that could not be removed.
pub fn reset_sentinel_state() -> Vec<String> {
    let dir = nidavellir_core::safe_loop::default_data_dir();
    let mut problems = Vec::new();
    for name in ["sentinel_baseline.txt", "sentinel_status.json", "sentinel_log.jsonl"] {
        if let Err(e) = remove_if_exists(&dir.join(name)) {
            problems.push(format!("sentinel {name}: {e}"));
        }
    }
    LAST_ACTION_EPOCH_S.store(0, std::sync::atomic::Ordering::SeqCst);
    problems
}

/// Cross-LAYER episode dedup: the canary (stall) and the event-log watcher (the driver's 153 from
/// the same episode, seconds later) must never both act — observed in the field as a doubled
/// "ladder exhausted" 4 s apart. Any layer that ACTS stamps this; the other skips within the window.
static LAST_ACTION_EPOCH_S: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
const CROSS_LAYER_DEDUP_S: u64 = 90;

fn epoch_s() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Atomically claim one cross-layer recovery episode. The watcher and canary are independent
/// threads; a load followed by an unconditional store lets both mutate the GPU when they observe
/// the same failure concurrently. Compare-exchange makes exactly one layer the recovery owner.
fn claim_action_epoch(
    last_action: &std::sync::atomic::AtomicU64,
    now: u64,
) -> bool {
    loop {
        let previous = last_action.load(std::sync::atomic::Ordering::SeqCst);
        if previous != 0 && now.saturating_sub(previous) < CROSS_LAYER_DEDUP_S {
            return false;
        }
        match last_action.compare_exchange(
            previous,
            now,
            std::sync::atomic::Ordering::SeqCst,
            std::sync::atomic::Ordering::SeqCst,
        ) {
            Ok(_) => return true,
            Err(_) => continue,
        }
    }
}

/// Handle one confirmed failure (TDR event or canary silent error). Returns true on a bump.
fn handle_failure(store: &SafeLoopStore, bumps_this_session: u32, bump_bins: usize, kind: &str) -> bool {
    let last = LAST_ACTION_EPOCH_S.load(std::sync::atomic::Ordering::SeqCst);
    let now = epoch_s();
    if last != 0 && now.saturating_sub(last) < CROSS_LAYER_DEDUP_S {
        info!("sentinel: {kind} within {CROSS_LAYER_DEDUP_S}s of the last action — same episode, skipping");
        append_sentinel_log(&format!("\"event\":\"{kind}\",\"action\":\"same-episode-skip\""));
        return false;
    }
    let applied = crate::gpu_apply::load_applied()
        .and_then(|p| p.undervolt.map(|u| (u.target_mhz, u.anchor_mv)));
    let action = sentinel_decide(
        applied,
        store.is_boot_flag_armed(),
        bumps_this_session,
        &applied.map(|(_, mv)| bins_above_anchor(mv)).unwrap_or_default(),
        bump_bins,
    );
    match action {
        SentinelAction::Ignore => {
            info!("sentinel: nvlddmkm-153 TDR with no undervolt applied — recorded, no action");
            append_sentinel_log(&format!("\"event\":\"{kind}\",\"action\":\"ignore\""));
            false
        }
        SentinelAction::ForgeOwns => {
            info!("sentinel: TDR while the Safe Loop boot flag is armed — forge owns recovery");
            append_sentinel_log(&format!("\"event\":\"{kind}\",\"action\":\"forge-owns\""));
            false
        }
        SentinelAction::Bump { target_mhz, new_anchor_mv } => {
            if !claim_action_epoch(&LAST_ACTION_EPOCH_S, now) {
                info!("sentinel: {kind} recovery was claimed concurrently by the other layer");
                append_sentinel_log(&format!(
                    "\"event\":\"{kind}\",\"action\":\"same-episode-skip\""
                ));
                return false;
            }
            let (_, failed_mv) = applied.expect("Bump implies applied");
            warn!(
                "sentinel: in-game TDR at {target_mhz} MHz @ {failed_mv} mV — resetting to stock, \
                 blacklisting, auto-fallback to {new_anchor_mv} mV (+{SENTINEL_TDR_BUMP_BINS} bins)"
            );
            // Capture label/mem-offset BEFORE reset() — it deletes gpu_applied.json (audit #1).
            let prior = crate::gpu_apply::load_applied();
            let prior_label = prior.as_ref().map(|p| p.label.clone()).unwrap_or_default();
            let prior_mem = prior.as_ref().and_then(|p| p.mem_offset_mhz);
            // 1. Deterministic stock (the driver already bus-reset; make our state match).
            if let Err(e) = crate::gpu_apply::reset(store) {
                warn!("sentinel: stock reset failed ({e}) — staying stock, no re-apply");
                crate::gpu_apply::sentinel_clear_applied();
                append_sentinel_log(&format!("\"event\":\"{kind}\",\"action\":\"stock\",\"reason\":\"reset-failed\""));
                return false;
            }
            // 2. Durable blacklist: the failed (clock, vf_bin) intent — the next forge descent
            //    stops ABOVE it (BlacklistedBoundary), so real-world failures refine the frontier.
            let mut rec = store.load_record();
            let intent = TuningPoint::from_axes([
                ("gpu_freq_mhz", target_mhz as i64),
                ("gpu_vf_bin_mv", failed_mv as i64),
            ]);
            rec.blacklist.push(BlacklistRegion::around(intent, DEFAULT_BLACKLIST_RADIUS));
            if let Err(e) = store.save_record(&rec) {
                warn!("sentinel: blacklist persist failed: {e}");
            }
            crate::gpu_undervolt::append_condemnation(
                store.base_dir(),
                nidavellir_core::condemnation::CondemnationSeverity::Rigid,
                nidavellir_core::condemnation::KIND_FIELD_TDR,
                Some(crate::gpu_power_sweep::current_gpu_key()),
                target_mhz,
                failed_mv,
                None,
                format!("in-game {kind}; sentinel auto-fallback to {new_anchor_mv} mV"),
            );
            // 3. Preserve-identity fallback: same clock, higher bin — through the FULL guarded
            //    apply path (audit #1: arms the Safe Loop boot flag around the autonomous write +
            //    8 s survival window, so a bumped point that cold-hangs is NOT re-applied on boot;
            //    also re-applies the prior mem offset and persists the composed label).
            // Compose from the BASE label (strip any previous sentinel suffix) so repeated
            // strikes update the description instead of growing it.
            let base_label = prior_label.split(" · sentinela").next().unwrap_or("").trim_end();
            let strike = bumps_this_session + 1;
            match crate::gpu_apply::apply_and_persist_undervolt(
                format!(
                    "{base_label} · sentinela {kind}: {failed_mv}→{new_anchor_mv} mV (strike {strike}/3)"
                ),
                target_mhz,
                new_anchor_mv,
                prior_mem,
                store,
            ) {
                Ok(()) => {
                    append_sentinel_log(&format!(
                        "\"event\":\"{kind}\",\"action\":\"bump\",\"target_mhz\":{target_mhz},\
                         \"failed_mv\":{failed_mv},\"new_mv\":{new_anchor_mv}"
                    ));
                    write_sentinel_status(&format!(
                        "{{\"ts\":\"{}\",\"event\":\"{kind}\",\"action\":\"bump\",\"strike\":{strike},\"target_mhz\":{target_mhz},\"failed_mv\":{failed_mv},\"new_mv\":{new_anchor_mv},\"recommendation\":\"Instabilidade detectada em jogo: o perfil foi rebaixado automaticamente para {new_anchor_mv} mV (strike {strike}/3). Se estabilizar, re-forje quando puder para requalificar; a falha ja esta na blacklist e o proximo Forge evita o ponto ruim sozinho.\"}}",
                        nidavellir_core::f2_observation::now_rfc3339()
                    ));
                    true
                }
                Err(e) => {
                    warn!("sentinel: fallback re-apply failed ({e}) — staying stock");
                    crate::gpu_apply::sentinel_clear_applied();
                    append_sentinel_log(&format!("\"event\":\"{kind}\",\"action\":\"stock\",\"reason\":\"reapply-failed\""));
                    false
                }
            }
        }
        SentinelAction::Stock { target_mhz, failed_anchor_mv } => {
            if !claim_action_epoch(&LAST_ACTION_EPOCH_S, now) {
                info!("sentinel: {kind} recovery was claimed concurrently by the other layer");
                append_sentinel_log(&format!(
                    "\"event\":\"{kind}\",\"action\":\"same-episode-skip\""
                ));
                return false;
            }
            warn!(
                "sentinel: TDR at {target_mhz} MHz @ {failed_anchor_mv} mV after a previous bump \
                 this session — staying at stock (ladder exhausted); profile cleared"
            );
            let _ = crate::gpu_apply::reset(store);
            let mut rec = store.load_record();
            let intent = TuningPoint::from_axes([
                ("gpu_freq_mhz", target_mhz as i64),
                ("gpu_vf_bin_mv", failed_anchor_mv as i64),
            ]);
            rec.blacklist.push(BlacklistRegion::around(intent, DEFAULT_BLACKLIST_RADIUS));
            let _ = store.save_record(&rec);
            crate::gpu_undervolt::append_condemnation(
                store.base_dir(),
                nidavellir_core::condemnation::CondemnationSeverity::Rigid,
                nidavellir_core::condemnation::KIND_FIELD_TDR,
                Some(crate::gpu_power_sweep::current_gpu_key()),
                target_mhz,
                failed_anchor_mv,
                None,
                format!("{kind} after a prior bump this session; ladder exhausted, stock"),
            );
            crate::gpu_apply::sentinel_clear_applied();
            append_sentinel_log(&format!("\"event\":\"{kind}\",\"action\":\"stock\",\"reason\":\"ladder-exhausted\""));
            write_sentinel_status(&format!(
                "{{\"ts\":\"{}\",\"event\":\"{kind}\",\"action\":\"stock\",\"strike\":3,\"target_mhz\":{target_mhz},\"failed_mv\":{failed_anchor_mv},\"recommendation\":\"3 falhas na mesma sessao: GPU em STOCK e perfil removido por seguranca. Recomendado: aplicar Deep Calm (validado) e re-forjar — os pontos ruins ja estao na blacklist e o novo Forge parte acima deles.\"}}",
                nidavellir_core::f2_observation::now_rfc3339()
            ));
            false
        }
    }
}

/// Spawn the sentinel thread. Baseline = the newest historical event at start (never re-handled).
pub fn spawn(store: SafeLoopStore) {
    // Shared bump budget across BOTH layers: one automatic bump per service session, total.
    let bumps = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));

    // Layer 1 — Event Log (TDR/BusReset; authoritative, zero GPU).
    {
        let store = store.clone();
        let bumps = std::sync::Arc::clone(&bumps);
        std::thread::spawn(move || {
            let mut last_handled = query_latest_tdr_event();
            // Audit #3: absolute time floor — even if the baseline query failed (Event Log not
            // ready at boot), a HISTORICAL event can never trigger an action. Both timestamp
            // formats share the "YYYY-MM-DDTHH:MM:SS" prefix, so a 19-char lexicographic compare
            // is a valid ordering at second granularity.
            let service_start = nidavellir_core::f2_observation::now_rfc3339();
            let start_floor: String = service_start.chars().take(19).collect();
            info!("sentinel: watching nvlddmkm-153 (baseline {last_handled:?}, floor {start_floor})");
            loop {
                std::thread::sleep(std::time::Duration::from_millis(SENTINEL_POLL_MS));
                let Some(newest) = query_latest_tdr_event() else { continue };
                if last_handled.as_deref() == Some(newest.as_str()) {
                    continue;
                }
                let is_historical = newest.len() >= 19 && newest[..19] < start_floor[..];
                persist_baseline(&newest);
                last_handled = Some(newest.clone());
                if is_historical {
                    continue;
                }
                // Never mutate hardware while Forge owns it. Hand the event to the run owner: it
                // persists attribution when the boot flag is armed, otherwise records an explicitly
                // unattributed incident, then requests a cooperative stop.
                if crate::gpu_power_sweep::FORGE_ACTIVE.load(std::sync::atomic::Ordering::SeqCst) {
                    let recorded = crate::gpu_power_sweep::record_active_forge_tdr(&store, &newest);
                    info!(
                        "sentinel: TDR event during active Forge — incident recorded={recorded}, cooperative stop requested"
                    );
                    append_sentinel_log(
                        "\"event\":\"tdr\",\"action\":\"forge-stop-requested\"",
                    );
                    continue;
                }
                let n = bumps.load(std::sync::atomic::Ordering::SeqCst);
                if handle_failure(&store, n, SENTINEL_TDR_BUMP_BINS, "tdr") {
                    bumps.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
                std::thread::sleep(std::time::Duration::from_millis(SENTINEL_COOLDOWN_MS));
                // Audit #5: absorb the SAME episode's cascade residue (multiple 153s logged while
                // we were handling + cooling down) so it can never burn the 2nd strike — only a
                // genuinely NEW failure after this point counts against the session budget.
                if let Some(residual) = query_latest_tdr_event() {
                    persist_baseline(&residual);
                    last_handled = Some(residual);
                }
            }
        });
    }

    // Layer 2 — GPU canary (v17.3, ACTIVE): a ~700 ms TextureRop SELF-CHECK every 20 s, ONLY while
    // an undervolt is applied AND the GPU is under real load (>30% util — an idle card is never
    // woken). It detects returned non-stable verdicts on the binding TextureRop path.
    // The check runs synchronously on this dedicated thread so its GPU context is never detached:
    // if the driver stalls, no replacement worker is spawned and no reset races a still-running
    // canary. The independent Event Log layer handles a recovered TDR; boot reconciliation remains
    // the final net for a full machine wedge.
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_millis(SENTINEL_CANARY_POLL_MS));
        let applied = crate::gpu_apply::load_applied().and_then(|p| p.undervolt);
        if applied.is_none()
            || store.is_boot_flag_armed()
            // Audit #2: never spin a second GPU context or act while a forge run owns the card.
            || crate::gpu_power_sweep::FORGE_ACTIVE.load(std::sync::atomic::Ordering::SeqCst)
        {
            continue;
        }
        let under_load = nidavellir_core::nvml_gpu::read_nvidia_gpus_nvml()
            .first()
            .and_then(|g| g.utilization_pct)
            .is_some_and(|u| u >= SENTINEL_CANARY_MIN_UTIL_PCT);
        if !under_load {
            continue;
        }
        // Keep the context and call owned by this thread for their entire lifetime. Rust threads
        // cannot be cancelled safely; a recv_timeout around an inner worker would only abandon the
        // worker and let recovery mutate the GPU while that worker was still running.
        let verdict = match nidavellir_gpu_stress::GpuCtx::new() {
            Ok(ctx) => {
                ctx.run_canary_texture_selfcheck(SENTINEL_CANARY_KERNEL_MS)
                    .result
            }
            // Context creation failed (driver busy/hiccup) — inconclusive, never a fallback.
            Err(_) => continue,
        };
        if canary_returned_failure(&verdict) {
            warn!("sentinel: texture canary detected {verdict:?} at the applied point");
            let n = bumps.load(std::sync::atomic::Ordering::SeqCst);
            if handle_failure(&store, n, SENTINEL_SILENT_BUMP_BINS, "silent-canary") {
                bumps.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
            std::thread::sleep(std::time::Duration::from_millis(SENTINEL_COOLDOWN_MS));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remove_if_exists_is_idempotent() {
        let mut path = std::env::temp_dir();
        path.push(format!("nid-sentinel-reset-test-{}.tmp", std::process::id()));
        std::fs::write(&path, b"x").unwrap();
        assert!(remove_if_exists(&path).is_ok(), "removes an existing file");
        assert!(!path.exists());
        // Second call on the now-missing file must still succeed (reset is idempotent).
        assert!(remove_if_exists(&path).is_ok(), "missing file is not an error");
    }

    #[test]
    fn sentinel_ladder_bumps_three_bins_then_goes_stock() {
        let bins = [850, 856, 862, 868];
        // First failure at 1815@843 → +3 bins = 862, same clock (preserve identity).
        assert_eq!(
            sentinel_decide(Some((1815, 843)), false, 0, &bins, SENTINEL_TDR_BUMP_BINS),
            SentinelAction::Bump { target_mhz: 1815, new_anchor_mv: 862 }
        );
        // Second failure → still bumps (3-strike policy); THIRD failure → stock.
        assert_eq!(
            sentinel_decide(Some((1815, 862)), false, 1, &[868, 875], SENTINEL_TDR_BUMP_BINS),
            SentinelAction::Bump { target_mhz: 1815, new_anchor_mv: 875 }
        );
        assert_eq!(
            sentinel_decide(Some((1815, 875)), false, 2, &[881], SENTINEL_TDR_BUMP_BINS),
            SentinelAction::Stock { target_mhz: 1815, failed_anchor_mv: 875 }
        );
        // Curve runs out before +3 → highest available bin, never a lower one.
        assert_eq!(
            sentinel_decide(Some((1815, 843)), false, 0, &[850], SENTINEL_TDR_BUMP_BINS),
            SentinelAction::Bump { target_mhz: 1815, new_anchor_mv: 850 }
        );
        // No higher bin at all → stock.
        assert_eq!(
            sentinel_decide(Some((1815, 843)), false, 0, &[], SENTINEL_TDR_BUMP_BINS),
            SentinelAction::Stock { target_mhz: 1815, failed_anchor_mv: 843 }
        );
        // Forge owns the GPU (boot flag armed) → never double-handle a dwell TDR.
        assert_eq!(sentinel_decide(Some((1815, 843)), true, 0, &bins, SENTINEL_TDR_BUMP_BINS), SentinelAction::ForgeOwns);
        // Canary silent error is boundary-class → +2 bins (850→856 from 843).
        assert_eq!(
            sentinel_decide(Some((1815, 843)), false, 0, &bins, SENTINEL_SILENT_BUMP_BINS),
            SentinelAction::Bump { target_mhz: 1815, new_anchor_mv: 856 }
        );
        // Stock / F1 applied → nothing to do.
        assert_eq!(sentinel_decide(None, false, 0, &bins, SENTINEL_TDR_BUMP_BINS), SentinelAction::Ignore);
    }

    #[test]
    fn canary_only_acts_on_a_returned_failure_verdict() {
        use nidavellir_core::gpu_sweep::StabilityResult;

        assert!(!canary_returned_failure(&StabilityResult::Stable));
        assert!(canary_returned_failure(&StabilityResult::SilentError));
        assert!(canary_returned_failure(&StabilityResult::Unstable));
        assert!(canary_returned_failure(&StabilityResult::Crash));
    }

    #[test]
    fn cross_layer_recovery_claim_is_atomic_and_respects_the_dedup_window() {
        let last = std::sync::atomic::AtomicU64::new(0);
        assert!(claim_action_epoch(&last, 1_000));
        assert!(!claim_action_epoch(&last, 1_000));
        assert!(!claim_action_epoch(
            &last,
            1_000 + CROSS_LAYER_DEDUP_S - 1
        ));
        assert!(claim_action_epoch(&last, 1_000 + CROSS_LAYER_DEDUP_S));
    }

    #[test]
    fn parses_wevtutil_xml_system_time() {
        let xml = "<Event><System><TimeCreated SystemTime='2026-07-09T07:43:40.123456700Z'/></System></Event>";
        assert_eq!(
            parse_event_system_time(xml).as_deref(),
            Some("2026-07-09T07:43:40.123456700Z")
        );
        assert_eq!(parse_event_system_time("no events"), None);
    }
}
