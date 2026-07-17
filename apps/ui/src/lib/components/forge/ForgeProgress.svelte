<script>
  import { Activity, ArrowRight, CircleCheck, Gauge, Play, Square, Timer, Zap } from "@lucide/svelte";
  import StatusBadge from "./StatusBadge.svelte";

  let {
    powerSweep = null,
    powerRunning = false,
    safeLoop = null,
    forgeMode = "standard",
    onStopPower,
    onStartPower,
    onRecoverContinue,
    onResumePower,
  } = $props();

  let now = $state(Date.now());
  let observedPhase = $state(null);
  let phaseStartedAt = $state(Date.now());
  let observedTask = $state(null);
  let taskReportedElapsedMs = $state(0);
  let taskObservedAt = $state(Date.now());
  let observedElapsedRaw = $state(null);
  let observedRemainingRaw = $state(null);
  let observedTimingRunning = $state(false);
  let timingElapsedBase = $state(null);
  let timingRemainingBase = $state(null);
  let timingObservedAt = $state(Date.now());

  const points = $derived(powerSweep?.points ?? []);
  const isUndervolt = $derived(Boolean(powerSweep?.is_undervolt));
  const profilesQualified = $derived(!isUndervolt || Boolean(powerSweep?.profiles_qualified));
  const isInterrupted = $derived(powerSweep?.phase === "interrupted");
  const isPaused = $derived(powerSweep?.phase === "paused");
  const isStopping = $derived(powerSweep?.phase === "stopping");
  const phase = $derived.by(() => {
    if (isInterrupted) return "Interrupted";
    if (isPaused) return "Paused safely";
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
  const reportedElapsedMs = $derived(validDuration(powerSweep?.elapsed_ms));
  const reportedRemainingMs = $derived(validDuration(powerSweep?.estimated_remaining_ms));
  const elapsedMs = $derived(
    timingElapsedBase == null
      ? null
      : timingElapsedBase + (powerRunning ? Math.max(0, now - timingObservedAt) : 0),
  );
  const estimatedRemainingMs = $derived(
    timingRemainingBase == null
      ? null
      : Math.max(0, timingRemainingBase - (powerRunning ? Math.max(0, now - timingObservedAt) : 0)),
  );
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
  const clockDomain = $derived.by(() => {
    const boost = positiveNumber(powerSweep?.observed_boost_clock_mhz);
    const bins = positiveNumber(powerSweep?.clock_table_bin_count);
    const tableCeiling = positiveNumber(powerSweep?.clock_table_ceiling_mhz);
    const temperature = positiveNumber(powerSweep?.preheat_temperature_c);
    const converged = powerSweep?.preheat_converged;
    if (boost == null && bins == null && tableCeiling == null && temperature == null && converged == null) return null;
    const parts = [];
    if (tableCeiling != null || bins != null) {
      parts.push(
        `Ctable${tableCeiling == null ? "" : ` ${fixed(tableCeiling)} MHz`}${bins == null ? "" : ` / ${fixed(bins)} bins`}`,
      );
    }
    if (boost != null) parts.push(`Cboost ${fixed(boost)} MHz observed`);
    if (converged === true) {
      parts.push(`preheat converged${temperature == null ? "" : ` at ${fixed(temperature)}°C`}`);
    } else if (converged === false) {
      parts.push("preheat not converged");
    }
    return parts.join(" · ");
  });
  const progressPercent = $derived.by(() => {
    if (!totalSteps) return 0;
    return Math.min(100, Math.max(0, (completedSteps / totalSteps) * 100));
  });
  const averageStepMs = $derived.by(() => {
    if (!completedSteps || elapsedMs == null) return null;
    return Math.max(1000, elapsedMs / completedSteps);
  });
  const currentTaskElapsedMs = $derived(validDuration(powerSweep?.current_task_elapsed_ms));
  const currentTaskEstimatedTotalMs = $derived(validDuration(powerSweep?.current_task_estimated_total_ms));
  const phaseElapsedMs = $derived(
    powerRunning
      ? powerSweep?.current_task
        ? taskReportedElapsedMs + Math.max(0, now - taskObservedAt)
        : Math.max(0, now - phaseStartedAt)
      : null,
  );
  const currentTaskRemainingMs = $derived.by(() => {
    if (!powerRunning || phaseElapsedMs == null) return null;
    if (currentTaskEstimatedTotalMs != null) {
      return Math.max(0, currentTaskEstimatedTotalMs - phaseElapsedMs);
    }
    if (averageStepMs == null || !["power", "descend", "calibrate", "apply-qualify"].includes(powerSweep?.phase)) return null;
    return Math.max(0, averageStepMs - phaseElapsedMs);
  });
  const currentTaskLabel = $derived(taskLabel(powerSweep?.current_task) ?? estimateStage.label);
  const nextTaskInfo = $derived.by(() => {
    if (powerSweep?.next_task) {
      return {
        label: taskLabel(powerSweep.next_task),
        durationMs: validDuration(powerSweep?.next_task_estimated_duration_ms),
      };
    }
    return nextTaskEstimate(powerSweep?.phase, averageStepMs);
  });
  const estimatedFinish = $derived.by(() => {
    if (!powerRunning || estimatedRemainingMs == null) return null;
    return new Intl.DateTimeFormat(undefined, { hour: "2-digit", minute: "2-digit" }).format(
      new Date(now + estimatedRemainingMs),
    );
  });
  const canContinueSaved = $derived.by(() => {
    return Boolean(!powerRunning && isPaused && powerSweep?.resume_available);
  });
  const latestMessage = $derived.by(() => {
    if (powerSweep?.note) return powerSweep.note;
    if (latestLogLine) return latestLogLine;
    if (powerRunning) return "Nidavellir is learning this GPU's stable core curve.";
    if (hasRun) return "The latest core VF forge run is available for review.";
    return "No core VF forge run is active yet. Start Forge GPU when you are ready to let Nidavellir learn this card.";
  });
  const title = $derived(isInterrupted ? "Forge Interrupted" : isPaused ? "Forge Paused" : powerRunning ? "Forge in Progress" : "Forge Progress");
  const intro = $derived.by(() => {
    if (powerRunning) return "Nidavellir is learning this GPU's stable core curve.";
    if (isInterrupted) {
      return "The previous core VF forge did not finish cleanly. Recover & continue can resume from saved learning after clearing recovery.";
    }
    if (isPaused) {
      return powerSweep?.resume_available
        ? "The GPU is back at stock and the compatible checkpoint is ready to resume explicitly."
        : "The GPU is back at stock, but Core refused resume compatibility for this checkpoint.";
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
    if (isPaused) {
      return powerSweep?.resume_available
        ? "Next: resume this exact checkpoint when you are ready."
        : `Resume unavailable: ${powerSweep?.resume_block_reason ?? "Core did not provide a compatibility verdict."}`;
    }
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
      case "preheat":
        return {
          label: "Normalizing stock conditions",
          detail: "Waiting for temperature and sustained clock to converge before sampling Cboost.",
        };
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

  function nextTaskEstimate(currentPhase, stepMs) {
    const estimatedStep = stepMs == null ? null : Math.max(1000, stepMs);
    switch (currentPhase) {
      case "preheat":
        return { label: "Find the sustainable maximum clock", durationMs: null };
      case "power":
        return { label: "Map the physical voltage frontier", durationMs: estimatedStep };
      case "descend":
        return { label: "Test the next hardware-derived candidate", durationMs: estimatedStep };
      case "calibrate":
        return { label: "Synthesize the three profile goals", durationMs: 90_000 };
      case "synthesize":
        return { label: "Qualify the exact Apply pairs", durationMs: estimatedStep };
      case "apply-qualify":
        return { label: "Publish the qualified profiles", durationMs: 60_000 };
      case "stopping":
        return { label: "Save learning and confirm stock reset", durationMs: null };
      default:
        return { label: "Prepare the next Forge stage", durationMs: null };
    }
  }

  function taskLabel(task) {
    if (!task) return null;
    const labels = {
      prepare_stock: "Prepare and verify stock state",
      stock_preheat: "Normalize stock temperature and clock",
      capture_goldens: "Capture stock render references",
      frontier_descent: "Map the physical clock and voltage frontier",
      power_calibration: "Calibrate exact Apply-pair power",
      profile_synthesis: "Synthesize the three profile goals",
      apply_calibration: "Calibrate exact Apply pairs",
      synthesize_profiles: "Synthesize profile goals",
      apply_qualification: "Qualify exact Apply pairs",
      publish_profiles: "Publish qualified profiles",
      final_stock_reset: "Restore and verify stock state",
      final_reset: "Restore and verify stock state",
    };
    return labels[task] ?? String(task).replaceAll("_", " ");
  }

  function continueSavedRun() {
    if (!canContinueSaved) return;
    onResumePower?.();
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

  $effect(() => {
    const current = powerSweep?.phase ?? "idle";
    if (current !== observedPhase) {
      observedPhase = current;
      phaseStartedAt = Date.now();
    }
  });

  $effect(() => {
    const current = powerSweep?.current_task ?? null;
    const reported = currentTaskElapsedMs ?? 0;
    if (current !== observedTask || reported > taskReportedElapsedMs) {
      observedTask = current;
      taskReportedElapsedMs = reported;
      taskObservedAt = Date.now();
    }
  });

  $effect(() => {
    if (
      reportedElapsedMs !== observedElapsedRaw ||
      reportedRemainingMs !== observedRemainingRaw ||
      powerRunning !== observedTimingRunning
    ) {
      observedElapsedRaw = reportedElapsedMs;
      observedRemainingRaw = reportedRemainingMs;
      observedTimingRunning = powerRunning;
      timingElapsedBase = reportedElapsedMs;
      timingRemainingBase = reportedRemainingMs;
      timingObservedAt = Date.now();
    }
  });

  $effect(() => {
    const interval = setInterval(() => (now = Date.now()), 1000);
    return () => clearInterval(interval);
  });

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
      <span class="run-state" class:live={powerRunning} class:interrupted={isInterrupted} class:paused={isPaused}>
        {isStopping ? "Stopping" : powerRunning ? "Running" : isInterrupted ? "Interrupted" : isPaused ? "Paused" : hasRun ? "Stopped" : "Idle"}
      </span>
      {#if powerRunning}
        <button class="btn stop" onclick={onStopPower} disabled={isStopping}>
          <Square size={14} strokeWidth={1.9} />
          <span>{isStopping ? "Stopping…" : "Stop forging"}</span>
        </button>
      {:else if canContinueSaved}
        <button class="btn continue" onclick={continueSavedRun}>
          <Play size={14} strokeWidth={1.9} />
          <span>Resume Forge</span>
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
        {#if clockDomain}
          <small class="clock-domain">{clockDomain}</small>
        {/if}
      </div>
      <small>{estimateStage.detail}</small>
    </div>
    {#if powerRunning}
      <div class="task-timeline" aria-live="polite">
        <article class="task-card current">
          <span class="task-icon"><Timer size={18} strokeWidth={1.8} /></span>
          <div>
            <small>Now · running for {duration(phaseElapsedMs)}</small>
            <strong>{currentTaskLabel}</strong>
            <span>
              {currentTaskRemainingMs == null
                ? "Current task ETA is still learning from hardware samples."
                : `≈ ${duration(currentTaskRemainingMs)} until the next task`}
            </span>
          </div>
        </article>
        <span class="task-arrow"><ArrowRight size={22} strokeWidth={1.6} /></span>
        <article class="task-card next">
          <span class="task-index">NEXT</span>
          <div>
            <small>
              {currentTaskRemainingMs == null
                ? "Starts after the current hardware task"
                : `Starts in ≈ ${duration(currentTaskRemainingMs)}`}
            </small>
            <strong>{nextTaskInfo.label}</strong>
            <span>{nextTaskInfo.durationMs == null ? "Duration pending live evidence" : `Expected duration ≈ ${duration(nextTaskInfo.durationMs)}`}</span>
          </div>
        </article>
      </div>
    {/if}
    <div class="timing-grid" aria-live="polite">
      <div class="timing-metric">
        <span>Remaining</span>
        <strong>{powerRunning && estimatedRemainingMs != null ? `≈ ${duration(estimatedRemainingMs)}` : "—"}</strong>
        <small>{estimatedFinish ? `Estimated finish at ${estimatedFinish}.` : "Updates after each reported dwell."}</small>
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
    {#if isPaused && !powerSweep?.resume_available}
      <p class="resume-contract-note" role="status">
        Resume unavailable: {powerSweep?.resume_block_reason ?? "Core did not provide a build, hardware and driver compatibility verdict."}
      </p>
    {/if}
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
  .btn.continue {
    border-color: rgba(126, 184, 78, 0.48);
    background: rgba(92, 145, 54, 0.15);
    color: #bce49a;
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
  .live-target {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.75rem;
  }
  .sweep-progress-head span {
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
  .estimate-stage .clock-domain {
    display: block;
    margin-top: 0.22rem;
    color: var(--muted);
    font-size: 0.7rem;
    font-variant-numeric: tabular-nums;
    line-height: 1.35;
    text-align: left;
    text-wrap: pretty;
  }
  .estimate-stage small {
    max-width: 28rem;
    color: var(--muted);
    font-size: 0.72rem;
    line-height: 1.35;
    text-align: right;
    text-wrap: pretty;
  }
  .run-state.paused {
    border-color: rgba(126, 184, 78, 0.42);
    background: rgba(92, 145, 54, 0.12);
    color: #bce49a;
  }
  .task-timeline {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto minmax(0, 1fr);
    align-items: stretch;
    gap: 0.7rem;
    margin-top: 0.55rem;
  }
  .task-card {
    display: grid;
    min-width: 0;
    grid-template-columns: 42px minmax(0, 1fr);
    align-items: center;
    gap: 0.7rem;
    min-height: 94px;
    padding: 0.75rem;
    background: rgba(5, 7, 11, 0.42);
    box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.055);
  }
  .task-card.current {
    background: rgba(214, 168, 93, 0.075);
    box-shadow: inset 0 0 0 1px rgba(214, 168, 93, 0.2);
  }
  .task-icon,
  .task-index {
    display: grid;
    width: 42px;
    min-height: 42px;
    place-items: center;
    color: var(--forge-gold);
    background: rgba(214, 168, 93, 0.1);
    box-shadow: inset 0 0 0 1px rgba(214, 168, 93, 0.22);
  }
  .task-index {
    color: var(--nord-dim);
    font-size: 0.58rem;
    font-weight: 800;
    letter-spacing: 0.08em;
  }
  .task-card > div {
    display: flex;
    min-width: 0;
    flex-direction: column;
    gap: 0.24rem;
  }
  .task-card small {
    color: var(--nord-dim);
    font-size: 0.66rem;
    font-variant-numeric: tabular-nums;
  }
  .task-card strong {
    color: var(--text);
    font-size: 0.86rem;
    font-weight: 650;
    text-wrap: balance;
  }
  .task-card > div > span {
    color: var(--muted);
    font-size: 0.7rem;
    line-height: 1.35;
    text-wrap: pretty;
  }
  .task-arrow {
    align-self: center;
    color: var(--nord-dim);
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
  .resume-contract-note {
    margin: 0.6rem 0 0;
    border-left: 2px solid var(--forge-gold);
    padding: 0.5rem 0.65rem;
    color: var(--muted);
    background: rgba(214, 168, 93, 0.05);
    font-size: 0.72rem;
    line-height: 1.45;
    text-wrap: pretty;
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
    .timing-grid,
    .task-timeline {
      grid-template-columns: 1fr;
    }
    .task-arrow {
      justify-self: center;
      transform: rotate(90deg);
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
