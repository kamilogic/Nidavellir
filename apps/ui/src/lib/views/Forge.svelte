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
  let gameTraceActionError = $state("");
  let gameTraceExportBusy = $state(false);
  let gameTraceExportMsg = $state("");
  let fullResetBusy = $state(false);
  let fullResetFeedback = $state(null);
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
  async function refreshForgeStateAfterReset() {
    const [ps, sl, ap] = await Promise.all([
      serviceCall("GetPowerSweepProgress"),
      serviceCall("GetSafeLoopStatus"),
      serviceCall("GetAppliedProfile"),
    ]);
    const failed = [ps, sl, ap].find((response) => response?.ok === false);
    if (failed) throw new Error(failed.error || "Unable to refresh Forge state");
    if (ps?.data?.type !== "PowerSweep") throw new Error("Invalid Forge status response");
    if (sl?.data?.type !== "SafeLoop") throw new Error("Invalid Safe Loop status response");
    if (ap?.data?.type !== "GpuApply") throw new Error("Invalid applied profile response");
    powerSweep = ps.data;
    safeLoop = sl.data;
    applied = ap.data;
    lastSlowRefreshAt = Date.now();
    await refreshSentinel();
  }

  async function fullResetTuning() {
    if (fullResetBusy) return false;
    fullResetBusy = true;
    fullResetFeedback = null;
    try {
      const response = await serviceCall("ResetGpuTuningFull");
      if (response?.ok === false) {
        throw new Error(response.error || "Unable to reset all GPU learning");
      }
      if (response?.data?.type !== "GpuApply") {
        throw new Error("Invalid full reset response");
      }

      applied = response.data;
      verification = null;
      const message = response.data.message || "Full reset completed";
      const partial = /some state could not be cleared/i.test(message);
      fullResetFeedback = { tone: partial ? "warning" : "success", message };

      try {
        await refreshForgeStateAfterReset();
      } catch (refreshError) {
        fullResetFeedback = {
          tone: "warning",
          message: `${message}. The reset ran, but the UI could not refresh immediately: ${String(refreshError)}`,
        };
      }
      return true;
    } catch (resetError) {
      fullResetFeedback = {
        tone: "error",
        message: `Full reset failed: ${String(resetError)}`,
      };
      return false;
    } finally {
      fullResetBusy = false;
    }
  }
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
      if (r?.ok === false) throw new Error(r.error || "Unable to read Sentinel status");
      if (r?.data?.type === "SentinelStatus") {
        sentinel = r.data.status ? JSON.parse(r.data.status) : null;
      }
    } catch {
      /* sentinel status is best-effort UI info */
    }
  }

  async function refreshGameTrace(clearRecoveredError = true) {
    try {
      const r = await serviceCall("GetGameTraceStatus");
      if (r?.ok === false) throw new Error(r.error || "Unable to read Game Trace status");
      if (r?.data?.type === "GameTrace") {
        gameTrace = r.data;
        if (clearRecoveredError && !gameTraceBusy) gameTraceActionError = "";
      }
    } catch {
      /* game-trace status is best-effort UI info */
    }
  }

  async function toggleGameTrace() {
    if (gameTraceBusy) return;
    gameTraceBusy = true;
    gameTraceActionError = "";
    try {
      const method = gameTrace?.running ? "StopGameTrace" : "StartGameTrace";
      const r = await serviceCall(method);
      if (r?.ok === false) throw new Error(r.error || "Unable to update Game Trace");
      if (r?.data?.type !== "GameTrace") throw new Error("Invalid Game Trace response");
      gameTrace = r.data;
      gameTraceExportMsg = "";
    } catch (e) {
      gameTraceActionError = String(e);
    } finally {
      await refreshGameTrace(false);
      gameTraceBusy = false;
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
    activeView = view === "advanced" || view === "settings" ? view : "forge";
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
    {fullResetBusy}
    {fullResetFeedback}
    {onThemeChange}
    onForgeModeChange={selectForgeMode}
    onStartPower={startPower}
    onStopPower={stopPower}
    onApplyPower={applyPower}
    onFullReset={fullResetTuning}
    onDismissFullResetFeedback={() => (fullResetFeedback = null)}
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
      {gameTraceActionError}
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
