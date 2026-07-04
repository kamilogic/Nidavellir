<script>
  import { Activity, CircleCheck, Gauge, Square, Terminal, Zap } from "@lucide/svelte";
  import LogTerminal from "./LogTerminal.svelte";
  import StatusBadge from "./StatusBadge.svelte";

  let {
    powerSweep = null,
    powerRunning = false,
    safeLoop = null,
    onStopPower,
  } = $props();

  const points = $derived(powerSweep?.points ?? []);
  const isUndervolt = $derived(Boolean(powerSweep?.is_undervolt));
  const profilesQualified = $derived(!isUndervolt || Boolean(powerSweep?.profiles_qualified));
  const isInterrupted = $derived(powerSweep?.phase === "interrupted");
  const isStopping = $derived(powerSweep?.phase === "stopping");
  const phase = $derived.by(() => {
    if (isInterrupted) return "Interrupted";
    return powerSweep?.phase && powerSweep.phase !== "idle" ? powerSweep.phase : "Not running";
  });
  const hasRun = $derived(Boolean(powerSweep && powerSweep.phase !== "idle"));
  const latestPoint = $derived(points.length ? points[points.length - 1] : null);
  const latestLogLine = $derived.by(() => {
    const log = powerSweep?.log ?? [];
    return log.length ? log[log.length - 1] : null;
  });
  const completedSteps = $derived(Number(powerSweep?.completed_steps ?? 0));
  const totalSteps = $derived(Number(powerSweep?.total_steps_estimate ?? 0));
  const elapsedMs = $derived(validDuration(powerSweep?.elapsed_ms));
  const estimatedRemainingMs = $derived(validDuration(powerSweep?.estimated_remaining_ms));
  const estimatedTotalMs = $derived.by(() => {
    if (estimatedRemainingMs == null) return elapsedMs;
    return (elapsedMs ?? 0) + estimatedRemainingMs;
  });
  const estimatedTotalUpperMs = $derived(validDuration(powerSweep?.estimated_total_upper_ms));
  const estimateStage = $derived.by(() => stageEstimate(powerSweep?.phase, powerRunning));
  const frontierPlan = $derived.by(() => {
    const cmax = positiveNumber(powerSweep?.cmax_clock_mhz);
    const floor = positiveNumber(powerSweep?.frontier_floor_clock_mhz);
    const clockCount = positiveNumber(powerSweep?.frontier_clock_count);
    if (cmax == null || floor == null) return null;
    return `${fixed(cmax)} → ${fixed(floor)} MHz${clockCount == null ? "" : ` · ${fixed(clockCount)} physical clocks`}`;
  });
  const progressPercent = $derived.by(() => {
    if (!totalSteps) return 0;
    return Math.min(100, Math.max(0, (completedSteps / totalSteps) * 100));
  });
  const technicalLog = $derived.by(() => {
    const lines = powerSweep?.log ?? [];
    return lines.length ? lines : ["No technical Forge events recorded yet."];
  });
  const latestMessage = $derived.by(() => {
    if (powerSweep?.note) return powerSweep.note;
    if (latestLogLine) return latestLogLine;
    if (powerRunning) return "Nidavellir is learning this GPU's stable core curve.";
    if (hasRun) return "The latest core VF forge run is available for review.";
    return "No core VF forge run is active yet. Start Forge GPU when you are ready to let Nidavellir learn this card.";
  });
  const title = $derived(isInterrupted ? "Forge Interrupted" : powerRunning ? "Forge in Progress" : "Forge Progress");
  const intro = $derived.by(() => {
    if (powerRunning) return "Nidavellir is learning this GPU's stable core curve.";
    if (isInterrupted) {
      return "The previous core VF forge did not finish cleanly. Recover & continue can resume from saved learning after clearing recovery.";
    }
    if (hasRun && (powerSweep?.godforge || powerSweep?.brokkrs || powerSweep?.deep_calm) && !profilesQualified) {
      return "The latest Fast result is provisional. Standard or Long qualification is required before Apply.";
    }
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
    if (isInterrupted) return "Next: recover and continue with saved Forge learning, or use Full reset only to start from zero.";
    if (!powerRunning && profileRows.length && !profilesQualified) {
      return "Next: run Standard or Long to qualify these boundaries and unlock Apply.";
    }
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

  function validDuration(value) {
    if (value == null || value === "") return null;
    const ms = Number(value);
    return Number.isFinite(ms) && ms >= 0 ? ms : null;
  }

  function positiveNumber(value) {
    if (value == null || value === "") return null;
    const number = Number(value);
    return Number.isFinite(number) && number > 0 ? number : null;
  }

  function stageEstimate(currentPhase, running) {
    if (!running && currentPhase === "finished") {
      return {
        label: "Forge complete",
        detail: "The final elapsed time is preserved for this run.",
      };
    }
    if (!running && currentPhase === "provisional") {
      return {
        label: "Provisional map complete",
        detail: "Fast discovery finished without deployable qualification.",
      };
    }
    if (!running) {
      return {
        label: "Waiting to start",
        detail: "A live estimate appears when the Forge begins.",
      };
    }
    switch (currentPhase) {
      case "power":
        return {
          label: "Finding sustainable maximum",
          detail: "Initial estimate; it tightens as Cmax and the physical frontier become known.",
        };
      case "descend":
        return {
          label: "Mapping physical frontier",
          detail: "Recalculated from each completed clock, voltage bin and qualification dwell.",
        };
      case "calibrate":
        return {
          label: "Calibrating Apply power",
          detail: "Filling only the exact Apply-bin measurements still missing.",
        };
      case "synthesize":
        return {
          label: "Selecting forged profiles",
          detail: "The final Apply pairs are being deduplicated before qualification.",
        };
      case "apply-qualify":
        return {
          label: "Final Apply qualification",
          detail: "The upper estimate tightens as each selected Apply pair completes.",
        };
      case "stopping":
        return {
          label: "Completing safe stop",
          detail: "The current bounded batch and checked stock reset are finishing.",
        };
      default:
        return {
          label: "Refining live estimate",
          detail: "The estimate updates as the active Forge stage reports progress.",
        };
    }
  }

  function profilePower(point) {
    const p99 = Number(point?.power_p99_w);
    if (Number.isFinite(p99) && p99 > 0) return p99;
    const peak = Number(point?.max_power_w);
    return Number.isFinite(peak) && peak > 0 ? peak : Number(point?.power_w ?? 0);
  }

  function profilePowerLabel(point) {
    const p99 = Number(point?.power_p99_w);
    return Number.isFinite(p99) && p99 > 0 ? "W sustained p99" : "W peak";
  }

  function profilePowerNote(point) {
    const p99 = Number(point?.power_p99_w);
    return Number.isFinite(p99) && p99 > 0
      ? "Measured sustained p99 power. Not a hard power limit; other workloads can vary."
      : "Measured saturation peak. Not a hard power limit; other workloads can vary.";
  }

  function duration(value) {
    const ms = validDuration(value);
    if (ms == null) return "Calculating…";
    const totalSeconds = Math.round(ms / 1000);
    const hours = Math.floor(totalSeconds / 3600);
    const minutes = Math.floor((totalSeconds % 3600) / 60);
    if (hours) return `${hours}h ${minutes}m`;
    const seconds = totalSeconds % 60;
    return minutes ? `${minutes}m ${seconds}s` : `${seconds}s`;
  }

  function targetLabel(point) {
    if (!point) return "Not available";
    return `${point.target_clock_mhz ?? point.clock_mhz} MHz target`;
  }

  function curveAnchor(point) {
    if (point?.vf_table_voltage_mv != null && point?.boundary_voltage_mv != null) {
      return `Apply VF bin: ${point.vf_table_voltage_mv} mV · learned boundary ${point.boundary_voltage_mv} mV · +${point.apply_margin_mv ?? 0} mV margin`;
    }
    if (point?.vf_table_voltage_mv != null) return `VF bin: ${point.vf_table_voltage_mv} mV`;
    return null;
  }

  function achievedClock(point) {
    if (point?.target_clock_mhz == null || point?.clock_mhz == null) return null;
    const p5 = point.p5_clock_mhz != null ? ` · electrical regime p5 ${point.p5_clock_mhz} MHz` : "";
    const p95 = point.p95_clock_mhz != null ? ` · sustained p95 ${point.p95_clock_mhz} MHz` : "";
    return `Measured avg: ${point.clock_mhz} MHz${p5}${p95}`;
  }

  function measuredVoltage(point) {
    if (!point) return null;
    const avg = point.avg_measured_voltage_mv;
    const min = point.min_measured_voltage_mv;
    const max = point.max_measured_voltage_mv;
    if (avg != null && min != null && max != null) {
      return `Measured voltage under load: ${avg} / ${min} / ${max} mV`;
    }
    if (point.measured_voltage_mv != null) return `Measured voltage under load: ${point.measured_voltage_mv} mV`;
    return null;
  }

  function confidenceSummary(point) {
    if (!point) return null;
    const parts = [];
    if (point.confidence != null) {
      const confidence = Number(point.confidence);
      if (Number.isFinite(confidence)) parts.push(`Stability confidence ${confidence.toFixed(2)}`);
    }
    if (point.validation_count != null) {
      const validationCount = Number(point.validation_count);
      if (Number.isFinite(validationCount)) {
        parts.push(`${validationCount} ${validationCount === 1 ? "confirmation" : "confirmations"}`);
      }
    }
    return parts.length ? parts.join(" · ") : null;
  }
</script>

<div class="forge-all">
  <div class="progress-head">
    <div>
      <span class="eyebrow">Current action</span>
      <h3>
        <Activity size={18} strokeWidth={1.85} />
        <span>{title}</span>
      </h3>
      <p class="sub">{intro}</p>
    </div>
    <div class="head-actions">
      <StatusBadge label={safetyState} variant={safetyVariant} symbol={safetyVariant === "attention" ? "attention" : "shield"} compact />
      {#if profileRows.length}
        <StatusBadge
          label={profilesQualified ? "Qualified" : "Provisional"}
          variant={profilesQualified ? "forged" : "tempered"}
          symbol={profilesQualified ? "knowledge" : "activity"}
          compact
        />
      {/if}
      <span class="run-state" class:live={powerRunning} class:interrupted={isInterrupted}>
        {isStopping ? "Stopping" : powerRunning ? "Running" : isInterrupted ? "Interrupted" : hasRun ? "Stopped" : "Idle"}
      </span>
      {#if powerRunning}
        <button class="btn stop" onclick={onStopPower} disabled={isStopping}>
          <Square size={14} strokeWidth={1.9} />
          <span>{isStopping ? "Stopping…" : "Stop forging"}</span>
        </button>
      {/if}
    </div>
  </div>

  <div class="progress-summary">
    <div>
      <span class="label-with-icon">
        <Activity size={13} strokeWidth={1.85} />
        Current phase
      </span>
      <strong>{phase}</strong>
    </div>
    <p>{latestMessage}</p>
  </div>

  <section class="sweep-progress" aria-label="GPU sweep progress">
    <div class="sweep-progress-head">
      <div>
        <span>Full GPU sweep</span>
        <strong>{completedSteps} / {totalSteps || "—"} estimated dwells</strong>
      </div>
      <div class="sweep-timing">
        <small>{fixed(progressPercent)}% estimated</small>
      </div>
    </div>
    <div
      class="progress-track"
      role="progressbar"
      aria-label="Estimated Forge completion"
      aria-valuemin="0"
      aria-valuemax="100"
      aria-valuenow={Math.round(progressPercent)}
    >
      <span style={`width: ${progressPercent}%`}></span>
    </div>
    <div class="estimate-stage">
      <div>
        <span>Live estimate · {estimateStage.label}</span>
        {#if frontierPlan}
          <strong>{frontierPlan}</strong>
        {/if}
      </div>
      <small>{estimateStage.detail}</small>
    </div>
    <div class="timing-grid" aria-live="polite">
      <div class="timing-metric">
        <span>Remaining</span>
        <strong>{powerRunning && estimatedRemainingMs != null ? `≈ ${duration(estimatedRemainingMs)}` : "—"}</strong>
        <small>Updates after each reported dwell.</small>
      </div>
      <div class="timing-metric">
        <span>Estimated run total</span>
        <strong>{hasRun && estimatedTotalMs != null ? `≈ ${duration(estimatedTotalMs)}` : "—"}</strong>
        <small>Elapsed plus the current remaining estimate.</small>
      </div>
      <div class="timing-metric maximum" class:pending={estimatedTotalUpperMs == null}>
        <span>Maximum estimated total</span>
        <strong>{estimatedTotalUpperMs != null ? `Up to ${duration(estimatedTotalUpperMs)}` : powerRunning ? "Refining" : "—"}</strong>
        <small>
          {estimatedTotalUpperMs != null
            ? "Includes the current conservative work ceiling."
            : "Becomes available when the backend publishes the refined ceiling."}
        </small>
      </div>
      <div class="timing-metric">
        <span>Elapsed</span>
        <strong>{hasRun && elapsedMs != null ? duration(elapsedMs) : "—"}</strong>
        <small>Measured wall time for this Forge run.</small>
      </div>
    </div>
    <div class="live-target">
      <span>
        {powerSweep?.current_clock_mhz
          ? `${powerSweep.current_clock_mhz} MHz @ ${powerSweep.current_voltage_mv ?? "—"} mV`
          : powerRunning
            ? "Preparing next hardware point"
            : "No active hardware point"}
      </span>
      <small>{powerSweep?.last_outcome ?? (powerRunning ? "Waiting for dwell result" : "Idle")}</small>
      <small class:saved={powerSweep?.learning_saved}>
        {powerSweep?.learning_saved
          ? `${powerSweep?.learned_points ?? 0} new dwell(s) saved`
          : "No saved learning in this run"}
      </small>
    </div>
  </section>

  <div class="progress-grid">
    <article>
      <span class="label-with-icon">
        <CircleCheck size={13} strokeWidth={1.85} />
        Tested points
      </span>
      <strong>{points.length}</strong>
      <small>{points.length ? "Measured during the canonical Power Sweep." : "No tested points yet."}</small>
    </article>
    <article>
      <span class="label-with-icon">
        <Gauge size={13} strokeWidth={1.85} />
        Latest tested point
      </span>
      <strong>{targetLabel(latestPoint)}</strong>
      {#if latestPoint}
        <small>Optimized boost curve</small>
        {#if achievedClock(latestPoint)}
          <small>{achievedClock(latestPoint)}</small>
        {/if}
        {#if curveAnchor(latestPoint)}
          <small>{curveAnchor(latestPoint)}</small>
        {/if}
        {#if measuredVoltage(latestPoint)}
          <small>{measuredVoltage(latestPoint)}</small>
        {/if}
        <small>{fixed(profilePower(latestPoint))} {profilePowerLabel(latestPoint)} / {fixed(latestPoint.perf_per_watt, 1)} MHz/W / {latestPoint.stable ? "stable" : "failed"}</small>
        {#if confidenceSummary(latestPoint)}
          <small class="confidence">{confidenceSummary(latestPoint)}</small>
        {/if}
      {:else}
        <small>Appears after the first measured point.</small>
      {/if}
    </article>
    <article>
      <span class="label-with-icon">
        <Zap size={13} strokeWidth={1.85} />
        Power target
      </span>
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
            <span>{targetLabel(point)}</span>
            <small>Optimized boost curve</small>
            {#if achievedClock(point)}
              <small>{achievedClock(point)}</small>
            {/if}
            {#if curveAnchor(point)}
              <small>{curveAnchor(point)}</small>
            {/if}
            {#if measuredVoltage(point)}
              <small>{measuredVoltage(point)}</small>
            {/if}
            <small>{fixed(profilePower(point))} {profilePowerLabel(point)} / {fixed(point.perf_per_watt, 1)} MHz/W</small>
            <small class="power-note">{profilePowerNote(point)}</small>
            {#if confidenceSummary(point)}
              <small class="confidence">{confidenceSummary(point)}</small>
            {/if}
          </article>
        {/each}
      </div>
    </div>
  {/if}

  <section class="progress-log" aria-label="Technical Power Sweep log">
      <header>
        <Terminal size={14} strokeWidth={1.85} />
        <span>Technical Power Sweep log</span>
        <small>{powerRunning ? "Live" : "Persistent history"}</small>
      </header>
      <LogTerminal
        title="nidavellir / core vf forge"
        status={powerSweep?.running ? powerSweep.phase : "idle"}
        live={Boolean(powerSweep?.running)}
        lines={technicalLog}
        runningText={powerSweep?.running ? `${powerSweep.phase}...` : null}
      />
  </section>
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
    display: inline-flex;
    align-items: center;
    gap: 0.45rem;
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
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 0.4rem;
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
  .run-state.interrupted {
    border-color: rgba(191, 97, 106, 0.42);
    background: rgba(191, 97, 106, 0.12);
    color: #f3b9bd;
  }
  .progress-summary,
  .sweep-progress,
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
  .sweep-progress {
    padding: 0.72rem 0.8rem;
  }
  .sweep-progress-head,
  .live-target,
  .progress-log header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.75rem;
  }
  .sweep-progress-head span,
  .progress-log header span {
    display: block;
    color: var(--nord-dim);
    font-size: 0.7rem;
    font-weight: 800;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }
  .sweep-progress-head strong {
    display: block;
    margin-top: 0.2rem;
    color: var(--text);
    font-size: 0.9rem;
    font-variant-numeric: tabular-nums;
  }
  .sweep-timing {
    display: flex;
    gap: 0.7rem;
    color: var(--forge-gold);
    font-size: 0.78rem;
    font-variant-numeric: tabular-nums;
  }
  .progress-track {
    height: 8px;
    margin-top: 0.65rem;
    overflow: hidden;
    border: 1px solid rgba(214, 168, 93, 0.22);
    border-radius: 999px;
    background: rgba(5, 7, 11, 0.72);
  }
  .progress-track span {
    display: block;
    width: 0;
    height: 100%;
    border-radius: inherit;
    background: linear-gradient(90deg, rgba(214, 168, 93, 0.72), var(--forge-gold));
    box-shadow: 0 0 14px rgba(214, 168, 93, 0.26);
    transition: width 220ms ease-out;
  }
  .estimate-stage {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.8rem;
    margin-top: 0.6rem;
    padding: 0.58rem 0.65rem;
    border-radius: 6px;
    background: rgba(214, 168, 93, 0.055);
    box-shadow: inset 0 0 0 1px rgba(214, 168, 93, 0.12);
  }
  .estimate-stage span,
  .timing-metric span {
    display: block;
    color: var(--nord-dim);
    font-size: 0.66rem;
    font-weight: 800;
    letter-spacing: 0.07em;
    text-transform: uppercase;
  }
  .estimate-stage strong {
    display: block;
    margin-top: 0.16rem;
    color: var(--forge-gold);
    font-size: 0.78rem;
    font-variant-numeric: tabular-nums;
  }
  .estimate-stage small {
    max-width: 28rem;
    color: var(--muted);
    font-size: 0.72rem;
    line-height: 1.35;
    text-align: right;
    text-wrap: pretty;
  }
  .timing-grid {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: 0.45rem;
    margin-top: 0.5rem;
  }
  .timing-metric {
    min-width: 0;
    padding: 0.55rem 0.62rem;
    border-radius: 6px;
    background: rgba(5, 7, 11, 0.42);
    box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.045);
  }
  .timing-metric strong {
    display: block;
    margin-top: 0.22rem;
    color: var(--text);
    font-size: 0.88rem;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .timing-metric small {
    display: block;
    margin-top: 0.18rem;
    color: var(--muted);
    font-size: 0.68rem;
    line-height: 1.32;
    text-wrap: pretty;
  }
  .timing-metric.maximum {
    background: rgba(214, 168, 93, 0.075);
    box-shadow: inset 0 0 0 1px rgba(214, 168, 93, 0.16);
  }
  .timing-metric.maximum strong {
    color: var(--forge-gold);
  }
  .timing-metric.maximum.pending {
    background: rgba(5, 7, 11, 0.32);
    box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.035);
  }
  .timing-metric.maximum.pending strong {
    color: var(--nord-dim);
  }
  .live-target {
    margin-top: 0.55rem;
    color: var(--muted);
    font-size: 0.74rem;
    font-variant-numeric: tabular-nums;
  }
  .live-target span {
    color: var(--text);
    font-weight: 700;
  }
  .live-target small.saved {
    color: var(--forge-green);
  }
  .label-with-icon {
    display: inline-flex;
    align-items: center;
    gap: 0.32rem;
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
  .confidence {
    color: var(--forge-green) !important;
    font-variant-numeric: tabular-nums;
  }
  .power-note {
    text-wrap: pretty;
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
  .result-grid span,
  .result-grid small {
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
  .progress-log header {
    justify-content: flex-start;
    color: var(--muted);
    margin-bottom: 0.55rem;
  }
  .progress-log header small {
    margin-left: auto;
    color: var(--nord-dim);
    font-size: 0.68rem;
    font-weight: 700;
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
    .result-grid,
    .timing-grid {
      grid-template-columns: 1fr;
    }
    .sweep-progress-head,
    .live-target,
    .estimate-stage {
      align-items: flex-start;
      flex-direction: column;
    }
    .estimate-stage small {
      text-align: left;
    }
  }
</style>
