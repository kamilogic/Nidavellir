<script>
  import LogTerminal from "./LogTerminal.svelte";
  import StatusBadge from "./StatusBadge.svelte";

  let {
    powerSweep = null,
    powerRunning = false,
    safeLoop = null,
    showLog = false,
    onStopPower,
  } = $props();

  const points = $derived(powerSweep?.points ?? []);
  const phase = $derived(powerSweep?.phase && powerSweep.phase !== "idle" ? powerSweep.phase : "Not running");
  const hasRun = $derived(Boolean(powerSweep && powerSweep.phase !== "idle"));
  const latestPoint = $derived(points.length ? points[points.length - 1] : null);
  const latestLogLine = $derived.by(() => {
    const log = powerSweep?.log ?? [];
    return log.length ? log[log.length - 1] : null;
  });
  const latestMessage = $derived.by(() => {
    if (powerSweep?.note) return powerSweep.note;
    if (latestLogLine) return latestLogLine;
    if (powerRunning) return "Nidavellir is learning this GPU's stable core curve.";
    if (hasRun) return "The latest core VF forge run is available for review.";
    return "No core VF forge run is active yet. Start Forge GPU when you are ready to let Nidavellir learn this card.";
  });
  const title = $derived(powerRunning ? "Forge in Progress" : "Forge Progress");
  const intro = $derived.by(() => {
    if (powerRunning) return "Nidavellir is learning this GPU's stable core curve.";
    if (hasRun) return "Review the latest core forge run and the profiles it produced.";
    return "Forge progress will appear here when Nidavellir starts learning the GPU core curve.";
  });
  const profileRows = $derived.by(() =>
    [
      ["Godforge", powerSweep?.godforge],
      ["Brokkr's Best", powerSweep?.brokkrs],
      ["Deep Calm", powerSweep?.deep_calm],
    ].filter(([, point]) => point),
  );
  const nextStep = $derived.by(() => {
    if (!powerRunning && profileRows.length) return "Next: choose and apply the profile that matches your goal.";
    if (!powerRunning) return "Next: start Forge GPU when you are ready.";
    if (!points.length) return "Next: measure the first stable operating point.";
    if (!profileRows.length) return "Next: profile generation after the stable region is confirmed.";
    return "Next: finish validation and refresh profile recommendations.";
  });
  const safetyState = $derived.by(() => {
    if (!safeLoop) return "Protected";
    if (safeLoop.safe_mode || safeLoop.state === "unstable") return "Needs Attention";
    if (safeLoop.boot_flag_armed || ["probing", "applying", "dwell"].includes(safeLoop.state)) return "Recovery Ready";
    if ((safeLoop.recent_crashes?.length ?? 0) > 0 && safeLoop.consecutive_crashes === 0) return "Recovered Successfully";
    return "Protected";
  });
  const safetyVariant = $derived.by(() => {
    if (safetyState === "Needs Attention") return "attention";
    if (safetyState === "Recovery Ready") return "recovery";
    if (safetyState === "Recovered Successfully") return "recovered";
    return "protected";
  });

  function fixed(value, digits = 0) {
    const n = Number(value);
    return Number.isFinite(n) ? n.toFixed(digits) : "0";
  }

  function voltageLabel(point) {
    if (!point) return "Not available";
    const bin = point.vf_table_voltage_mv;
    const voltage = bin ?? point.voltage_mv;
    return `${point.clock_mhz} MHz @ ${voltage} mV${bin != null ? " VF bin" : ""}`;
  }
</script>

<div class="forge-all">
  <div class="progress-head">
    <div>
      <span class="eyebrow">Current action</span>
      <h3>{title}</h3>
      <p class="sub">{intro}</p>
    </div>
    <div class="head-actions">
      <StatusBadge label={safetyState} variant={safetyVariant} compact />
      <span class="run-state" class:live={powerRunning}>{powerRunning ? "Running" : hasRun ? "Stopped" : "Idle"}</span>
      {#if powerRunning}
        <button class="btn stop" onclick={onStopPower}>Stop forging</button>
      {/if}
    </div>
  </div>

  <div class="progress-summary">
    <div>
      <span>Current phase</span>
      <strong>{phase}</strong>
    </div>
    <p>{latestMessage}</p>
  </div>

  <div class="progress-grid">
    <article>
      <span>Tested points</span>
      <strong>{points.length}</strong>
      <small>{points.length ? "Measured during the canonical Power Sweep." : "No tested points yet."}</small>
    </article>
    <article>
      <span>Latest tested point</span>
      <strong>{voltageLabel(latestPoint)}</strong>
      {#if latestPoint}
        <small>{fixed(latestPoint.power_w)} W / {fixed(latestPoint.perf_per_watt, 1)} MHz/W / {latestPoint.stable ? "stable" : "failed"}</small>
      {:else}
        <small>Appears after the first measured point.</small>
      {/if}
    </article>
    <article>
      <span>Power target</span>
      <strong>{powerSweep?.target_w ? `${fixed(powerSweep.target_w)} W` : "Not set"}</strong>
      <small>{powerSweep?.power_limit_w ? `Power limit ${fixed(powerSweep.power_limit_w)} W` : "Available after forge data loads."}</small>
    </article>
  </div>

  <div class="next-step">
    <span>What happens next</span>
    <strong>{nextStep}</strong>
    <small>VRAM optimization is planned for a later pipeline step after the core curve is forged.</small>
  </div>

  <div class="pipeline-steps" aria-label="Forge pipeline status">
    <span class:active={powerRunning} class:done={hasRun}>Core VF forge</span>
    <span class:done={profileRows.length > 0}>Profile generation</span>
    <span class="planned">VRAM optimization planned</span>
    <span class="planned">Final validation planned</span>
  </div>

  {#if profileRows.length}
    <div class="profile-results">
      <span class="results-title">Generated profiles</span>
      <div class="result-grid">
        {#each profileRows as [name, point]}
          <article>
            <strong>{name}</strong>
            <span>{voltageLabel(point)}</span>
            <small>{fixed(point.power_w)} W / {fixed(point.perf_per_watt, 1)} MHz/W</small>
          </article>
        {/each}
      </div>
    </div>
  {/if}

  {#if powerSweep?.log?.length}
    <details class="progress-log" open={showLog}>
      <summary>Technical Power Sweep log</summary>
      <LogTerminal
        title="nidavellir / core vf forge"
        status={powerSweep.running ? powerSweep.phase : "done"}
        live={powerSweep.running}
        lines={powerSweep.log ?? []}
        runningText={powerSweep.running ? `${powerSweep.phase}...` : null}
      />
    </details>
  {/if}
</div>

<style>
  .forge-all {
    background: var(--forge-panel-bg);
    border: 1px solid var(--forge-line);
    border-radius: 12px;
    padding: 0.85rem 1rem;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    box-shadow: var(--forge-panel-edge);
  }
  .progress-head {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    gap: 1rem;
  }
  .eyebrow,
  .progress-summary span,
  .progress-grid span,
  .next-step span,
  .results-title {
    display: block;
    font-size: 0.72rem;
    font-weight: 700;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--nord-dim);
    margin-bottom: 0.3rem;
  }
  h3 {
    margin: 0;
    color: var(--text);
    font-size: 1.05rem;
  }
  .sub {
    margin: 0.25rem 0;
    font-size: 0.82rem;
    color: var(--muted);
    line-height: 1.45;
  }
  .head-actions {
    display: flex;
    justify-content: flex-end;
    align-items: center;
    gap: 0.45rem;
    flex-wrap: wrap;
  }
  .btn {
    border: 1px solid var(--border);
    border-radius: 9px;
    padding: 0.46rem 0.8rem;
    font-weight: 700;
    font-size: 0.78rem;
    cursor: pointer;
    background: rgba(8, 11, 16, 0.66);
    color: var(--text);
  }
  .btn.stop {
    background: rgba(191, 97, 106, 0.16);
    color: #f3b9bd;
    border-color: rgba(191, 97, 106, 0.45);
  }
  .run-state {
    border: 1px solid var(--forge-line);
    border-radius: 999px;
    background: rgba(5, 7, 11, 0.3);
    color: var(--nord-dim);
    font-size: 0.68rem;
    font-weight: 800;
    letter-spacing: 0.08em;
    line-height: 1;
    padding: 0.38rem 0.62rem;
    text-transform: uppercase;
    white-space: nowrap;
  }
  .run-state.live {
    border-color: rgba(214, 168, 93, 0.42);
    background: rgba(214, 168, 93, 0.1);
    color: var(--forge-gold);
  }
  .progress-summary,
  .progress-grid article,
  .next-step,
  .profile-results {
    background: rgba(5, 7, 11, 0.28);
    border: 1px solid rgba(255, 255, 255, 0.055);
    border-radius: 8px;
  }
  .progress-summary {
    display: grid;
    grid-template-columns: minmax(150px, 0.28fr) minmax(0, 1fr);
    gap: 0.75rem;
    padding: 0.7rem 0.8rem;
  }
  .progress-summary strong,
  .progress-grid strong,
  .next-step strong {
    color: var(--text);
    font-size: 0.95rem;
    font-variant-numeric: tabular-nums;
  }
  .progress-summary p,
  .progress-grid small,
  .next-step small,
  .result-grid small {
    margin: 0.2rem 0 0;
    color: var(--muted);
    font-size: 0.8rem;
    line-height: 1.4;
  }
  .progress-grid {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 0.55rem;
  }
  .progress-grid article {
    padding: 0.68rem 0.75rem;
  }
  .next-step {
    padding: 0.68rem 0.75rem;
  }
  .next-step strong {
    display: block;
  }
  .pipeline-steps {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: 0.45rem;
  }
  .pipeline-steps span {
    border: 1px solid var(--forge-line);
    border-radius: 8px;
    background: rgba(5, 7, 11, 0.22);
    color: var(--nord-dim);
    font-size: 0.68rem;
    font-weight: 800;
    letter-spacing: 0.06em;
    line-height: 1.25;
    padding: 0.48rem 0.58rem;
    text-transform: uppercase;
  }
  .pipeline-steps span.active {
    border-color: rgba(214, 168, 93, 0.42);
    color: var(--forge-gold);
  }
  .pipeline-steps span.done {
    border-color: rgba(157, 191, 145, 0.36);
    color: var(--forge-green);
  }
  .pipeline-steps span.planned {
    border-style: dashed;
    opacity: 0.62;
  }
  .profile-results {
    padding: 0.72rem 0.8rem;
  }
  .result-grid {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 0.55rem;
    margin-top: 0.45rem;
  }
  .result-grid article {
    border-left: 2px solid var(--forge-line);
    padding-left: 0.6rem;
  }
  .result-grid strong,
  .result-grid span {
    display: block;
  }
  .result-grid strong {
    color: var(--text);
    font-size: 0.86rem;
  }
  .result-grid span {
    color: var(--forge-gold);
    font-size: 0.82rem;
    font-variant-numeric: tabular-nums;
    margin-top: 0.2rem;
  }
  .progress-log {
    border-top: 1px solid var(--forge-line);
    padding-top: 0.65rem;
  }
  .progress-log summary {
    cursor: pointer;
    color: var(--muted);
    font-size: 0.75rem;
    font-weight: 800;
    letter-spacing: 0.08em;
    list-style: none;
    margin-bottom: 0.55rem;
    text-transform: uppercase;
  }
  .progress-log summary::-webkit-details-marker {
    display: none;
  }
  @media (max-width: 760px) {
    .progress-head,
    .progress-summary {
      grid-template-columns: 1fr;
    }
    .progress-head {
      flex-direction: column;
    }
    .head-actions {
      justify-content: flex-start;
    }
    .progress-grid,
    .pipeline-steps,
    .result-grid {
      grid-template-columns: 1fr;
    }
  }
</style>
