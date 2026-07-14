<script>
  import { open } from "@tauri-apps/plugin-shell";
  import { serviceCall } from "../service.js";
  import AdvancedDiagnosticsHub from "../components/forge/AdvancedDiagnosticsHub.svelte";
  import ForgeProgress from "../components/forge/ForgeProgress.svelte";
  import ForgeThemeScreen from "../components/forge/ForgeThemeScreen.svelte";
  import GpuHeroStatus from "../components/forge/GpuHeroStatus.svelte";
  import MonitoringPanel from "../components/forge/MonitoringPanel.svelte";
  import ProfileCards from "../components/forge/ProfileCards.svelte";

  let { theme = "command", onThemeChange } = $props();

  let error = $state(null);
  let timer = $state(null);
  let hardware = $state(null);
  let safeLoop = $state(null);
  let activeView = $state("forge");
  let applied = $state(null);
  let verification = $state(null);
  let exporting = $state(false);
  let exportMsg = $state("");
  let exportFailed = $state(false);
  // v17 sentinel: last automatic action + recommendation (sentinel_status.json via IPC).
  let sentinel = $state(null);
  // Game-trace: read-only NVML/NVAPI workload logger.
  let gameTrace = $state(null);
  let gameTraceBusy = $state(false);
  let gameTraceExportBusy = $state(false);
  let gameTraceExportMsg = $state("");
  // Live GPU telemetry (ReadSensors) + rolling sparkline buffers for the monitoring panel.
  let sensors = $state(null);
  let sparks = $state({ core: [], mem: [], temp: [], power: [], usage: [] });
  const SPARK_CAP = 20;
  let powerSweep = $state(null);
  let forgeMode = $state("standard");
  let hardwareLoaded = $state(false);
  let autoResumeAttempted = $state(false);
  let refreshInFlight = false;
  let lastSlowRefreshAt = 0;

  const powerRunning = $derived(Boolean(powerSweep?.running));
  const hasProfiles = $derived(Boolean(powerSweep?.godforge || powerSweep?.brokkrs || powerSweep?.deep_calm));
  const hasKnowledge = $derived(Boolean(powerSweep?.points?.length || verification?.status));
  const hasForgeRun = $derived(Boolean(powerSweep && powerSweep.phase !== "idle"));

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
        const ap = await serviceCall("GetAppliedProfile");
        applied = ap?.data?.type === "GpuApply" ? ap.data : applied;
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

  async function exportLog() {
    exporting = true;
    exportMsg = "";
    exportFailed = false;
    try {
      const r = await serviceCall("ExportForgeLog");
      if (r?.data?.type === "ForgeLogExport") {
        exportMsg = `${r.data.note} → ${r.data.path}`;
        error = null;
      } else {
        exportFailed = true;
        exportMsg = r?.error ? String(r.error) : "Unable to export the Forge log.";
      }
    } catch (e) {
      exportFailed = true;
      error = String(e);
      exportMsg = String(e);
    } finally {
      exporting = false;
    }
  }

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

  async function refreshGameTrace() {
    try {
      const r = await serviceCall("GetGameTraceStatus");
      if (r?.data?.type === "GameTrace") gameTrace = r.data;
    } catch {
      /* game-trace status is best-effort UI info */
    }
  }

  async function toggleGameTrace() {
    if (gameTraceBusy) return;
    gameTraceBusy = true;
    try {
      const method = gameTrace?.running ? "StopGameTrace" : "StartGameTrace";
      const r = await serviceCall(method);
      if (r?.data?.type === "GameTrace") gameTrace = r.data;
      gameTraceExportMsg = "";
    } catch (e) {
      gameTrace = { ...(gameTrace ?? {}), note: `Error: ${String(e)}` };
    } finally {
      gameTraceBusy = false;
      refreshGameTrace();
    }
  }

  async function openGameTraceLog() {
    if (!gameTrace?.out_path || gameTraceExportBusy) return;
    gameTraceExportBusy = true;
    gameTraceExportMsg = "";
    try {
      await open(gameTrace.out_path);
      gameTraceExportMsg = "The exported JSONL was opened with your default application.";
    } catch (e) {
      gameTraceExportMsg = `Unable to open the exported log: ${String(e)}`;
    } finally {
      gameTraceExportBusy = false;
    }
  }

  function closeAdvancedDiagnostics() {
    changeView("forge");
  }

  function changeView(view) {
    activeView = view === "advanced" ? "advanced" : "forge";
    requestAnimationFrame(() => window.scrollTo({ top: 0, behavior: "auto" }));
  }

  function pushSpark(arr, value) {
    const n = Number(value);
    if (!Number.isFinite(n)) return arr;
    const next = [...arr, n];
    return next.length > SPARK_CAP ? next.slice(next.length - SPARK_CAP) : next;
  }

  async function refreshSensors() {
    try {
      const r = await serviceCall("ReadSensors");
      if (r?.data?.type !== "Sensors") return;
      sensors = r.data;
      const g = sensors.gpu?.[0];
      if (!g) return;
      sparks = {
        core: pushSpark(sparks.core, g.core_clock_mhz),
        mem: pushSpark(sparks.mem, g.memory_clock_mhz),
        temp: pushSpark(sparks.temp, g.temperature_c),
        power: pushSpark(sparks.power, g.power_w),
        usage: pushSpark(sparks.usage, g.utilization_pct),
      };
    } catch {
      /* telemetry is best-effort UI info */
    }
  }

  const primarySensorGpu = $derived(sensors?.gpu?.[0] ?? null);
  const logLines = $derived(powerSweep?.log ?? []);
  const sentinelState = $derived.by(() => {
    if (!sentinel) return "No events";
    if (sentinel.action === "bump") return "Automatic adjustment";
    if (sentinel.action === "stock") return "Returned to stock";
    return "Event recorded";
  });
  const sentinelSummary = $derived.by(() => {
    if (!sentinel) return "No automatic recovery action recorded.";
    if (sentinel.action === "bump") {
      return `Kept ${sentinel.target_mhz} MHz and moved the unstable point from ${sentinel.failed_mv} to ${sentinel.new_mv} mV (strike ${sentinel.strike}/3).`;
    }
    return `Removed ${sentinel.target_mhz} MHz @ ${sentinel.failed_mv} mV and returned the GPU to stock after three failures.`;
  });

  $effect(() => {
    loadHardware();
    refresh();
    refreshSentinel();
    refreshSensors();
    refreshGameTrace();
    timer = setInterval(refresh, 500);
    const sentinelTimer = setInterval(refreshSentinel, 10_000);
    const sensorTimer = setInterval(refreshSensors, 2000);
    const gameTraceTimer = setInterval(refreshGameTrace, 1000);
    return () => {
      clearInterval(timer);
      clearInterval(sentinelTimer);
      clearInterval(sensorTimer);
      clearInterval(gameTraceTimer);
    };
  });
</script>

<section class={`forge theme-${theme}`}>
  <ForgeThemeScreen
    {theme}
    {hardware}
    gpu={primarySensorGpu}
    {sparks}
    {powerSweep}
    {safeLoop}
    {applied}
    {forgeMode}
    {powerRunning}
    {onThemeChange}
    onForgeModeChange={selectForgeMode}
    onStartPower={startPower}
    onStopPower={stopPower}
    onApplyPower={applyPower}
    {activeView}
    onViewChange={changeView}
  >
    <AdvancedDiagnosticsHub
      {theme}
      embedded
      {powerSweep}
      {logLines}
      {exporting}
      {exportMsg}
      {exportFailed}
      {sentinel}
      {sentinelState}
      {sentinelSummary}
      {gameTrace}
      {gameTraceBusy}
      {gameTraceExportBusy}
      {gameTraceExportMsg}
      onExportLog={exportLog}
      onToggleGameTrace={toggleGameTrace}
      onOpenGameTraceLog={openGameTraceLog}
      onClose={closeAdvancedDiagnostics}
    />
  </ForgeThemeScreen>

  <div class="legacy-forge-content">
  <GpuHeroStatus
    {theme}
    {error}
    {applied}
    {hardware}
    {powerSweep}
    {safeLoop}
    {powerRunning}
    {hasProfiles}
    {hasKnowledge}
    {verification}
    {forgeMode}
    onStartPower={startPower}
    onForgeModeChange={selectForgeMode}
    onReset={resetTuning}
    onFullReset={fullResetTuning}
    onRecoverContinue={recoverAndStartPower}
  />

  <MonitoringPanel gpu={primarySensorGpu} {sparks} live={powerRunning} />

  {#if hasForgeRun}
    <ForgeProgress
      {powerSweep}
      {powerRunning}
      {safeLoop}
      onStopPower={stopPower}
    />
  {/if}

  {#if powerRunning}
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
  {/if}

  </div>
</section>

<style>
  .forge {
    display: block;
  }

  .legacy-forge-content {
    display: none;
  }
</style>
