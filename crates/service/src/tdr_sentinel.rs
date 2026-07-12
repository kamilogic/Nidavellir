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
/// Canary cadence + kernel budget: ~5 ms of known-answer ALU every 30 s (<0.02% GPU), and ONLY
/// while the GPU is under real load (silent errors at elastic idle voltages are meaningless and
/// the kernel must never keep an idle card awake).
const SENTINEL_CANARY_POLL_MS: u64 = 20_000;
/// TextureRop self-check burst: long enough for ≥2 of the 250 ms checksum windows (reference +
/// compare). ~700 ms of shared GPU load every 20 s ≈ 3.5% duty while gaming — the price of a
/// canary that samples the BINDING failure path instead of being statistically blind.
const SENTINEL_CANARY_KERNEL_MS: u64 = 700;
const SENTINEL_CANARY_MIN_UTIL_PCT: f64 = 30.0;
/// Stall watchdog: the canary normally returns in well under a second. If it has not come back by
/// this deadline the GPU is ALREADY stalling — that IS the pre-hang precursor of the BusReset
/// cascade. Act immediately (stock reset from this thread) instead of waiting for the driver's
/// ~2 s TDR watchdog to start the cascade.
const SENTINEL_CANARY_STALL_MS: u64 = 3_000;
/// One automatic bump per service session; the second event goes to stock and stays there.
const SENTINEL_MAX_BUMPS_PER_SESSION: u32 = 1;

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
fn query_latest_tdr_event() -> Option<String> {
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
    crate::gpu_apply::sentinel_clear_applied();
    append_sentinel_log(&format!(
        "\"event\":\"boot-reconcile\",\"action\":\"stock\",\"target_mhz\":{},\"failed_mv\":{}",
        applied.target_mhz, applied.anchor_mv
    ));
    true
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

/// Handle one confirmed failure (TDR event or canary silent error). Returns true on a bump.
fn handle_failure(store: &SafeLoopStore, bumps_this_session: u32, bump_bins: usize, kind: &str) -> bool {
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
            // 3. Preserve-identity fallback: same clock, higher bin — through the FULL guarded
            //    apply path (audit #1: arms the Safe Loop boot flag around the autonomous write +
            //    8 s survival window, so a bumped point that cold-hangs is NOT re-applied on boot;
            //    also re-applies the prior mem offset and persists the composed label).
            match crate::gpu_apply::apply_and_persist_undervolt(
                format!("{} (sentinela +{bump_bins} bins)", prior_label.trim_end()),
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
            crate::gpu_apply::sentinel_clear_applied();
            append_sentinel_log(&format!("\"event\":\"{kind}\",\"action\":\"stock\",\"reason\":\"ladder-exhausted\""));
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
                last_handled = Some(newest);
                if is_historical {
                    continue;
                }
                // Audit #2: never act while a forge run owns the GPU (the per-step boot flag has
                // inter-step windows a forge-induced 153 can land in).
                if crate::gpu_power_sweep::FORGE_ACTIVE.load(std::sync::atomic::Ordering::SeqCst) {
                    info!("sentinel: TDR event during an active forge run — forge owns recovery");
                    append_sentinel_log("\"event\":\"tdr\",\"action\":\"forge-active-skip\"");
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

    // Layer 2 — GPU canary (v17.2, ACTIVE): a ~700 ms TextureRop SELF-CHECK every 20 s, ONLY while
    // an undervolt is applied AND the GPU is under real load (>30% util — an idle card is never
    // woken). Two jobs: (1) silent-error detection on the BINDING path (texture-rop — where every
    // forge failure fired; the old 5 ms ALU kernel was statistically blind: wrong unit, 0.017%
    // duty); (2) STALL WATCHDOG — a canary that never returns is the pre-hang itself, and we reset
    // to stock BEFORE the driver's 2 s TDR watchdog can start the cascade. Honest limit: a
    // full bus wedge can still freeze even our reset call — boot reconciliation is the final net.
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
        // v17.2: the canary runs on ITS OWN thread with a stall watchdog. Normal return: a
        // TextureRop self-check verdict (the BINDING detector — every forge boundary failure fired
        // in texture-rop, never ALU). No return by the deadline: the GPU is already stalling — the
        // pre-hang precursor of the BusReset cascade — and we act from HERE, before the driver's
        // ~2 s watchdog starts the cascade the operator had to power-cycle out of.
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let verdict = nidavellir_gpu_stress::GpuCtx::new().map(|ctx| {
                ctx.run_canary_texture_selfcheck(SENTINEL_CANARY_KERNEL_MS).result
            });
            let _ = tx.send(verdict);
        });
        let outcome =
            rx.recv_timeout(std::time::Duration::from_millis(SENTINEL_CANARY_STALL_MS));
        let (failed, kind) = match outcome {
            // Healthy canary — clean self-consistent render.
            Ok(Ok(nidavellir_core::gpu_sweep::StabilityResult::Stable)) => (false, ""),
            // Context creation failed (driver busy/hiccup) — inconclusive, never a fallback.
            Ok(Err(_)) => (false, ""),
            // Corruption or in-canary crash at the applied point.
            Ok(Ok(verdict)) => {
                warn!("sentinel: texture canary detected {verdict:?} at the applied point");
                (true, "silent-canary")
            }
            // Stall: the canary never came back — the GPU is hanging RIGHT NOW.
            Err(_) => {
                warn!(
                    "sentinel: canary STALLED >{SENTINEL_CANARY_STALL_MS} ms — pre-hang; acting \
                     before the driver TDR cascade"
                );
                (true, "pre-hang-canary")
            }
        };
        if failed {
            let n = bumps.load(std::sync::atomic::Ordering::SeqCst);
            // Pre-hang is TDR-grade (+3 bins); corruption is boundary-grade (+2 bins).
            let bins = if kind == "pre-hang-canary" {
                SENTINEL_TDR_BUMP_BINS
            } else {
                SENTINEL_SILENT_BUMP_BINS
            };
            if handle_failure(&store, n, bins, kind) {
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
    fn sentinel_ladder_bumps_three_bins_then_goes_stock() {
        let bins = [850, 856, 862, 868];
        // First failure at 1815@843 → +3 bins = 862, same clock (preserve identity).
        assert_eq!(
            sentinel_decide(Some((1815, 843)), false, 0, &bins, SENTINEL_TDR_BUMP_BINS),
            SentinelAction::Bump { target_mhz: 1815, new_anchor_mv: 862 }
        );
        // Second failure in the same session → ladder exhausted → stock.
        assert_eq!(
            sentinel_decide(Some((1815, 862)), false, 1, &[868], SENTINEL_TDR_BUMP_BINS),
            SentinelAction::Stock { target_mhz: 1815, failed_anchor_mv: 862 }
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
    fn parses_wevtutil_xml_system_time() {
        let xml = "<Event><System><TimeCreated SystemTime='2026-07-09T07:43:40.123456700Z'/></System></Event>";
        assert_eq!(
            parse_event_system_time(xml).as_deref(),
            Some("2026-07-09T07:43:40.123456700Z")
        );
        assert_eq!(parse_event_system_time("no events"), None);
    }
}
