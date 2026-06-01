//! "Forge all" — the full auto pipeline in the correct order:
//!   1. VRAM integrity gate (stock)
//!   2. Core undervolt at a fixed voltage, under combined core+mem load
//!   3. Apply the core profile
//!   4. Memory bandwidth peak, tested at the applied core (combined load)
//!   5. Final whole-package soak (core+mem together) — the real-world judge
//!
//! Order matters: Vcore is the shared rail (shaders + memory controller), so it
//! is fixed first; memory is then tuned against the final core voltage. Every
//! stage uses combined load. Windows-only.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use nidavellir_core::gpu_sweep::VfPoint;
use nidavellir_core::ipc::ForgeAllProgress;
use nidavellir_core::safe_loop::SafeLoopStore;
use tracing::info;

const CORE_VOLTAGE_MV: u32 = 900;

fn idle() -> ForgeAllProgress {
    ForgeAllProgress { running: false, phase: "idle".into(), log: Vec::new(), note: None }
}

#[derive(Clone)]
pub struct ForgeAllHandle {
    progress: Arc<Mutex<ForgeAllProgress>>,
    stop: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
}

impl Default for ForgeAllHandle {
    fn default() -> Self {
        Self {
            progress: Arc::new(Mutex::new(idle())),
            stop: Arc::new(AtomicBool::new(false)),
            running: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl ForgeAllHandle {
    pub fn progress(&self) -> ForgeAllProgress {
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
            run_forge_all(progress, stop, store);
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
struct P {
    progress: Arc<Mutex<ForgeAllProgress>>,
}

#[cfg(windows)]
impl P {
    fn set(&self, phase: &str) {
        if let Ok(mut g) = self.progress.lock() {
            g.phase = phase.into();
        }
    }
    fn log(&self, line: String) {
        info!("forge-all: {line}");
        if let Ok(mut g) = self.progress.lock() {
            g.log.push(line);
        }
    }
    fn note(&self, n: &str) {
        if let Ok(mut g) = self.progress.lock() {
            g.note = Some(n.into());
        }
    }
}

/// Recover the GPU context after a TDR. The driver needs a few seconds to
/// reset; recreating the device immediately fails ("lost during init"), so we
/// wait and retry a few times.
#[cfg(windows)]
fn recover_ctx() -> Option<nidavellir_gpu_stress::GpuCtx> {
    use nidavellir_gpu_stress::GpuCtx;
    for _ in 0..6 {
        std::thread::sleep(std::time::Duration::from_secs(3));
        if let Ok(c) = GpuCtx::new() {
            return Some(c);
        }
    }
    None
}

/// Run a combined core+mem load while sampling the peak core clock.
#[cfg(windows)]
fn combined_clock(
    ctx: &nidavellir_gpu_stress::GpuCtx,
    ms: u64,
) -> (nidavellir_core::gpu_sweep::StabilityResult, u32) {
    use std::panic::{catch_unwind, AssertUnwindSafe};
    let peak = Arc::new(AtomicU32::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let (p2, s2) = (peak.clone(), stop.clone());
    let sampler = std::thread::spawn(move || {
        while !s2.load(Ordering::SeqCst) {
            if let Some(c) = nidavellir_core::nvml_gpu::read_nvidia_gpus_nvml()
                .into_iter()
                .next()
                .and_then(|r| r.core_clock_mhz)
            {
                p2.fetch_max(c, Ordering::SeqCst);
            }
            std::thread::sleep(std::time::Duration::from_millis(120));
        }
    });
    let res = match catch_unwind(AssertUnwindSafe(|| ctx.run_combined(ms).result)) {
        Ok(r) => r,
        Err(_) => nidavellir_core::gpu_sweep::StabilityResult::Crash,
    };
    stop.store(true, Ordering::SeqCst);
    let _ = sampler.join();
    (res, peak.load(Ordering::SeqCst))
}

#[cfg(windows)]
fn run_forge_all(progress: Arc<Mutex<ForgeAllProgress>>, stop: Arc<AtomicBool>, store: SafeLoopStore) {
    use nidavellir_core::gpu_sweep::StabilityResult;
    use nidavellir_gpu_nvapi as gpu;
    use nidavellir_gpu_stress::GpuCtx;
    use std::panic::{catch_unwind, AssertUnwindSafe};

    if let Ok(mut g) = progress.lock() {
        *g = ForgeAllProgress { running: true, phase: "starting".into(), log: Vec::new(), note: None };
    }
    let p = P { progress: progress.clone() };
    let finish = |phase: &str| {
        let _ = gpu::reset_all();
        if let Ok(mut g) = progress.lock() {
            g.running = false;
            g.phase = phase.into();
        }
    };

    let mut ctx = match GpuCtx::new() {
        Ok(c) => c,
        Err(e) => {
            p.log(format!("Falha ao iniciar GPU: {e}"));
            finish("aborted");
            return;
        }
    };
    let _ = gpu::reset_all();

    // 1. VRAM gate ----------------------------------------------------------
    p.set("vram");
    p.log("1/5 · VRAM: testando integridade em stock…".into());
    let vram = match catch_unwind(AssertUnwindSafe(|| ctx.run_vram_check(4 * 1024 * 1024 * 1024, 2))) {
        Ok(r) => r.result,
        Err(_) => StabilityResult::Crash,
    };
    if !vram.is_stable() {
        p.log("VRAM instável em stock — abortando (problema de memória, não de tuning)".into());
        p.note("VRAM falhou no gate — não é seguro tunar.");
        finish("aborted");
        return;
    }
    p.log("VRAM íntegra ✓".into());
    if stop.load(Ordering::SeqCst) {
        finish("aborted");
        return;
    }

    // 2. Core undervolt at fixed voltage, combined load ---------------------
    p.set("core");
    p.log(format!("2/5 · Core: undervolt travado em {CORE_VOLTAGE_MV} mV, carga combinada…"));
    if gpu::lock_core_voltage_mv(CORE_VOLTAGE_MV).is_err() {
        p.log("Falha ao travar voltagem do core — abortando".into());
        finish("aborted");
        return;
    }
    let mut best_clk = 0u32;
    let mut offset = 0i32;
    loop {
        if stop.load(Ordering::SeqCst) {
            finish("aborted");
            return;
        }
        let _ = gpu::set_core_offset_mhz(offset);
        let (res, clk) = combined_clock(&ctx, 6000);
        p.log(format!("   +{offset} MHz → {clk} MHz : {res:?}"));
        match res {
            StabilityResult::Stable => {
                best_clk = clk;
                offset += 15;
                if offset > 300 {
                    break;
                }
            }
            StabilityResult::Crash => {
                // Device lost / TDR. The ctx is dead — recover it so the memory
                // and soak phases run against a live device, not garbage.
                p.log("   device-lost no core — recuperando GPU e recuando".into());
                let _ = gpu::set_core_offset_mhz(0);
                if let Some(fresh) = recover_ctx() {
                    ctx = fresh;
                    p.log("   GPU recuperada ✓".into());
                } else {
                    p.log("   GPU não recuperou — mantendo stock".into());
                }
                break;
            }
            StabilityResult::SilentError => break,
        }
    }
    let _ = gpu::set_core_offset_mhz(0);
    if best_clk == 0 {
        p.log("Core não estabilizou — mantendo stock".into());
    }
    let core_freq = best_clk.saturating_sub(45);
    let core_point = VfPoint { freq_mhz: core_freq, voltage_mv: CORE_VOLTAGE_MV };
    p.log(format!("Core escolhido: {core_freq} MHz @ {CORE_VOLTAGE_MV} mV (margem 45)"));

    // 3. Apply core ---------------------------------------------------------
    p.set("apply-core");
    if core_freq > 0 {
        let _ = crate::gpu_apply::apply_core(core_point);
        p.log("Core aplicado ✓".into());
    }

    // 4. Memory at the applied core, combined load --------------------------
    p.set("memory");
    p.log("4/5 · Memória: pico de banda no core aplicado, carga combinada…".into());
    let mut best_mem = 0i32;
    let mut moff = 50i32;
    loop {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        let _ = gpu::set_mem_offset_mhz(moff);
        let (res, _) = combined_clock(&ctx, 6000);
        let (peak, minbw) = if matches!(res, StabilityResult::Crash) {
            (0.0, 0.0)
        } else {
            ctx.measure_bandwidth_stats(3000)
        };
        let consistent = peak > 0.0 && minbw / peak >= 0.95;
        // Strict bit-error gate: a too-high mem clock can keep bandwidth looking
        // fine (GDDR6 link CRC masks the errors) while silently corrupting data.
        // Verify integrity at this clock — any mismatch fails the offset.
        let integ = if matches!(res, StabilityResult::Crash) {
            StabilityResult::Crash
        } else {
            match catch_unwind(AssertUnwindSafe(|| ctx.run_vram_check(2 * 1024 * 1024 * 1024, 1))) {
                Ok(r) => r.result,
                Err(_) => StabilityResult::Crash,
            }
        };
        let ok = res.is_stable() && integ.is_stable() && consistent;
        p.log(format!(
            "   +{moff} MHz → {peak:.0} GB/s (min {minbw:.0}) · int {} : {}",
            if integ.is_stable() { "ok" } else { "ERRO" },
            if ok { "ok" } else { "instável/queda" }
        ));
        if ok {
            best_mem = moff;
            moff += 50;
            if moff > 1500 {
                break;
            }
        } else {
            // A crash here also kills the device — recover before the soak.
            if matches!(res, StabilityResult::Crash) {
                let _ = gpu::set_mem_offset_mhz(0);
                if let Some(fresh) = recover_ctx() {
                    ctx = fresh;
                }
            }
            break;
        }
    }
    let _ = gpu::set_mem_offset_mhz(0);
    p.log(format!("Memória escolhida: +{best_mem} MHz"));

    // 5. Final whole-package soak ------------------------------------------
    p.set("package-soak");
    p.log("5/5 · Soak final do pacote (core+mem juntos, ~2 min)…".into());
    // The memory sweep may have left the device in a bad state (a hard
    // bandwidth collapse at the cliff) — start this critical validation on a
    // guaranteed-fresh device, recovering with backoff if it was lost.
    match GpuCtx::new() {
        Ok(fresh) => ctx = fresh,
        Err(_) => {
            if let Some(fresh) = recover_ctx() {
                ctx = fresh;
            }
        }
    }
    if core_freq > 0 {
        let _ = crate::gpu_apply::apply_core(core_point);
    }
    let mut mem_final = best_mem;
    let mut soak_ok = false;
    for attempt in 0..3 {
        if mem_final < 0 {
            mem_final = 0;
        }
        let _ = gpu::set_mem_offset_mhz(mem_final);
        let (res, _) = combined_clock(&ctx, 120_000);
        if res.is_stable() {
            soak_ok = true;
            p.log(format!("Pacote estável: core {core_freq}@{CORE_VOLTAGE_MV} + mem +{mem_final} ✓"));
            break;
        }
        p.log(format!("Pacote instável em mem +{mem_final} ({res:?}) — recuando 100 MHz"));
        // A crash kills the device — recover before the next attempt, else every
        // remaining attempt crashes instantly on the dead context.
        if matches!(res, StabilityResult::Crash) {
            let _ = gpu::set_mem_offset_mhz(0);
            if let Some(fresh) = recover_ctx() {
                ctx = fresh;
                let _ = crate::gpu_apply::apply_core(core_point);
            }
        }
        mem_final -= 100;
        if mem_final <= 0 && attempt >= 1 {
            mem_final = 0;
            break;
        }
    }

    // Persist the validated package (re-applied on every boot).
    let label = "Forge All".to_string();
    let core_opt = if core_freq > 0 { Some(core_point) } else { None };
    // Only persist a memory offset that actually survived the package soak —
    // never apply an unvalidated value just because the sweep had found it.
    let mem_opt = if soak_ok && mem_final > 0 { Some(mem_final) } else { None };
    let _ = crate::gpu_apply::apply_and_persist(label, core_opt, mem_opt, &store);
    p.note(if soak_ok {
        "Forja completa e validada — aplicada e persistida. Confirme em jogo."
    } else {
        "Concluído com recuo — perfil conservador aplicado. Confirme em jogo."
    });
    p.log(format!(
        "Aplicado: core {} mem +{}",
        if core_freq > 0 { format!("{core_freq}@{CORE_VOLTAGE_MV}") } else { "stock".into() },
        mem_final.max(0)
    ));

    if let Ok(mut g) = progress.lock() {
        g.running = false;
        g.phase = "done".into();
    }
    info!("forge-all finished (soak_ok={soak_ok})");
}
