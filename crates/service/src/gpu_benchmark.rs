//! Before/after GPU benchmark: run a fixed, repeatable battery at **stock** and
//! again with the **applied profile**, then report the real deltas (FPS, sustained
//! clock, power, temp, perf/watt, memory bandwidth) and whether the card was
//! power-capped. On a power-limited card this quantifies exactly how much
//! headroom the undervolt reclaimed under the power cap.
//!
//! Windows-only (NVAPI/NVML). Read-mostly: it does reset→apply around the runs,
//! and leaves the tuned profile applied at the end.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use nidavellir_core::ipc::{BenchSnapshot, BenchmarkProgress};
use nidavellir_core::safe_loop::SafeLoopStore;
use tracing::info;

fn idle() -> BenchmarkProgress {
    BenchmarkProgress {
        running: false,
        phase: "idle".into(),
        log: Vec::new(),
        stock: None,
        tuned: None,
        power_limit_w: 0.0,
        note: None,
    }
}

#[derive(Clone)]
pub struct BenchmarkHandle {
    progress: Arc<Mutex<BenchmarkProgress>>,
    stop: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
}

impl Default for BenchmarkHandle {
    fn default() -> Self {
        Self {
            progress: Arc::new(Mutex::new(idle())),
            stop: Arc::new(AtomicBool::new(false)),
            running: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl BenchmarkHandle {
    pub fn progress(&self) -> BenchmarkProgress {
        self.progress.lock().map(|p| p.clone()).unwrap_or_else(|_| idle())
    }
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }
    pub fn start(&self, store: SafeLoopStore) -> bool {
        if self.running.swap(true, Ordering::SeqCst) {
            return false;
        }
        self.stop.store(false, Ordering::SeqCst);
        let progress = Arc::clone(&self.progress);
        let stop = Arc::clone(&self.stop);
        let running = Arc::clone(&self.running);
        std::thread::spawn(move || {
            #[cfg(windows)]
            run_benchmark(progress, stop, store);
            #[cfg(not(windows))]
            {
                let _ = (&progress, &stop, &store);
            }
            running.store(false, Ordering::SeqCst);
        });
        true
    }
}

#[cfg(windows)]
fn set(progress: &Arc<Mutex<BenchmarkProgress>>, p: BenchmarkProgress) {
    if let Ok(mut g) = progress.lock() {
        *g = p;
    }
}

/// NVML sample accumulator over a workload window.
#[cfg(windows)]
#[derive(Default)]
struct Sampled {
    clock_sum: u64,
    power_mw_sum: u64,
    n: u64,
    max_temp_c: u32,
    capped: u64,
}

/// Run `body` while sampling NVML at ~150 ms (peak temp, mean clock/power, and
/// the fraction of time the card was power-capped).
#[cfg(windows)]
fn sample_during<R>(body: impl FnOnce() -> R) -> (R, Sampled) {
    let stop = Arc::new(AtomicBool::new(false));
    let clk = Arc::new(AtomicU64::new(0));
    let pw = Arc::new(AtomicU64::new(0));
    let n = Arc::new(AtomicU64::new(0));
    let temp = Arc::new(AtomicU32::new(0));
    let capped = Arc::new(AtomicU64::new(0));
    let (s2, c2, p2, n2, t2, cap2) =
        (stop.clone(), clk.clone(), pw.clone(), n.clone(), temp.clone(), capped.clone());
    let sampler = std::thread::spawn(move || {
        while !s2.load(Ordering::SeqCst) {
            if let Some(r) = nidavellir_core::nvml_gpu::read_nvidia_gpus_nvml().into_iter().next() {
                if let Some(c) = r.core_clock_mhz {
                    c2.fetch_add(c as u64, Ordering::SeqCst);
                    if let Some(p) = r.power_w {
                        p2.fetch_add((p * 1000.0) as u64, Ordering::SeqCst);
                    }
                    n2.fetch_add(1, Ordering::SeqCst);
                }
                if let Some(t) = r.temperature_c {
                    t2.fetch_max(t as u32, Ordering::SeqCst);
                }
                if r.power_capped == Some(true) {
                    cap2.fetch_add(1, Ordering::SeqCst);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(150));
        }
    });
    let r = body();
    stop.store(true, Ordering::SeqCst);
    let _ = sampler.join();
    let s = Sampled {
        clock_sum: clk.load(Ordering::SeqCst),
        power_mw_sum: pw.load(Ordering::SeqCst),
        n: n.load(Ordering::SeqCst),
        max_temp_c: temp.load(Ordering::SeqCst),
        capped: capped.load(Ordering::SeqCst),
    };
    (r, s)
}

/// One benchmark pass: warm up, run the render battery (FPS) while sampling, then
/// measure memory bandwidth. `ctx` is rebuilt by the caller between passes.
#[cfg(windows)]
fn bench_pass(
    ctx: &nidavellir_gpu_stress::GpuCtx,
    stop: &Arc<AtomicBool>,
) -> BenchSnapshot {
    use std::panic::{catch_unwind, AssertUnwindSafe};

    // Warm up to thermal/clock equilibrium (~6 s), discarded.
    let warm = std::time::Instant::now() + std::time::Duration::from_secs(6);
    while std::time::Instant::now() < warm && !stop.load(Ordering::SeqCst) {
        let _ = catch_unwind(AssertUnwindSafe(|| ctx.run_render_stress(800)));
    }

    // Render battery (~12 s) — FPS is the perf metric; sample clock/power/temp.
    let (render, s) = sample_during(|| {
        catch_unwind(AssertUnwindSafe(|| ctx.run_render_stress(12_000)))
            .map(|r| r.fps)
            .unwrap_or(0.0)
    });
    let fps = render;

    // Memory bandwidth (~4 s).
    let bandwidth_gbps = catch_unwind(AssertUnwindSafe(|| ctx.measure_bandwidth_stats(4_000)))
        .map(|(peak, _min)| peak)
        .unwrap_or(0.0);

    let avg_power_w = if s.n > 0 { (s.power_mw_sum as f32 / s.n as f32) / 1000.0 } else { 0.0 };
    let avg_clock_mhz = if s.n > 0 { (s.clock_sum / s.n) as u32 } else { 0 };
    let perf_per_watt = if avg_power_w > 0.0 { fps / avg_power_w as f64 } else { 0.0 };
    let power_capped_frac = if s.n > 0 { s.capped as f32 / s.n as f32 } else { 0.0 };

    BenchSnapshot {
        fps,
        bandwidth_gbps,
        avg_clock_mhz,
        avg_power_w,
        max_temp_c: s.max_temp_c as f32,
        perf_per_watt,
        power_capped_frac,
    }
}

#[cfg(windows)]
fn run_benchmark(
    progress: Arc<Mutex<BenchmarkProgress>>,
    stop: Arc<AtomicBool>,
    store: SafeLoopStore,
) {
    use nidavellir_gpu_nvapi as gpu;
    use nidavellir_gpu_stress::GpuCtx;

    info!("Benchmark starting (before/after)");
    let mut prog = idle();
    prog.running = true;
    prog.phase = "stock".into();
    prog.log.push("Benchmark antes/depois — preparando…".into());
    set(&progress, prog.clone());

    let applied = crate::gpu_apply::load_applied();
    if applied.is_none() {
        prog.running = false;
        prog.phase = "done".into();
        prog.note = Some("Nenhum perfil aplicado para comparar. Forje/aplique um perfil primeiro.".into());
        set(&progress, prog);
        return;
    }

    // ---- STOCK pass --------------------------------------------------------
    prog.log.push("1/2 · Stock: revertendo para fábrica e medindo…".into());
    set(&progress, prog.clone());
    let _ = nidavellir_core::nvml_gpu::reset_core_clock_lock();
    let _ = gpu::reset_all();
    std::thread::sleep(std::time::Duration::from_secs(2));

    let ctx = match GpuCtx::new() {
        Ok(c) => c,
        Err(e) => {
            prog.running = false;
            prog.phase = "done".into();
            prog.note = Some(format!("Falha ao iniciar GPU: {e}"));
            set(&progress, prog);
            return;
        }
    };
    let stock = bench_pass(&ctx, &stop);
    prog.power_limit_w = sample_power_limit();
    prog.stock = Some(stock.clone());
    prog.log.push(format!(
        "   stock: {:.0} fps · {} MHz · {:.0} W · {:.0} GB/s{}",
        stock.fps, stock.avg_clock_mhz, stock.avg_power_w, stock.bandwidth_gbps,
        if stock.power_capped_frac > 0.15 { " · power-cap" } else { "" }
    ));
    set(&progress, prog.clone());
    drop(ctx);

    if stop.load(Ordering::SeqCst) {
        finish_restore(&applied, &store);
        prog.running = false;
        prog.phase = "done".into();
        prog.note = Some("Cancelado — perfil reaplicado.".into());
        set(&progress, prog);
        return;
    }

    // ---- TUNED pass --------------------------------------------------------
    prog.phase = "tuned".into();
    prog.log.push("2/2 · Tuned: aplicando perfil e medindo…".into());
    set(&progress, prog.clone());
    apply_profile(&applied);
    std::thread::sleep(std::time::Duration::from_secs(2));

    let ctx = match GpuCtx::new() {
        Ok(c) => c,
        Err(e) => {
            prog.running = false;
            prog.phase = "done".into();
            prog.note = Some(format!("Falha ao iniciar GPU (tuned): {e}"));
            set(&progress, prog);
            return;
        }
    };
    let tuned = bench_pass(&ctx, &stop);
    prog.tuned = Some(tuned.clone());
    prog.log.push(format!(
        "   tuned: {:.0} fps · {} MHz · {:.0} W · {:.0} GB/s{}",
        tuned.fps, tuned.avg_clock_mhz, tuned.avg_power_w, tuned.bandwidth_gbps,
        if tuned.power_capped_frac > 0.15 { " · power-cap" } else { "" }
    ));
    drop(ctx);

    // ---- Report ------------------------------------------------------------
    let pct = |a: f64, b: f64| if a > 0.0 { (b - a) / a * 100.0 } else { 0.0 };
    let fps_d = pct(stock.fps, tuned.fps);
    let pw_d = pct(stock.avg_power_w as f64, tuned.avg_power_w as f64);
    let ppw_d = pct(stock.perf_per_watt, tuned.perf_per_watt);
    let bw_d = pct(stock.bandwidth_gbps, tuned.bandwidth_gbps);
    let clk_d = tuned.avg_clock_mhz as i64 - stock.avg_clock_mhz as i64;
    let cap_note = if stock.power_capped_frac > 0.15 && tuned.power_capped_frac < stock.power_capped_frac {
        " · saiu do power-cap"
    } else {
        ""
    };
    prog.note = Some(format!(
        "FPS {fps_d:+.0}% · clock {clk_d:+} MHz · potência {pw_d:+.0}% · perf/watt {ppw_d:+.0}% · banda {bw_d:+.0}%{cap_note}"
    ));
    prog.running = false;
    prog.phase = "done".into();
    set(&progress, prog);
    info!("Benchmark finished: fps {fps_d:+.0}%, perf/watt {ppw_d:+.0}%");
}

/// Re-apply the saved profile (used to restore after the stock pass / on cancel).
#[cfg(windows)]
fn apply_profile(applied: &Option<crate::gpu_apply::AppliedProfile>) {
    if let Some(ap) = applied {
        if let Some(c) = ap.core {
            let _ = crate::gpu_apply::apply_core(c);
        }
        if let Some(m) = ap.mem_offset_mhz {
            let _ = nidavellir_gpu_nvapi::set_mem_offset_mhz(m);
        }
    }
}

#[cfg(windows)]
fn finish_restore(applied: &Option<crate::gpu_apply::AppliedProfile>, _store: &SafeLoopStore) {
    apply_profile(applied);
}

/// Read the enforced power limit (W) directly from NVML, 0 if unavailable.
#[cfg(windows)]
fn sample_power_limit() -> f32 {
    nidavellir_core::nvml_gpu::read_nvidia_gpus_nvml()
        .into_iter()
        .next()
        .and_then(|r| r.power_limit_w)
        .unwrap_or(0.0)
}
