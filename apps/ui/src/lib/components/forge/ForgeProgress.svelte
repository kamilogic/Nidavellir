<script>
  import { Activity, ArrowRight, Clock3, Play, ShieldCheck, Square, Timer } from "@lucide/svelte";

  let {
    powerSweep = null,
    powerRunning = false,
    safeLoop = null,
    forgeMode = "standard",
    onStopPower,
    onRecoverContinue,
    onResumePower,
  } = $props();

  let now = $state(Date.now());
  let observedElapsed = $state(null);
  let observedRemaining = $state(null);
  let elapsedBase = $state(null);
  let remainingBase = $state(null);
  let timingObservedAt = $state(Date.now());
  let observedTask = $state(null);
  let taskElapsedBase = $state(0);
  let taskObservedAt = $state(Date.now());

  const hasRun = $derived(Boolean(powerSweep && powerSweep.phase !== "idle"));
  const isInterrupted = $derived(powerSweep?.phase === "interrupted");
  const isPaused = $derived(powerSweep?.phase === "paused");
  const isStopping = $derived(powerSweep?.phase === "stopping");
  const isFinished = $derived(["finished", "provisional"].includes(powerSweep?.phase));
  const reportedElapsedMs = $derived(validDuration(powerSweep?.elapsed_ms));
  const reportedRemainingMs = $derived(validDuration(powerSweep?.estimated_remaining_ms));
  const currentTaskReportedMs = $derived(validDuration(powerSweep?.current_task_elapsed_ms) ?? 0);
  const elapsedMs = $derived(
    elapsedBase == null
      ? null
      : elapsedBase + (powerRunning ? Math.max(0, now - timingObservedAt) : 0),
  );
  const remainingMs = $derived(
    remainingBase == null
      ? null
      : Math.max(0, remainingBase - (powerRunning ? Math.max(0, now - timingObservedAt) : 0)),
  );
  const taskElapsedMs = $derived(
    powerRunning
      ? taskElapsedBase + Math.max(0, now - taskObservedAt)
      : taskElapsedBase,
  );
  const taskEstimatedTotalMs = $derived(validDuration(powerSweep?.current_task_estimated_total_ms));
  const taskRemainingMs = $derived(
    taskEstimatedTotalMs == null ? null : Math.max(0, taskEstimatedTotalMs - taskElapsedMs),
  );
  const estimatedTotalMs = $derived.by(() => {
    if (elapsedMs == null) return null;
    return remainingMs == null ? elapsedMs : elapsedMs + remainingMs;
  });
  const phaseInfo = $derived(stageInfo(powerSweep?.phase, powerRunning));
  const currentTaskLabel = $derived(taskLabel(powerSweep?.current_task) ?? phaseInfo.label);
  const nextTaskLabel = $derived(
    taskLabel(powerSweep?.next_task) ?? nextStageLabel(powerSweep?.phase),
  );
  const nextTaskDurationMs = $derived(validDuration(powerSweep?.next_task_estimated_duration_ms));
  const completedSteps = $derived(Number(powerSweep?.completed_steps ?? 0));
  const totalSteps = $derived(Number(powerSweep?.total_steps_estimate ?? 0));
  const progressPercent = $derived.by(() => {
    if (isFinished) return 100;
    if (totalSteps > 0) return clampPercent((completedSteps / totalSteps) * 100);
    if (powerRunning && elapsedMs != null && remainingMs != null && elapsedMs + remainingMs > 0) {
      return clampPercent((elapsedMs / (elapsedMs + remainingMs)) * 100);
    }
    return powerRunning ? 3 : 0;
  });
  const estimatedFinish = $derived.by(() => {
    if (isFinished) return "Complete";
    if (!powerRunning) return "—";
    if (remainingMs == null) return "Calculating";
    return new Intl.DateTimeFormat(undefined, { hour: "2-digit", minute: "2-digit" }).format(
      new Date(now + remainingMs),
    );
  });
  const canResume = $derived(Boolean(!powerRunning && isPaused && powerSweep?.resume_available));
  const safetyLabel = $derived.by(() => {
    if (!safeLoop) return "Protection pending";
    if (safeLoop.safe_mode || safeLoop.state === "unstable") return "Needs attention";
    if (safeLoop.boot_flag_armed || safeLoop.recovery_pending_ack) return "Recovery ready";
    return "Protected";
  });
  const title = $derived(
    isInterrupted ? "Forge interrupted" : isPaused ? "Forge paused" : isFinished ? "Forge complete" : powerRunning ? "Forging your GPU" : "Forge progress",
  );
  const runState = $derived(
    isStopping ? "Stopping safely" : powerRunning ? "Running" : isInterrupted ? "Interrupted" : isPaused ? "Paused" : isFinished ? "Complete" : "Idle",
  );

  function validDuration(value) {
    if (value == null || value === "") return null;
    const number = Number(value);
    return Number.isFinite(number) && number >= 0 ? number : null;
  }

  function clampPercent(value) {
    return Math.min(100, Math.max(0, value));
  }

  function duration(value) {
    const milliseconds = validDuration(value);
    if (milliseconds == null) return "Calculating";
    const totalSeconds = Math.max(0, Math.round(milliseconds / 1000));
    const hours = Math.floor(totalSeconds / 3600);
    const minutes = Math.floor((totalSeconds % 3600) / 60);
    const seconds = totalSeconds % 60;
    if (hours) return `${hours}h ${minutes}m`;
    if (minutes) return `${minutes}m ${seconds}s`;
    return `${seconds}s`;
  }

  function taskLabel(task) {
    if (!task) return null;
    const labels = {
      prepare_stock: "Preparing a clean stock state",
      stock_preheat: "Normalizing temperature and stock clock",
      capture_goldens: "Capturing stock render references",
      frontier_descent: "Mapping the stable core frontier",
      power_calibration: "Measuring real Apply power",
      profile_synthesis: "Forging the three profile goals",
      apply_calibration: "Measuring exact Apply pairs",
      synthesize_profiles: "Forging the three profile goals",
      apply_qualification: "Stress-testing the final Apply pairs",
      publish_profiles: "Finishing the forged profiles",
      final_stock_reset: "Restoring and verifying stock state",
      final_reset: "Restoring and verifying stock state",
    };
    return labels[task] ?? String(task).replaceAll("_", " ");
  }

  function stageInfo(phase, running) {
    if (!running && phase === "finished") {
      return { label: "Profiles forged", detail: "The run finished and the GPU returned to a verified state." };
    }
    if (!running && phase === "provisional") {
      return { label: "Preview complete", detail: "The map is ready, but final qualification is still required." };
    }
    if (!running && phase === "paused") {
      return { label: "Learning saved safely", detail: "The GPU is at stock and this compatible run can be resumed." };
    }
    if (!running && phase === "interrupted") {
      return { label: "Recovery required", detail: "Saved learning is intact; review recovery before continuing." };
    }
    if (!running) return { label: "Ready to forge", detail: "Progress appears here when the next run begins." };
    const stages = {
      preheat: ["Preparing the forge", "Normalizing the GPU before the first measurement."],
      power: ["Finding sustainable performance", "Locating the highest clock the hardware can hold cleanly."],
      descend: ["Testing the stability frontier", "Searching the real voltage boundary of each useful clock."],
      calibrate: ["Measuring profile power", "Recording real power at the exact Apply points."],
      synthesize: ["Forging the profiles", "Selecting performance, balance and efficiency from measured evidence."],
      "apply-qualify": ["Tempering the final profiles", "Texture Hop v13-r3 and Endurance are challenging each final Apply point."],
      stopping: ["Stopping safely", "Saving learning and returning the GPU to stock."],
    };
    const [label, detail] = stages[phase] ?? ["Refining the forge", "The next estimate arrives with the current hardware task."];
    return { label, detail };
  }

  function nextStageLabel(phase) {
    const labels = {
      preheat: "Find sustainable performance",
      power: "Map the stability frontier",
      descend: "Test the next hardware candidate",
      calibrate: "Forge the three profile goals",
      synthesize: "Temper the final Apply pairs",
      "apply-qualify": "Publish the forged profiles",
      stopping: "Confirm the safe stock state",
    };
    return labels[phase] ?? "Prepare the next forge stage";
  }

  $effect(() => {
    if (
      reportedElapsedMs !== observedElapsed ||
      reportedRemainingMs !== observedRemaining
    ) {
      observedElapsed = reportedElapsedMs;
      observedRemaining = reportedRemainingMs;
      elapsedBase = reportedElapsedMs;
      remainingBase = reportedRemainingMs;
      timingObservedAt = Date.now();
    }
  });

  $effect(() => {
    const task = powerSweep?.current_task ?? null;
    if (task !== observedTask || currentTaskReportedMs > taskElapsedBase) {
      observedTask = task;
      taskElapsedBase = currentTaskReportedMs;
      taskObservedAt = Date.now();
    }
  });

  $effect(() => {
    const interval = setInterval(() => (now = Date.now()), 1000);
    return () => clearInterval(interval);
  });
</script>

<section class="forge-progress" aria-labelledby="forge-progress-title">
  <header class="progress-header">
    <div class="progress-heading">
      <span class="eyebrow">Forge progress</span>
      <h3 id="forge-progress-title"><Activity size={19} strokeWidth={1.8} />{title}</h3>
      <p>{phaseInfo.detail}</p>
    </div>
    <div class="progress-actions">
      <span class="safety-pill"><ShieldCheck size={14} strokeWidth={1.9} />{safetyLabel}</span>
      <span class:live={powerRunning} class:warning={isInterrupted} class="run-pill">{runState}</span>
      {#if powerRunning}
        <button class="progress-button stop" type="button" onclick={onStopPower} disabled={isStopping}>
          <Square size={14} strokeWidth={1.9} />{isStopping ? "Stopping…" : "Stop safely"}
        </button>
      {:else if canResume}
        <button class="progress-button resume" type="button" onclick={onResumePower}>
          <Play size={14} strokeWidth={1.9} />Resume Forge
        </button>
      {:else if isInterrupted}
        <button class="progress-button resume" type="button" onclick={() => onRecoverContinue?.(forgeMode)}>
          <Play size={14} strokeWidth={1.9} />Review & continue
        </button>
      {/if}
    </div>
  </header>

  <div class="progress-overview">
    <div class="progress-copy">
      <span>{phaseInfo.label}</span>
      <strong>{Math.round(progressPercent)}%</strong>
    </div>
    <div
      class="progress-track"
      class:forging={powerRunning}
      role="progressbar"
      aria-label="Estimated Forge completion"
      aria-valuemin="0"
      aria-valuemax="100"
      aria-valuenow={Math.round(progressPercent)}
    >
      <span style={`width: ${progressPercent}%`}></span>
    </div>
  </div>

  {#if powerRunning}
    <div class="task-flow" aria-live="polite">
      <article class="task-card current">
        <span class="task-icon"><Timer size={20} strokeWidth={1.75} /></span>
        <div>
          <small>Now · running for {duration(taskElapsedMs)}</small>
          <strong>{currentTaskLabel}</strong>
          <p>{taskRemainingMs == null ? "The live estimate is still settling." : `About ${duration(taskRemainingMs)} until the next stage.`}</p>
        </div>
      </article>
      <span class="task-arrow"><ArrowRight size={21} strokeWidth={1.6} /></span>
      <article class="task-card next">
        <span class="task-icon next-icon">NEXT</span>
        <div>
          <small>{taskRemainingMs == null ? "Starts after the current stage" : `Starts in about ${duration(taskRemainingMs)}`}</small>
          <strong>{nextTaskLabel}</strong>
          <p>{nextTaskDurationMs == null ? "Duration updates from live hardware evidence." : `Expected duration: ${duration(nextTaskDurationMs)}.`}</p>
        </div>
      </article>
    </div>
  {/if}

  <div class="run-timing" aria-live="polite">
    <article>
      <Clock3 size={17} strokeWidth={1.7} />
      <span>Elapsed<strong>{hasRun && elapsedMs != null ? duration(elapsedMs) : "—"}</strong></span>
    </article>
    <article>
      <Timer size={17} strokeWidth={1.7} />
      <span>Remaining<strong>{powerRunning && remainingMs != null ? duration(remainingMs) : "—"}</strong></span>
    </article>
    <article>
      <Activity size={17} strokeWidth={1.7} />
      <span>Estimated total<strong>{hasRun && estimatedTotalMs != null ? duration(estimatedTotalMs) : "—"}</strong></span>
    </article>
    <article>
      <ArrowRight size={17} strokeWidth={1.7} />
      <span>Estimated finish<strong>{estimatedFinish}</strong></span>
    </article>
  </div>

  {#if isPaused && !powerSweep?.resume_available}
    <p class="resume-note" role="status">
      Resume unavailable: {powerSweep?.resume_block_reason ?? "This checkpoint no longer matches the current program, GPU or driver."}
    </p>
  {/if}
</section>

<style>
  .forge-progress {
    --panel-radius: 11px;
    --inner-radius: 9px;
    display: grid;
    gap: 14px;
    padding: 18px;
    border: 0;
    border-radius: var(--panel-radius);
    background: var(--progress-surface, rgba(7, 10, 12, 0.58));
    box-shadow: inset 0 0 0 1px var(--progress-outline, rgba(126, 136, 143, 0.34));
    color: #e3e3df;
    font-family: inherit;
  }

  .progress-header,
  .progress-actions,
  .progress-copy,
  .run-timing article {
    display: flex;
    align-items: center;
  }

  .progress-header {
    justify-content: space-between;
    gap: 18px;
  }

  .progress-heading {
    min-width: 0;
  }

  .eyebrow {
    display: block;
    margin-bottom: 5px;
    color: #737d82;
    font-size: 0.68rem;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }

  h3 {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 0;
    color: #e3e3df;
    font-size: 1.125rem;
    font-weight: 580;
    letter-spacing: -0.01em;
  }

  .progress-heading p {
    max-width: 700px;
    margin: 5px 0 0;
    color: #92999d;
    font-size: 0.75rem;
    line-height: 1.5;
    text-wrap: pretty;
  }

  .progress-actions {
    justify-content: flex-end;
    gap: 8px;
    flex-wrap: wrap;
  }

  .safety-pill,
  .run-pill {
    display: inline-flex;
    min-height: 30px;
    align-items: center;
    gap: 6px;
    border-radius: 999px;
    padding: 0 10px;
    background: rgba(126, 184, 78, 0.1);
    box-shadow: inset 0 0 0 1px rgba(126, 184, 78, 0.34);
    color: #bce49a;
    font-size: 0.65rem;
    font-weight: 780;
    letter-spacing: 0.07em;
    text-transform: uppercase;
    white-space: nowrap;
  }

  .run-pill {
    background: rgba(255, 255, 255, 0.035);
    box-shadow: inset 0 0 0 1px var(--forge-line);
    color: var(--nord-dim);
  }

  .run-pill.live {
    background: rgba(214, 168, 93, 0.1);
    box-shadow: inset 0 0 0 1px rgba(214, 168, 93, 0.38);
    color: var(--forge-gold);
  }

  .run-pill.warning {
    background: rgba(191, 97, 106, 0.12);
    box-shadow: inset 0 0 0 1px rgba(191, 97, 106, 0.4);
    color: #f3b9bd;
  }

  .progress-button {
    display: inline-flex;
    min-height: 40px;
    align-items: center;
    justify-content: center;
    gap: 7px;
    border: 0;
    border-radius: 9px;
    padding: 0 13px;
    background: rgba(126, 184, 78, 0.13);
    box-shadow: inset 0 0 0 1px rgba(126, 184, 78, 0.4);
    color: #c6eba8;
    font: inherit;
    font-size: 0.75rem;
    font-weight: 720;
    cursor: pointer;
    transition: background-color 150ms ease, box-shadow 150ms ease, transform 100ms ease;
  }

  .progress-button.stop {
    background: rgba(191, 97, 106, 0.12);
    box-shadow: inset 0 0 0 1px rgba(191, 97, 106, 0.4);
    color: #f3b9bd;
  }

  .progress-button:hover:not(:disabled) {
    background-color: rgba(214, 168, 93, 0.16);
    box-shadow: inset 0 0 0 1px rgba(214, 168, 93, 0.48);
  }

  .progress-button:active:not(:disabled) {
    transform: scale(0.96);
  }

  .progress-button:disabled {
    cursor: wait;
    opacity: 0.58;
  }

  .progress-overview {
    padding: 14px;
    border-radius: var(--inner-radius);
    background: rgba(0, 0, 0, 0.2);
    box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.055);
  }

  .progress-copy {
    justify-content: space-between;
    gap: 12px;
    color: #92999d;
    font-size: 0.75rem;
  }

  .progress-copy strong {
    color: var(--forge-gold);
    font-size: 0.86rem;
    font-variant-numeric: tabular-nums;
  }

  .progress-track {
    height: 10px;
    margin-top: 10px;
    overflow: hidden;
    border-radius: 999px;
    background: rgba(0, 0, 0, 0.52);
    box-shadow: inset 0 0 0 1px rgba(214, 168, 93, 0.18);
  }

  .progress-track > span {
    position: relative;
    display: block;
    height: 100%;
    overflow: hidden;
    border-radius: inherit;
    background: linear-gradient(90deg, #9a6039, var(--forge-gold), #a6cf69);
    box-shadow: 0 0 16px rgba(214, 168, 93, 0.28);
    transition: width 240ms ease-out;
  }

  .progress-track.forging > span::after {
    position: absolute;
    inset: 0;
    content: "";
    background: linear-gradient(
      100deg,
      transparent 20%,
      rgba(255, 245, 215, 0.08) 38%,
      rgba(255, 250, 229, 0.5) 50%,
      rgba(255, 245, 215, 0.08) 62%,
      transparent 80%
    );
    transform: translateX(-140%);
    animation: forge-progress-sheen 1.9s linear infinite;
    will-change: transform;
  }

  @keyframes forge-progress-sheen {
    to { transform: translateX(140%); }
  }

  .task-flow {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 28px minmax(0, 1fr);
    align-items: center;
    gap: 10px;
  }

  .task-card {
    display: grid;
    min-width: 0;
    min-height: 108px;
    grid-template-columns: 44px minmax(0, 1fr);
    align-items: center;
    gap: 12px;
    padding: 14px;
    border-radius: var(--inner-radius);
    background: rgba(0, 0, 0, 0.2);
    box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.055);
  }

  .task-card.current {
    background: rgba(214, 168, 93, 0.07);
    box-shadow: inset 0 0 0 1px rgba(214, 168, 93, 0.22);
  }

  .task-icon {
    display: grid;
    width: 44px;
    height: 44px;
    place-items: center;
    border-radius: 9px;
    background: rgba(214, 168, 93, 0.11);
    box-shadow: inset 0 0 0 1px rgba(214, 168, 93, 0.24);
    color: var(--forge-gold);
  }

  .next-icon {
    color: var(--nord-dim);
    font-size: 0.58rem;
    font-weight: 800;
    letter-spacing: 0.08em;
  }

  .task-card div {
    min-width: 0;
  }

  .task-card small {
    color: var(--nord-dim);
    font-size: 0.67rem;
    font-variant-numeric: tabular-nums;
  }

  .task-card strong {
    display: block;
    margin-top: 4px;
    color: #d7d9d7;
    font-size: 0.88rem;
    font-weight: 670;
    text-wrap: balance;
  }

  .task-card p {
    margin: 5px 0 0;
    color: #92999d;
    font-size: 0.72rem;
    line-height: 1.4;
    text-wrap: pretty;
  }

  .task-arrow {
    display: grid;
    place-items: center;
    color: rgba(214, 168, 93, 0.65);
  }

  .run-timing {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: 8px;
  }

  .run-timing article {
    min-width: 0;
    gap: 9px;
    padding: 11px 12px;
    border-radius: 9px;
    background: rgba(255, 255, 255, 0.025);
    box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.05);
    color: var(--nord-dim);
  }

  .run-timing span {
    min-width: 0;
    color: var(--nord-dim);
    font-size: 0.62rem;
    font-weight: 760;
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }

  .run-timing strong {
    display: block;
    margin-top: 3px;
    overflow: hidden;
    color: #d7d9d7;
    font-size: 0.82rem;
    font-variant-numeric: tabular-nums;
    text-overflow: ellipsis;
    text-transform: none;
    white-space: nowrap;
  }

  .resume-note {
    margin: 0;
    border-radius: 9px;
    padding: 10px 12px;
    background: rgba(191, 97, 106, 0.09);
    box-shadow: inset 0 0 0 1px rgba(191, 97, 106, 0.28);
    color: #e8b4b8;
    font-size: 0.75rem;
    line-height: 1.45;
  }

  @media (max-width: 940px) {
    .progress-header {
      align-items: flex-start;
      flex-direction: column;
    }
    .progress-actions {
      justify-content: flex-start;
    }
    .run-timing {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }
  }

  @media (max-width: 680px) {
    .forge-progress {
      padding: 14px;
    }
    .task-flow {
      grid-template-columns: 1fr;
    }
    .task-arrow {
      transform: rotate(90deg);
    }
    .run-timing {
      grid-template-columns: 1fr;
    }
    .progress-button {
      min-height: 44px;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .progress-track > span,
    .progress-button {
      transition: none;
    }
    .progress-track.forging > span::after {
      animation: none;
    }
  }
</style>
