<script>
  import { Activity, CircleCheck, Play, ShieldCheck, Square, Terminal, TriangleAlert } from "@lucide/svelte";
  import { serviceCall } from "../service.js";
  import { t } from "../i18n.js";
  import VfChart from "../components/VfChart.svelte";
  import DiagnosticsPanel from "../components/forge/DiagnosticsPanel.svelte";
  import ForgeKnowledge from "../components/forge/ForgeKnowledge.svelte";
  import ForgeProgress from "../components/forge/ForgeProgress.svelte";
  import GpuHeroStatus from "../components/forge/GpuHeroStatus.svelte";
  import ProfileCards from "../components/forge/ProfileCards.svelte";
  import RecommendedAction from "../components/forge/RecommendedAction.svelte";
  import VfCurvePanel from "../components/forge/VfCurvePanel.svelte";

  let error = $state(null);
  let timer = $state(null);
  let hardware = $state(null);
  let safeLoop = $state(null);
  let realCurve = $state(null);
  let validation = $state(null);
  let advanced = $state(false);
  let expanded = $state(false);
  let memSweep = $state(null);
  let memPreflight = $state(false);
  let applied = $state(null);
  let verification = $state(null);
  let verifying = $state(false);
  let exporting = $state(false);
  let exportMsg = $state("");
  // v17 sentinel: last automatic action + recommendation (sentinel_status.json via IPC).
  let sentinel = $state(null);
  let benchmark = $state(null);
  let powerSweep = $state(null);
  let forgeMode = $state("standard");
  let hardwareLoaded = $state(false);
  let autoResumeAttempted = $state(false);
  let refreshInFlight = false;
  let lastSlowRefreshAt = 0;

  const powerRunning = $derived(Boolean(powerSweep?.running));
  const memRunning = $derived(Boolean(memSweep?.running));
  const benchRunning = $derived(Boolean(benchmark?.running));
  const hasProfiles = $derived(Boolean(powerSweep?.godforge || powerSweep?.brokkrs || powerSweep?.deep_calm));
  const hasKnowledge = $derived(
    Boolean(powerSweep?.points?.length || validation?.result || verification?.status),
  );

  // Keep diagnostics output pinned to its newest line.
  function autoscroll(node, _dep) {
    const toBottom = () => {
      node.scrollTop = node.scrollHeight;
    };
    toBottom();
    return { update: toBottom };
  }

  function fixed(value, digits = 0) {
    const n = Number(value);
    return Number.isFinite(n) ? n.toFixed(digits) : "0";
  }

  function numeric(value) {
    const n = Number(value);
    return Number.isFinite(n) ? n : null;
  }

  function sameNumber(a, b) {
    return a != null && b != null && Number(a) === Number(b);
  }

  function normalizeProfile(s) {
    return String(s ?? "").toLowerCase().replace(/[^a-z0-9]/g, "");
  }

  const powerProfileSlots = [
    { key: "godforge", label: "Godforge" },
    { key: "brokkrs", label: "Brokkr's Best" },
    { key: "deep_calm", label: "Deep Calm" },
  ];

  const appliedPowerPoint = $derived.by(() => {
    if (!applied?.core || !powerSweep) return null;
    for (const slot of powerProfileSlots) {
      const point = powerSweep?.[slot.key];
      if (!point) continue;
      const labelMatches = normalizeProfile(applied.label) === normalizeProfile(slot.label);
      const profileClock = powerSweep?.is_undervolt
        ? (point.target_clock_mhz ?? point.clock_mhz)
        : point.clock_mhz;
      const clockMatches = sameNumber(applied.core.freq_mhz, profileClock);
      if (labelMatches && clockMatches) return { ...slot, point };
    }
    return null;
  });

  function verificationMatchesAppliedProfile() {
    if (verification?.vf_table_voltage_mv == null || verification?.target_mhz == null || !applied?.core) return false;
    const targetMatches = sameNumber(verification.target_mhz, applied.core.freq_mhz);
    const labelMatches =
      !verification.label ||
      !applied.label ||
      normalizeProfile(verification.label) === normalizeProfile(applied.label);
    return targetMatches && labelMatches;
  }

  function buildCurveOverlay({ targetMhz, anchorMv = null, anchorSource = "none", anchorPrecise = false }) {
    const target = numeric(targetMhz);
    if (target == null) return null;
    const anchor = numeric(anchorMv);
    const hasTrustedAnchor = anchor != null && anchorPrecise;
    return {
      targetMhz: target,
      anchorMv: hasTrustedAnchor ? anchor : null,
      anchorSource,
      anchorPrecise: hasTrustedAnchor,
      showBand: hasTrustedAnchor,
    };
  }

  const curveOverlay = $derived.by(() => {
    if (verificationMatchesAppliedProfile()) {
      return buildCurveOverlay({
        targetMhz: verification.target_mhz,
        anchorMv: verification.vf_table_voltage_mv,
        anchorSource: verification.status === "verified_curve" ? "verified_vf_bin" : "verification_vf_bin",
        anchorPrecise: true,
      });
    }

    if (appliedPowerPoint?.point?.vf_table_voltage_mv != null) {
      return buildCurveOverlay({
        targetMhz: powerSweep?.is_undervolt
          ? (appliedPowerPoint.point.target_clock_mhz ?? appliedPowerPoint.point.clock_mhz)
          : appliedPowerPoint.point.clock_mhz,
        anchorMv: appliedPowerPoint.point.vf_table_voltage_mv,
        anchorSource: "profile_vf_bin",
        anchorPrecise: true,
      });
    }

    if (realCurve?.real && realCurve?.plateau?.voltage_mv != null && realCurve?.plateau?.freq_mhz != null) {
      return buildCurveOverlay({
        targetMhz: realCurve.plateau.freq_mhz,
        anchorMv: realCurve.plateau.voltage_mv,
        anchorSource: "curve_read_plateau",
        anchorPrecise: true,
      });
    }

    if (applied?.core?.freq_mhz != null) {
      return buildCurveOverlay({
        targetMhz: applied.core.freq_mhz,
        anchorSource: "none",
        anchorPrecise: false,
      });
    }

    return null;
  });

  async function loadHardware() {
    if (hardwareLoaded) return;
    try {
      const hw = await serviceCall("DetectHardware");
      hardware = hw?.data?.type === "Hardware" ? hw.data : hardware;
      hardwareLoaded = true;
    } catch {
      hardwareLoaded = true;
    }
  }

  async function refresh(forceSlow = false) {
    if (refreshInFlight) return;
    refreshInFlight = true;
    try {
      const now = Date.now();
      const slowDue = forceSlow || !powerRunning || now - lastSlowRefreshAt >= 3000;
      const [ps, sl] = await Promise.all([
        serviceCall("GetPowerSweepProgress"),
        serviceCall("GetSafeLoopStatus"),
      ]);
      powerSweep = ps?.data?.type === "PowerSweep" ? ps.data : powerSweep;
      safeLoop = sl?.data?.type === "SafeLoop" ? sl.data : safeLoop;
      if (slowDue) {
        const [v, ms, ap, bm] = await Promise.all([
          serviceCall("GetGpuValidation"),
          serviceCall("GetMemSweepProgress"),
          serviceCall("GetAppliedProfile"),
          serviceCall("GetBenchmarkProgress"),
        ]);
        validation = v?.data?.type === "GpuValidation" ? v.data : validation;
        memSweep = ms?.data?.type === "MemSweep" ? ms.data : memSweep;
        applied = ap?.data?.type === "GpuApply" ? ap.data : applied;
        benchmark = bm?.data?.type === "Benchmark" ? bm.data : benchmark;
        lastSlowRefreshAt = now;
      }
      error = null;
      if (powerSweep?.phase === "interrupted" && !powerSweep?.running && !autoResumeAttempted) {
        void autoRecoverInterruptedPower();
      }
    } catch (e) {
      error = String(e);
    } finally {
      refreshInFlight = false;
    }
  }

  async function call(method, set) {
    try {
      const r = await serviceCall(method);
      set(r);
      error = null;
    } catch (e) {
      error = String(e);
    }
  }

  const readRealCurve = () =>
    call("GetGpuCurve", (r) => (realCurve = r?.data?.type === "GpuCurve" ? r.data : realCurve));
  const startValidation = () =>
    call("StartGpuValidation", (r) => (validation = r?.data?.type === "GpuValidation" ? r.data : validation));
  const setMem = (r) => (memSweep = r?.data?.type === "MemSweep" ? r.data : memSweep);
  const startMem = () => {
    memPreflight = false;
    call("StartMemSweep", setMem);
  };
  const stopMem = () => call("StopMemSweep", setMem);
  const setApplied = (r) => (applied = r?.data?.type === "GpuApply" ? r.data : applied);
  const resetTuning = async () => {
    const confirmed = globalThis.confirm?.(
      "Reset GPU tuning latch? This returns the GPU to stock, clears recovery, and preserves learned Forge observations.",
    ) ?? true;
    if (!confirmed) return;
    verification = null;
    await call("ResetGpuTuning", setApplied);
    await refresh();
  };
  const fullResetTuning = async () => {
    const confirmed = globalThis.confirm?.(
      "Full reset will return the GPU to stock AND delete learned Forge observations, legacy GPU knowledge, and the Safe Loop blacklist. Use this only when you want to start this GPU from zero. Continue?",
    ) ?? true;
    if (!confirmed) return;
    verification = null;
    await call("ResetGpuTuningFull", setApplied);
    await refresh();
  };
  const setBench = (r) => (benchmark = r?.data?.type === "Benchmark" ? r.data : benchmark);
  const startBench = () => call("StartBenchmark", setBench);
  const stopBench = () => call("StopBenchmark", setBench);
  const setPower = (r) => (powerSweep = r?.data?.type === "PowerSweep" ? r.data : powerSweep);
  const POWER_START = {
    fast: "StartPowerSweepFast",
    standard: "StartPowerSweep",
    long: "StartPowerSweepLong",
  };
  const selectForgeMode = (mode) => {
    forgeMode = mode;
  };
  const startPower = (mode = forgeMode) =>
    call(POWER_START[mode] ?? POWER_START.standard, setPower);
  const restoredForgeMode = () => {
    const mode = String(powerSweep?.mode ?? "").toLowerCase();
    if (mode === "fast" || mode === "rápida") return "fast";
    if (mode === "long" || mode === "longa") return "long";
    return "standard";
  };
  const autoRecoverInterruptedPower = async () => {
    autoResumeAttempted = true;
    const mode = restoredForgeMode();
    forgeMode = mode;
    verification = null;
    try {
      const reset = await serviceCall("ResetGpuTuning");
      setApplied(reset);
      const started = await serviceCall(POWER_START[mode]);
      setPower(started);
      error = null;
    } catch (e) {
      error = `Automatic Forge recovery failed: ${String(e)}`;
    }
  };
  const recoverAndStartPower = async (mode = forgeMode) => {
    const confirmed = globalThis.confirm?.(
      "Continue Forge from saved learning? Nidavellir will reset the GPU to stock, clear recovery, preserve learned observations, then start the selected Forge mode.",
    ) ?? true;
    if (!confirmed) return;
    autoResumeAttempted = true;
    verification = null;
    try {
      const reset = await serviceCall("ResetGpuTuning");
      setApplied(reset);
      const started = await serviceCall(POWER_START[mode] ?? POWER_START.standard);
      setPower(started);
      error = null;
      await refresh();
    } catch (e) {
      error = String(e);
      await refresh();
    }
  };
  const stopPower = async () => {
    if (!powerSweep?.running || powerSweep?.phase === "stopping") return;
    powerSweep = {
      ...powerSweep,
      phase: "stopping",
      note: "Stopping Forge and restoring stock safely…",
    };
    await call("StopPowerSweep", setPower);
  };
  const POWER_APPLY = {
    godforge: "ApplyPowerGodforge",
    brokkrs: "ApplyPowerBrokkrs",
    deep_calm: "ApplyPowerDeepCalm",
  };
  const applyPower = async (which) => {
    verification = null;
    await call(POWER_APPLY[which], setApplied);
  };

  async function verifyAppliedProfile() {
    verifying = true;
    try {
      const r = await serviceCall("VerifyAppliedProfile");
      verification = r?.data?.type === "ApplyVerification" ? r.data : verification;
      error = null;
    } catch (e) {
      error = String(e);
      verification = { status: "verification_failed", live_curve_match: false, message: String(e) };
    } finally {
      verifying = false;
    }
  }

  async function exportLog() {
    exporting = true;
    exportMsg = "";
    try {
      const r = await serviceCall("ExportForgeLog");
      if (r?.data?.type === "ForgeLogExport") {
        exportMsg = `${r.data.note} → ${r.data.path}`;
        error = null;
      } else {
        exportMsg = r?.error ? String(r.error) : "Falha ao exportar log.";
      }
    } catch (e) {
      error = String(e);
      exportMsg = String(e);
    } finally {
      exporting = false;
    }
  }

  function closeMemPreflight() {
    memPreflight = false;
  }

  function closeExpandedCurve() {
    expanded = false;
  }

  function handleKeydown(event) {
    if (event.key !== "Escape") return;
    if (memPreflight) closeMemPreflight();
    if (expanded) closeExpandedCurve();
  }

  const verificationLabel = $derived.by(() => {
    if (!verification) return "Curve verification: Not checked";
    if (verification.status === "verified_curve") return "Curve verification: Verified";
    if (verification.status === "live_mismatch") return "Curve verification: Mismatch";
    return "Curve verification: Unavailable";
  });

  async function refreshSentinel() {
    try {
      const r = await serviceCall("GetSentinelStatus");
      if (r?.data?.type === "SentinelStatus" && r.data.status) {
        sentinel = JSON.parse(r.data.status);
      }
    } catch {
      /* sentinel status is best-effort UI info */
    }
  }

  $effect(() => {
    loadHardware();
    refresh();
    refreshSentinel();
    timer = setInterval(refresh, 500);
    const sentinelTimer = setInterval(refreshSentinel, 10_000);
    return () => {
      clearInterval(timer);
      clearInterval(sentinelTimer);
    };
  });
</script>

<svelte:window onkeydown={handleKeydown} />

<section class="forge">
  <GpuHeroStatus
    {error}
    {applied}
    {hardware}
    {powerSweep}
    {safeLoop}
    {powerRunning}
    {hasProfiles}
    {hasKnowledge}
    {verification}
    onReset={resetTuning}
    onFullReset={fullResetTuning}
  />

  <ForgeProgress
    {powerSweep}
    {powerRunning}
    {safeLoop}
    onStopPower={stopPower}
  />

  {#if powerRunning}
    <ForgeKnowledge summary compact {powerSweep} {validation} />

    <section class="home-section profile-section active-forging">
      <div>
        <span class="section-kicker">Profile Comparison</span>
        <h3>Profiles being produced</h3>
        <p>Profiles update as forging completes. Existing profiles remain available for reference.</p>
      </div>
      <ProfileCards
        {powerSweep}
        {applied}
        {verification}
        showPlaceholders
        onApplyPower={applyPower}
      />
    </section>
  {:else}
    <RecommendedAction
      {applied}
      {powerSweep}
      {powerRunning}
      {safeLoop}
      {forgeMode}
      onStartPower={startPower}
      onForgeModeChange={selectForgeMode}
      onStopPower={stopPower}
      onReset={resetTuning}
      onFullReset={fullResetTuning}
      onRecoverContinue={recoverAndStartPower}
    />

    <section class="home-section profile-section">
      <div>
        <span class="section-kicker">Profile Comparison</span>
        <h3>Choose how this GPU should behave</h3>
        <p>Brokkr's Best is recommended for most users unless your own forge data points elsewhere.</p>
      </div>
      <ProfileCards
        {powerSweep}
        {applied}
        {verification}
        showPlaceholders
        onApplyPower={applyPower}
      />
    </section>

    <ForgeKnowledge summary {powerSweep} {validation} />
  {/if}

  {#if sentinel}
    <div class="sentinel-card" class:stock={sentinel.action === "stock"}>
      <TriangleAlert size={17} strokeWidth={1.85} />
      <div class="sentinel-body">
        <strong>
          Sentinela ·
          {#if sentinel.action === "bump"}
            instabilidade em {sentinel.target_mhz} MHz — rebaixado {sentinel.failed_mv}→{sentinel.new_mv} mV (strike {sentinel.strike}/3)
          {:else}
            {sentinel.target_mhz} MHz @ {sentinel.failed_mv} mV removido — GPU em stock (3 falhas)
          {/if}
        </strong>
        <small>{sentinel.recommendation}</small>
        <small class="sentinel-ts">{sentinel.ts}</small>
      </div>
    </div>
  {/if}

  <details class="advanced-diagnostics">
    <summary>
      <span class="summary-title">
        <Terminal size={17} strokeWidth={1.85} />
        <span>Advanced Diagnostics</span>
      </span>
      <small>Curve checks, validation, benchmark, applied-profile verification and experimental memory diagnostics</small>
    </summary>

    <div class="diagnostic-stack">
      <section class="diagnostic-group">
        <div class="diagnostic-group-head">
          <span class="section-kicker">Current safe diagnostics</span>
          <h4>
            <Activity size={16} strokeWidth={1.85} />
            <span>Inspect and verify the current GPU state</span>
          </h4>
          <p>These actions are explicit diagnostics. They do not replace the primary Forge GPU path.</p>
        </div>

        <VfCurvePanel
          {realCurve}
          {validation}
          {curveOverlay}
          bind:advanced
          bind:expanded
          onReadRealCurve={readRealCurve}
          onStartValidation={startValidation}
        />

        <DiagnosticsPanel
          {benchmark}
          {benchRunning}
          {applied}
          onStartBench={startBench}
          onStopBench={stopBench}
        />

        <section class="diagnostic-card">
          <div>
            <h4 class="section-head">
              <Terminal size={14} strokeWidth={1.85} />
              <span>Export forge log</span>
            </h4>
            <p class="sub">Writes a rich, human-readable log of the last run — profiles, frontier, live log, and every recorded dwell — to a file under the data dir. Read-only; captures no hardware action.</p>
          </div>
          <button class="btn" onclick={exportLog} disabled={exporting}>
            <Terminal size={15} strokeWidth={1.9} />
            <span>{exporting ? "Exporting..." : "Export forge log"}</span>
          </button>
          {#if exportMsg}
            <p class="point accent">
              <CircleCheck size={14} strokeWidth={1.9} />
              <span>{exportMsg}</span>
            </p>
          {/if}
        </section>

        <section class="diagnostic-card">
          <div>
            <h4 class="section-head">
              <ShieldCheck size={14} strokeWidth={1.85} />
              <span>Verify applied profile</span>
            </h4>
            <p class="sub">Read-only check: compares the live modern VF curve against the applied profile. It does not apply or re-apply tuning.</p>
          </div>
          <button class="btn go" onclick={verifyAppliedProfile} disabled={verifying || !applied?.core}>
            <ShieldCheck size={15} strokeWidth={1.9} />
            <span>{verifying ? "Verifying..." : "Verify applied profile"}</span>
          </button>
          <p
            class="point"
            class:accent={verification?.status === "verified_curve"}
            class:danger={verification?.status === "live_mismatch"}
          >
            {#if verification?.status === "verified_curve"}
              <CircleCheck size={14} strokeWidth={1.9} />
            {:else if verification?.status === "live_mismatch"}
              <TriangleAlert size={14} strokeWidth={1.9} />
            {:else}
              <ShieldCheck size={14} strokeWidth={1.9} />
            {/if}
            <span>{verificationLabel}</span>
          </p>
          {#if verification?.message}
            <p class="sub">{verification.message}</p>
          {/if}
          {#if verification?.load_state}
            <p class="sub">Stored load evidence: {verification.load_state}</p>
          {/if}
        </section>
      </section>

      <section class="diagnostic-group future">
        <div class="diagnostic-group-head">
          <span class="section-kicker">Future / experimental pipeline steps</span>
          <h4>
            <Activity size={16} strokeWidth={1.85} />
            <span>VRAM optimization is not part of the current Forge GPU pipeline yet</span>
          </h4>
          <p>Memory tuning must come after the core VF curve is forged and validated. This tool is experimental diagnostics only.</p>
        </div>

        <section class="diagnostic-card">
          <div class="real-head">
            <div>
              <h4 class="section-head">Memory sweep (experimental)</h4>
              <p class="sub">Future pipeline-related diagnostic. It is not a primary product action and does not define the current Forge GPU path.</p>
            </div>
            {#if memRunning}
              <button class="btn stop" onclick={stopMem}>
                <Square size={14} strokeWidth={1.9} />
                <span>Stop memory sweep</span>
              </button>
            {:else}
              <button class="btn" onclick={() => (memPreflight = true)}>
                <Play size={15} strokeWidth={1.9} />
                <span>Run memory sweep (experimental)</span>
              </button>
            {/if}
          </div>

          {#if memSweep && memSweep.phase !== "idle"}
            <div class="terminal">
              <div class="term-head">
                <span class="dots"><i></i><i></i><i></i></span>
                <span class="term-title">nidavellir / memory sweep experimental</span>
                <span class="term-status" class:live={memRunning}>{memRunning ? "running" : "done"}</span>
              </div>
              <div class="term-body" use:autoscroll={(memSweep.points?.length ?? 0) + (memRunning ? 1 : 0)}>
                <div class="tline base"><span class="gutter">--</span><span class="tlead">base / {fixed(memSweep.baseline_gbps)} GB/s</span></div>
                {#each memSweep.points as p, i}
                  <div class="tline">
                    <span class="gutter">{(i + 1).toString().padStart(2, "0")}</span>
                    <span class="tlead">+{p.offset_mhz} MHz / {p.mem_mhz} MHz</span>
                    <span class="tval" class:accent={p.stable} class:danger={!p.stable}>{fixed(p.bandwidth_gbps)} GB/s</span>
                    {#if p.min_gbps > 0}<span class="tmin">min {fixed(p.min_gbps)}</span>{/if}
                    <span class="tstatus" class:danger={!p.stable}>{p.stable ? "ok" : "failed"}</span>
                  </div>
                {/each}
                {#if memRunning}
                  <div class="tline running">
                    <span class="gutter">&gt;</span>
                    <span class="cursor"></span>
                    <span class="tlead">{memSweep.validation_note ?? "..."}</span>
                  </div>
                {/if}
              </div>
            </div>
            {#if memSweep.peak_gbps > 0}
              <p class="point accent">
                {$t("forge.peakResult", { o: memSweep.peak_offset_mhz, g: fixed(memSweep.peak_gbps) })}
              </p>
              {#if !memRunning && memSweep.validation_note}<p class="sub">{memSweep.validation_note}</p>{/if}
            {/if}
          {/if}
        </section>
      </section>
    </div>
  </details>
</section>

{#if memPreflight}
  <div class="overlay" onclick={closeMemPreflight} role="presentation">
    <div
      class="modal"
      onclick={(e) => e.stopPropagation()}
      onkeydown={(e) => e.stopPropagation()}
      role="dialog"
      aria-modal="true"
      aria-labelledby="memory-sweep-dialog-title"
      tabindex="-1"
    >
      <div class="modal-head"><strong id="memory-sweep-dialog-title">Memory sweep (experimental)</strong></div>
      <p class="pre-body">This experimental diagnostic writes memory clocks and is not part of the current Forge GPU pipeline. VRAM optimization is planned for a later pipeline step after the core VF curve is forged and validated.</p>
      <div class="pre-actions">
        <button class="btn ghost" onclick={closeMemPreflight}>{$t("forge.preCancel")}</button>
        <button class="btn go" onclick={startMem}>
          <Play size={15} strokeWidth={1.9} />
          <span>Run memory sweep (experimental)</span>
        </button>
      </div>
    </div>
  </div>
{/if}

{#if expanded && realCurve?.real}
  <div class="overlay" onclick={closeExpandedCurve} role="presentation">
    <div
      class="modal"
      onclick={(e) => e.stopPropagation()}
      onkeydown={(e) => e.stopPropagation()}
      role="dialog"
      aria-modal="true"
      aria-labelledby="vf-curve-dialog-title"
      tabindex="-1"
    >
      <div class="modal-head">
        <strong id="vf-curve-dialog-title">{realCurve.name}</strong>
        <button class="btn ghost" onclick={closeExpandedCurve}>{$t("forge.close")}</button>
      </div>
      <VfChart points={realCurve.points} overlay={curveOverlay} height={560} />
    </div>
  </div>
{/if}

<style>
  .forge {
    --surface: var(--forge-panel);
    --border: var(--forge-line);
    --muted: var(--nord-mist);
    --text: var(--nord-silver);
    --accent: var(--nord-aurora);
    display: flex;
    flex-direction: column;
    gap: 1.25rem;
  }
  .home-section,
  .advanced-diagnostics {
    background: var(--forge-panel-bg);
    border: 1px solid var(--forge-line);
    border-radius: 12px;
    padding: 1rem 1.1rem;
    box-shadow: var(--forge-panel-edge);
  }
  .home-section {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }
  .section-kicker {
    display: block;
    font-size: 0.68rem;
    font-weight: 700;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--nord-dim);
    margin-bottom: 0.35rem;
  }
  .home-section h3 {
    margin: 0;
    color: var(--text);
    font-size: 1rem;
  }
  .home-section p {
    margin: 0.35rem 0 0;
    color: var(--muted);
    font-size: 0.86rem;
    line-height: 1.5;
  }
  .profile-section.active-forging {
    background: rgba(14, 18, 24, 0.58);
    border-color: rgba(255, 255, 255, 0.045);
    box-shadow: none;
  }
  .profile-section.active-forging h3 {
    color: var(--nord-mist);
  }
  .profile-section.active-forging :global(.profiles) {
    opacity: 0.88;
  }
  .advanced-diagnostics > summary {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) auto;
    gap: 0.7rem;
    align-items: center;
    cursor: pointer;
    color: var(--text);
    font-weight: 700;
    list-style: none;
  }
  .advanced-diagnostics > summary::-webkit-details-marker {
    display: none;
  }
  .advanced-diagnostics > summary::after {
    content: "+";
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.55rem;
    height: 1.55rem;
    border: 1px solid var(--forge-line);
    border-radius: 999px;
    color: var(--forge-gold);
    background: rgba(214, 168, 93, 0.08);
    font-weight: 800;
  }
  .advanced-diagnostics[open] > summary::after {
    content: "-";
  }
  .advanced-diagnostics > summary span,
  .advanced-diagnostics > summary small {
    display: block;
  }
  .advanced-diagnostics > summary .summary-title {
    display: inline-flex;
    align-items: center;
    gap: 0.42rem;
  }
  .advanced-diagnostics > summary .summary-title span {
    display: inline;
  }
  .advanced-diagnostics > summary small {
    color: var(--muted);
    font-size: 0.78rem;
    font-weight: 500;
    line-height: 1.4;
  }
  .diagnostic-stack {
    display: flex;
    flex-direction: column;
    gap: 1rem;
    margin-top: 1rem;
    padding-top: 1rem;
    border-top: 1px solid var(--forge-line);
  }
  .diagnostic-group,
  .diagnostic-card {
    border: 1px solid rgba(255, 255, 255, 0.055);
    border-radius: 10px;
    background: rgba(5, 7, 11, 0.22);
  }
  .diagnostic-group {
    padding: 0.9rem;
  }
  .diagnostic-card {
    margin-top: 1rem;
    padding: 0.85rem;
  }
  .diagnostic-group.future {
    border-style: dashed;
  }
  .diagnostic-group-head {
    margin-bottom: 0.85rem;
  }
  .diagnostic-group-head h4 {
    display: inline-flex;
    align-items: center;
    gap: 0.42rem;
    margin: 0;
    color: var(--text);
    font-size: 0.95rem;
  }
  .diagnostic-group-head p {
    margin: 0.35rem 0 0;
    color: var(--muted);
    font-size: 0.84rem;
    line-height: 1.5;
  }
  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 0.42rem;
    border: 1px solid var(--border);
    border-radius: 9px;
    padding: 0.55rem 1.1rem;
    font-weight: 600;
    font-size: 0.85rem;
    cursor: pointer;
    background: rgba(8, 11, 16, 0.66);
    color: var(--text);
    transition:
      border-color 0.15s ease,
      color 0.15s ease,
      background 0.15s ease;
  }
  .btn:hover {
    border-color: var(--forge-line-strong);
    color: var(--forge-gold);
  }
  .btn.go {
    background: rgba(214, 168, 93, 0.13);
    color: var(--forge-gold);
    border-color: rgba(214, 168, 93, 0.42);
  }
  .btn.stop {
    background: rgba(191, 97, 106, 0.16);
    color: #f3b9bd;
    border-color: rgba(191, 97, 106, 0.45);
  }
  .btn.ghost {
    background: transparent;
    color: var(--muted);
  }
  .btn:disabled {
    opacity: 0.55;
    cursor: default;
  }
  .real-head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 1rem;
  }
  .section-head {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    margin: 0 0 0.5rem;
    font-size: 0.72rem;
    font-weight: 700;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--muted);
  }
  .point {
    display: inline-flex;
    align-items: center;
    gap: 0.38rem;
    margin: 0.45rem 0 0;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    color: var(--text);
  }
  .point.accent {
    color: var(--accent);
  }
  .point.danger {
    color: var(--nord-danger);
  }
  .sub {
    margin: 0.25rem 0;
    font-size: 0.82rem;
    color: var(--muted);
    line-height: 1.45;
  }
  .overlay {
    --surface: var(--forge-panel);
    --border: var(--forge-line);
    --muted: var(--nord-mist);
    --text: var(--nord-silver);
    --accent: var(--nord-aurora);
    position: fixed;
    inset: 0;
    background: rgba(4, 6, 12, 0.78);
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 2rem;
    z-index: 50;
  }
  .modal {
    background: var(--forge-panel-bg);
    border: 1px solid var(--forge-line);
    border-radius: 14px;
    padding: 1.1rem;
    width: min(1100px, 95vw);
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.5);
  }
  .modal-head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 0.6rem;
    color: var(--text);
  }
  .pre-body {
    color: var(--muted);
    font-size: 0.9rem;
    line-height: 1.55;
    margin: 0 0 1rem;
  }
  .pre-actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.6rem;
  }
  .terminal {
    font-family: "Cascadia Code", "Consolas", ui-monospace, monospace;
    font-size: 0.8rem;
    background: rgba(5, 7, 11, 0.92);
    border: 1px solid var(--forge-line);
    border-radius: 10px;
    overflow: hidden;
    box-shadow: inset 0 0 0 1px rgba(214, 168, 93, 0.04), 0 8px 24px rgba(0, 0, 0, 0.35);
  }
  .term-head {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    padding: 0.4rem 0.7rem;
    background: rgba(214, 168, 93, 0.055);
    border-bottom: 1px solid var(--forge-line);
  }
  .dots {
    display: inline-flex;
    gap: 0.32rem;
  }
  .dots i {
    width: 0.62rem;
    height: 0.62rem;
    border-radius: 50%;
    background: var(--nord-dim);
    opacity: 0.6;
  }
  .dots i:nth-child(1) {
    background: var(--nord-danger);
  }
  .dots i:nth-child(2) {
    background: var(--nord-ember-bright);
  }
  .dots i:nth-child(3) {
    background: var(--nord-aurora);
  }
  .term-title {
    color: var(--nord-mist);
    font-size: 0.74rem;
    letter-spacing: 0.04em;
  }
  .term-status {
    margin-left: auto;
    font-size: 0.68rem;
    text-transform: lowercase;
    color: var(--nord-dim);
    padding: 0.08rem 0.5rem;
    border-radius: 999px;
    border: 1px solid var(--border);
  }
  .term-status.live {
    color: var(--nord-ember-bright);
    border-color: rgba(235, 203, 139, 0.4);
    background: rgba(235, 203, 139, 0.08);
  }
  .term-body {
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
    padding: 0.55rem 0.7rem;
    max-height: 340px;
    overflow-y: auto;
    scroll-behavior: smooth;
  }
  .term-body::-webkit-scrollbar {
    width: 8px;
  }
  .term-body::-webkit-scrollbar-thumb {
    background: rgba(214, 168, 93, 0.18);
    border-radius: 8px;
  }
  .tline {
    display: flex;
    align-items: baseline;
    gap: 0.75rem;
    padding: 0.12rem 0;
    color: var(--muted);
    font-variant-numeric: tabular-nums;
    border-radius: 4px;
  }
  .gutter {
    color: var(--nord-dim);
    opacity: 0.55;
    min-width: 1.4rem;
    text-align: right;
    user-select: none;
    flex-shrink: 0;
  }
  .tline.base {
    color: var(--nord-dim);
    border-bottom: 1px solid var(--border);
    padding-bottom: 0.3rem;
    margin-bottom: 0.2rem;
  }
  .tlead {
    min-width: 16rem;
    color: var(--text);
  }
  .cursor {
    display: inline-block;
    width: 0.5rem;
    height: 0.85rem;
    background: var(--nord-ember-bright);
    align-self: center;
    animation: blink 1s steps(2, start) infinite;
    flex-shrink: 0;
  }
  @keyframes blink {
    50% {
      opacity: 0;
    }
  }
  .tval {
    min-width: 5rem;
    text-align: right;
  }
  .tval.accent {
    color: var(--accent);
  }
  .tval.danger,
  .tstatus.danger {
    color: var(--nord-danger);
  }
  .tmin {
    min-width: 5rem;
    text-align: right;
    color: var(--nord-dim);
    font-size: 0.72rem;
  }
  .tstatus {
    color: var(--nord-aurora);
    font-size: 0.72rem;
    opacity: 0.8;
  }
  .tline.running {
    color: var(--nord-ember-bright);
  }
  @media (max-width: 760px) {
    .advanced-diagnostics > summary {
      grid-template-columns: minmax(0, 1fr) auto;
    }
    .advanced-diagnostics > summary small {
      grid-column: 1 / -1;
    }
    .advanced-diagnostics > summary::after {
      grid-column: 2;
      grid-row: 1;
      justify-self: end;
    }
    .real-head,
    .pre-actions {
      align-items: stretch;
      flex-direction: column;
    }
    .btn {
      width: fit-content;
    }
  }
  .sentinel-card {
    display: flex;
    gap: 0.65rem;
    align-items: flex-start;
    padding: 0.75rem 0.9rem;
    border-radius: 10px;
    border: 1px solid color-mix(in srgb, orange 45%, transparent);
    background: color-mix(in srgb, orange 10%, transparent);
    margin-top: 0.75rem;
  }
  .sentinel-card.stock {
    border-color: color-mix(in srgb, crimson 45%, transparent);
    background: color-mix(in srgb, crimson 10%, transparent);
  }
  .sentinel-body {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
  }
  .sentinel-ts {
    opacity: 0.6;
    font-size: 0.72rem;
  }
</style>
