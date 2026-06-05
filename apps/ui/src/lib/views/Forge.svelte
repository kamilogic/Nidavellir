<script>
  import { serviceCall } from "../service.js";
  import { t } from "../i18n.js";
  import VfChart from "../components/VfChart.svelte";
  import DiagnosticsPanel from "../components/forge/DiagnosticsPanel.svelte";
  import ForgeKnowledge from "../components/forge/ForgeKnowledge.svelte";
  import ForgeProgress from "../components/forge/ForgeProgress.svelte";
  import GpuHeroStatus from "../components/forge/GpuHeroStatus.svelte";
  import LogTerminal from "../components/forge/LogTerminal.svelte";
  import PowerSweepPanel from "../components/forge/PowerSweepPanel.svelte";
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
  let realSweep = $state(null);
  let preflight = $state(false);
  let memSweep = $state(null);
  let memPreflight = $state(false);
  let applied = $state(null);
  let forge = $state(null);
  let forgePreflight = $state(false);
  let benchmark = $state(null);
  let powerSweep = $state(null);
  let hardwareLoaded = $state(false);
  const forgeRunning = $derived(forge?.running);
  const powerRunning = $derived(powerSweep?.running);

  // Keep a terminal pinned to its newest line (tail -f). The `dep` param makes
  // the action re-run on every appended line so the latest is always in view.
  function autoscroll(node, _dep) {
    const toBottom = () => {
      node.scrollTop = node.scrollHeight;
    };
    toBottom();
    return { update: toBottom };
  }

  const SWEEPING = ["baseline", "vram_diagnostic", "voltage_bisection", "synthesis"];
  const realRunning = $derived(realSweep && SWEEPING.includes(realSweep.phase));
  const memRunning = $derived(memSweep?.running);
  const benchRunning = $derived(benchmark?.running);
  const hasProfiles = $derived(Boolean(realSweep?.profiles || powerSweep?.brokkrs));
  const hasKnowledge = $derived(
    Boolean(realSweep?.tradeoffs?.length || powerSweep?.points?.length || validation?.result || realSweep?.validation_note),
  );

  // The point the chart should flatten the curve at. When a profile is applied
  // the GPU is hard-capped there (clock lock), so the *effective* curve is flat
  // from that voltage on — show THAT, not the silicon curve's natural plateau
  // (which is the uncapped top, e.g. 2175 MHz @ 1075 mV, and is misleading once
  // a lower undervolt limit like 1920 @ 900 is locked in).
  const appliedLimit = $derived(
    applied?.core ? { voltage_mv: applied.core.voltage_mv, freq_mhz: applied.core.freq_mhz } : null,
  );
  const chartLimit = $derived(appliedLimit ?? realCurve?.plateau ?? null);

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

  async function refresh() {
    try {
      const v = await serviceCall("GetGpuValidation");
      validation = v?.data?.type === "GpuValidation" ? v.data : validation;
      const rs = await serviceCall("GetRealSweepProgress");
      realSweep = rs?.data?.type === "GpuSweep" ? rs.data : realSweep;
      const ms = await serviceCall("GetMemSweepProgress");
      memSweep = ms?.data?.type === "MemSweep" ? ms.data : memSweep;
      const ap = await serviceCall("GetAppliedProfile");
      applied = ap?.data?.type === "GpuApply" ? ap.data : applied;
      const fa = await serviceCall("GetForgeAllProgress");
      forge = fa?.data?.type === "ForgeAll" ? fa.data : forge;
      const bm = await serviceCall("GetBenchmarkProgress");
      benchmark = bm?.data?.type === "Benchmark" ? bm.data : benchmark;
      const ps = await serviceCall("GetPowerSweepProgress");
      powerSweep = ps?.data?.type === "PowerSweep" ? ps.data : powerSweep;
      const sl = await serviceCall("GetSafeLoopStatus");
      safeLoop = sl?.data?.type === "SafeLoop" ? sl.data : safeLoop;
      error = null;
    } catch (e) {
      error = String(e);
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
  const setReal = (r) => (realSweep = r?.data?.type === "GpuSweep" ? r.data : realSweep);
  const startReal = (method) => {
    preflight = false;
    call(method, setReal);
  };
  const stopReal = () => call("StopRealSweep", setReal);
  const setMem = (r) => (memSweep = r?.data?.type === "MemSweep" ? r.data : memSweep);
  const startMem = () => {
    memPreflight = false;
    call("StartMemSweep", setMem);
  };
  const stopMem = () => call("StopMemSweep", setMem);
  const setApplied = (r) => (applied = r?.data?.type === "GpuApply" ? r.data : applied);
  const CORE_APPLY = ["ApplyGodforge", "ApplyBrokkrs", "ApplyDeepCalm"];
  const applyCore = (i) => call(CORE_APPLY[i], setApplied);
  const applyMem = () => call("ApplyMemPeak", setApplied);
  const resetTuning = () => call("ResetGpuTuning", setApplied);
  const setForge = (r) => (forge = r?.data?.type === "ForgeAll" ? r.data : forge);
  const startForge = () => {
    forgePreflight = false;
    call("StartForgeAll", setForge);
  };
  const stopForge = () => call("StopForgeAll", setForge);
  const setBench = (r) => (benchmark = r?.data?.type === "Benchmark" ? r.data : benchmark);
  const startBench = () => call("StartBenchmark", setBench);
  const stopBench = () => call("StopBenchmark", setBench);
  const setPower = (r) => (powerSweep = r?.data?.type === "PowerSweep" ? r.data : powerSweep);
  const startPower = () => call("StartPowerSweep", setPower);
  const stopPower = () => call("StopPowerSweep", setPower);
  const POWER_APPLY = {
    godforge: "ApplyPowerGodforge",
    brokkrs: "ApplyPowerBrokkrs",
    deep_calm: "ApplyPowerDeepCalm",
  };
  const applyPower = (which) => call(POWER_APPLY[which], setApplied);
  const applyRecommended = () => {
    if (realSweep?.profiles?.brokkrs_best) {
      applyCore(1);
    } else if (powerSweep?.brokkrs) {
      applyPower("brokkrs");
    }
  };

  $effect(() => {
    loadHardware();
    refresh();
    timer = setInterval(refresh, 500);
    return () => clearInterval(timer);
  });
</script>

<section class="forge">
  <GpuHeroStatus
    {error}
    {applied}
    {hardware}
    {safeLoop}
    {forgeRunning}
    {realRunning}
    {memRunning}
    {powerRunning}
    {hasProfiles}
    {hasKnowledge}
    onReset={resetTuning}
  />

  <RecommendedAction
    {applied}
    {forge}
    {forgeRunning}
    realProfiles={realSweep?.profiles}
    {powerSweep}
    {safeLoop}
    onStartForge={() => (forgePreflight = true)}
    onStopForge={stopForge}
    onApplyRecommended={applyRecommended}
  />

  <section class="home-section">
    <div>
      <span class="section-kicker">Profile Comparison</span>
      <h3>Choose how this GPU should behave</h3>
      <p>Brokkr's Best is recommended for most users unless your own forge data points elsewhere.</p>
    </div>
    <ProfileCards
      realProfiles={realSweep?.profiles}
      {powerSweep}
      {applied}
      showPlaceholders
      onApplyCore={applyCore}
      onApplyPower={applyPower}
    />
  </section>

  <ForgeKnowledge summary {realSweep} {powerSweep} {validation} />

  <ForgeProgress
    {forge}
    {forgeRunning}
    onRequestStart={() => (forgePreflight = true)}
    onStop={stopForge}
  />

  <details class="advanced-diagnostics">
    <summary>
      <span>Advanced Diagnostics</span>
      <small>Benchmark details, V/F curve, raw sweep tables, logs and validation traces</small>
    </summary>

    <div class="diagnostic-stack">
      {#if forge && forge.phase !== "idle" && (forge.log?.length || forge.running)}
        <section class="diagnostic-block">
          <h4 class="section-head">Forge log</h4>
          <LogTerminal
            title="nidavellir / forge"
            status={forge.running ? forge.phase : "done"}
            live={forge.running}
            lines={forge.log ?? []}
            runningText={forge.running ? `${forge.phase}...` : null}
          />
        </section>
      {/if}

      <DiagnosticsPanel
        {benchmark}
        {benchRunning}
        {applied}
        onStartBench={startBench}
        onStopBench={stopBench}
      />

      <PowerSweepPanel
        {powerSweep}
        {powerRunning}
        onStartPower={startPower}
        onStopPower={stopPower}
        onApplyPower={applyPower}
        {applied}
      />

  <div class="section real">
    <VfCurvePanel
      {realCurve}
      {validation}
      {chartLimit}
      {appliedLimit}
      bind:advanced
      bind:expanded
      onReadRealCurve={readRealCurve}
      onStartValidation={startValidation}
    />

    <div class="realsweep">
      <div class="real-head">
        <h4 class="section-head">{$t("forge.realSweep")}</h4>
        {#if realRunning}
          <button class="btn stop" onclick={stopReal}>{$t("forge.stopReal")}</button>
        {:else}
          <button class="btn go" onclick={() => (preflight = true)}>{$t("forge.runReal")}</button>
        {/if}
      </div>

      {#if realSweep && realSweep.phase !== "idle"}
        <div class="grid">
          <article class="tile">
            <span class="lab">{$t("forge.phase")}</span>
            <p class="val">{$t("phase." + realSweep.phase)}</p>
          </article>
          <article class="tile">
            <span class="lab">{$t("forge.voltageIdx")}</span>
            <p class="val">{realSweep.freq_index} / {realSweep.total_freqs}</p>
          </article>
          <article class="tile">
            <span class="lab">{$t("forge.testingNow")}</span>
            {#if realSweep.current}
              <p class="val" class:accent={realSweep.last_result === "stable"} class:danger={realSweep.last_result && realSweep.last_result !== "stable"}>
                {realSweep.current.freq_mhz} MHz @ {realSweep.current.voltage_mv} mV
              </p>
              <p class="sub">
                {#if realSweep.gpu_temp_c != null}{$t("forge.tempC", { t: realSweep.gpu_temp_c.toFixed(0) })}{/if}
                {#if realSweep.last_result} · {$t("stage." + realSweep.last_result)}{/if}
              </p>
            {:else}
              <p class="val">—</p>
            {/if}
          </article>
        </div>

        <ForgeKnowledge {realSweep} />

        {#if realSweep.profiles}
          <h5 class="section-head">{$t("forge.profiles")}</h5>
          <ProfileCards realProfiles={realSweep.profiles} {applied} onApplyCore={applyCore} />
        {/if}
      {/if}
    </div>

    <div class="realsweep">
      <div class="real-head">
        <h4 class="section-head">{$t("forge.memSweep")}</h4>
        {#if memRunning}
          <button class="btn stop" onclick={stopMem}>{$t("forge.stopMem")}</button>
        {:else}
          <button class="btn go" onclick={() => (memPreflight = true)}>{$t("forge.runMem")}</button>
        {/if}
      </div>
      {#if memSweep && memSweep.phase !== "idle"}
        <div class="terminal">
          <div class="term-head">
            <span class="dots"><i></i><i></i><i></i></span>
            <span class="term-title">nidavellir · memory sweep</span>
            <span class="term-status" class:live={memRunning}>{memRunning ? "running" : "done"}</span>
          </div>
          <div class="term-body" use:autoscroll={(memSweep.points?.length ?? 0) + (memRunning ? 1 : 0)}>
            <div class="tline base"><span class="gutter">··</span><span class="tlead">base · {memSweep.baseline_gbps.toFixed(0)} GB/s</span></div>
            {#each memSweep.points as p, i}
              <div class="tline">
                <span class="gutter">{(i + 1).toString().padStart(2, "0")}</span>
                <span class="tlead">+{p.offset_mhz} MHz · {p.mem_mhz} MHz</span>
                <span class="tval" class:accent={p.stable} class:danger={!p.stable}>{p.bandwidth_gbps.toFixed(0)} GB/s</span>
                {#if p.min_gbps > 0}<span class="tmin">min {p.min_gbps.toFixed(0)}</span>{/if}
                <span class="tstatus" class:danger={!p.stable}>{p.stable ? "ok" : "✗ queda"}</span>
              </div>
            {/each}
            {#if memRunning}
              <div class="tline running">
                <span class="gutter">»</span>
                <span class="cursor"></span>
                <span class="tlead">{memSweep.validation_note ?? "…"}</span>
              </div>
            {/if}
          </div>
        </div>
        {#if memSweep.peak_gbps > 0}
          <p class="point accent">
            {$t("forge.peakResult", { o: memSweep.peak_offset_mhz, g: memSweep.peak_gbps.toFixed(0) })}
          </p>
          {#if !memRunning && memSweep.validation_note}<p class="sub">{memSweep.validation_note}</p>{/if}
          {#if !memRunning}<button class="btn go small" onclick={applyMem}>{$t("forge.applyMem")}</button>{/if}
        {/if}
      {/if}
    </div>
  </div>
    </div>
  </details>
</section>

{#if preflight}
  <div class="overlay" onclick={() => (preflight = false)} role="presentation">
    <div class="modal" onclick={(e) => e.stopPropagation()} role="presentation">
      <div class="modal-head">
        <strong>⚠ {$t("forge.preTitle")}</strong>
      </div>
      <p class="pre-body">{$t("forge.preBody")}</p>
      <div class="pre-actions">
        <button class="btn ghost" onclick={() => (preflight = false)}>{$t("forge.preCancel")}</button>
        <button class="btn" onclick={() => startReal("StartRealSweepFast")}>{$t("forge.preFast")}</button>
        <button class="btn go" onclick={() => startReal("StartRealSweep")}>{$t("forge.preThorough")}</button>
      </div>
    </div>
  </div>
{/if}

{#if memPreflight}
  <div class="overlay" onclick={() => (memPreflight = false)} role="presentation">
    <div class="modal" onclick={(e) => e.stopPropagation()} role="presentation">
      <div class="modal-head"><strong>⚠ {$t("forge.preTitle")}</strong></div>
      <p class="pre-body">{$t("forge.memPreBody")}</p>
      <div class="pre-actions">
        <button class="btn ghost" onclick={() => (memPreflight = false)}>{$t("forge.preCancel")}</button>
        <button class="btn go" onclick={startMem}>{$t("forge.runMem")}</button>
      </div>
    </div>
  </div>
{/if}

{#if forgePreflight}
  <div class="overlay" onclick={() => (forgePreflight = false)} role="presentation">
    <div class="modal" onclick={(e) => e.stopPropagation()} role="presentation">
      <div class="modal-head"><strong>⚒ {$t("forge.preTitle")}</strong></div>
      <p class="pre-body">{$t("forge.forgePreBody")}</p>
      <div class="pre-actions">
        <button class="btn ghost" onclick={() => (forgePreflight = false)}>{$t("forge.preCancel")}</button>
        <button class="btn go" onclick={startForge}>{$t("forge.runForge")}</button>
      </div>
    </div>
  </div>
{/if}

{#if expanded && realCurve?.real}
  <div class="overlay" onclick={() => (expanded = false)} role="presentation">
    <div class="modal" onclick={(e) => e.stopPropagation()} role="presentation">
      <div class="modal-head">
        <strong>{realCurve.name}</strong>
        <button class="btn ghost" onclick={() => (expanded = false)}>{$t("forge.close")}</button>
      </div>
      <VfChart points={realCurve.points} plateau={chartLimit} height={560} />
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
  .advanced-diagnostics summary {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: 0.7rem;
    align-items: center;
    cursor: pointer;
    color: var(--text);
    font-weight: 700;
    list-style: none;
  }
  .advanced-diagnostics summary::-webkit-details-marker {
    display: none;
  }
  .advanced-diagnostics summary::after {
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
  .advanced-diagnostics[open] summary::after {
    content: "-";
  }
  .advanced-diagnostics summary span,
  .advanced-diagnostics summary small {
    display: block;
  }
  .advanced-diagnostics summary small {
    margin-top: 0.25rem;
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
  .diagnostic-block {
    display: flex;
    flex-direction: column;
    gap: 0.55rem;
  }
  .btn {
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
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
    gap: 0.85rem;
  }
  .tile {
    background: rgba(5, 7, 11, 0.26);
    border: 1px solid rgba(255, 255, 255, 0.055);
    border-radius: 12px;
    padding: 1rem 1.1rem;
  }
  .lab {
    display: block;
    font-size: 0.68rem;
    font-weight: 700;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--nord-dim);
    margin-bottom: 0.4rem;
  }
  .val {
    margin: 0;
    font-weight: 600;
    color: var(--text);
  }
  .section-head {
    margin: 0 0 0.5rem;
    font-size: 0.72rem;
    font-weight: 700;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--muted);
  }
  .point {
    margin: 0.3rem 0;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    color: var(--text);
  }
  .point.accent {
    color: var(--accent);
  }
  .sub {
    margin: 0.25rem 0;
    font-size: 0.82rem;
    color: var(--muted);
  }
  .real {
    border-top: 1px solid var(--border);
    padding-top: 1rem;
  }
  .real-head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 1rem;
  }
  .overlay {
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
  .realsweep {
    margin-top: 1rem;
    padding-top: 0.85rem;
    border-top: 1px dashed var(--forge-line);
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
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
  .btn.small {
    padding: 0.35rem 0.8rem;
    font-size: 0.78rem;
    margin-top: 0.5rem;
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
</style>
