<script>
  import {
    Activity,
    AlertTriangle,
    ArrowLeft,
    CheckCircle2,
    ExternalLink,
    FileDown,
    FlaskConical,
    Gauge,
    Play,
    Radio,
    RotateCcw,
    ShieldCheck,
    Square,
    Terminal,
  } from "@lucide/svelte";
  import LogTerminal from "./LogTerminal.svelte";

  let {
    theme = "command",
    embedded = false,
    powerSweep = null,
    logLines = [],
    exporting = false,
    exportMsg = "",
    exportFailed = false,
    sentinel = null,
    safeLoop = null,
    sentinelState = "No events",
    sentinelSummary = "No automatic recovery action recorded.",
    gameTrace = null,
    gameTraceBusy = false,
    gameTraceActionError = "",
    gameTraceExportBusy = false,
    gameTraceExportMsg = "",
    manualPoint = null,
    manualPointBusy = false,
    manualPointActionError = "",
    detectorLab = null,
    detectorLabBusy = false,
    detectorLabActionError = "",
    onExportLog,
    onToggleGameTrace,
    onOpenGameTraceLog,
    onApplyManualPoint,
    onResetManualPoint,
    onStartDetectorLab,
    onStopDetectorLab,
    onOpenDetectorLabLog,
    onClose,
  } = $props();

  let activeTab = $state("log");
  const blacklist = $derived(safeLoop?.blacklist ?? []);
  const condemnations = $derived.by(() =>
    [...(safeLoop?.condemnations ?? [])].sort((a, b) => {
      const rehabilitationOrder = Number(Boolean(a?.rehabilitated)) - Number(Boolean(b?.rehabilitated));
      if (rehabilitationOrder) return rehabilitationOrder;
      return String(b?.timestamp ?? "").localeCompare(String(a?.timestamp ?? ""));
    }),
  );
  const activeCondemnations = $derived(condemnations.filter((event) => !event?.rehabilitated).length);

  const tabs = $derived([
    {
      id: "log",
      label: "Live Log",
      detail: `${logLines.length} ${logLines.length === 1 ? "line" : "lines"}`,
      icon: Terminal,
    },
    {
      id: "sentinel",
      label: "Sentinel",
      detail: activeCondemnations || blacklist.length
        ? `${activeCondemnations + blacklist.length} safety records`
        : sentinelState,
      icon: ShieldCheck,
    },
    {
      id: "game-trace",
      label: "Game Trace",
      detail: gameTrace?.running ? `Gravando · ${gameTrace.samples ?? 0}` : "Parado",
      icon: Radio,
    },
    {
      id: "manual-point",
      label: "Manual point",
      detail: detectorLab?.running
        ? `Lab · ${formatLabPhase(detectorLab.current_phase ?? detectorLab.stage)}`
        : manualPoint?.active
        ? `${manualPoint.target_mhz} MHz · ${manualPoint.resolved_voltage_mv} mV`
        : "Temporary curve",
      icon: Gauge,
    },
  ]);

  const terminalStatus = $derived(
    powerSweep?.running ? powerSweep.phase : logLines.length ? "ready" : "idle",
  );
  const traceReady = $derived(Boolean(gameTrace));
  let manualTargetMhz = $state("");
  let manualVoltageMv = $state("");
  let labRecipe = $state("dense_v14");
  let labDurationS = $state(60);
  const manualPointValid = $derived(
    Number(manualTargetMhz) >= 300 &&
      Number(manualTargetMhz) <= 4000 &&
      Number(manualVoltageMv) >= 500 &&
      Number(manualVoltageMv) <= 1250,
  );
  const labProgress = $derived(
    Math.max(0, Math.min(100, Number(detectorLab?.progress_pct) || 0)),
  );
  const gpuRebootRequired = $derived(
    detectorLab?.stage === "reboot_required" || detectorLab?.result === "tdr",
  );
  const detectorLabCanStart = $derived(
    Boolean(manualPoint?.active && manualPoint?.verified) &&
      !gpuRebootRequired &&
      !detectorLab?.running &&
      !detectorLabBusy,
  );

  function applyManualPoint() {
    if (!manualPointValid || manualPoint?.active) return;
    onApplyManualPoint?.({
      target_mhz: Number(manualTargetMhz),
      voltage_mv: Number(manualVoltageMv),
    });
  }

  function startDetectorLab() {
    if (!detectorLabCanStart) return;
    onStartDetectorLab?.({
      recipe: labRecipe,
      duration_s: Number(labDurationS),
    });
  }

  function formatLabPhase(value) {
    const phase = String(value ?? "idle").replaceAll("_", " ").replaceAll("-", " ");
    return phase.replace(/\b\w/g, (letter) => letter.toUpperCase());
  }

  function detectorResultLabel(result) {
    const labels = {
      stable: "No error detected",
      silent_error: "Silent error detected",
      unstable: "Instability detected",
      crash: "Driver or device failure",
      tdr: "TDR detected · reboot required",
      stopped: "Stopped safely",
      inconclusive: "Point authority not proven",
      environment_error: "Environment error",
    };
    return labels[result] ?? "Awaiting a run";
  }

  function formatElapsed(seconds) {
    const total = Math.max(0, Number(seconds) || 0);
    const hours = Math.floor(total / 3600);
    const minutes = Math.floor((total % 3600) / 60);
    const secs = total % 60;
    return hours > 0
      ? `${hours.toString().padStart(2, "0")}:${minutes.toString().padStart(2, "0")}:${secs.toString().padStart(2, "0")}`
      : `${minutes.toString().padStart(2, "0")}:${secs.toString().padStart(2, "0")}`;
  }

  function metric(value, suffix = "") {
    return value == null || value === "" ? "—" : `${value}${suffix}`;
  }

  function traceNote(note) {
    const value = String(note ?? "").trim();
    if (!value) return gameTrace?.running ? "Recording the current workload." : "No game trace recorded yet.";
    if (value === "iniciando") return "Starting the recorder…";
    if (value.startsWith("parado")) return value.replace("parado", "Stopped").replace("amostras", "samples");
    if (value.startsWith("erro:")) return value.replace("erro:", "Error:");
    return value;
  }

  function blacklistAxes(region) {
    const axes = region?.center?.axes ?? {};
    const labels = {
      gpu_freq_mhz: ["Clock", "MHz"],
      gpu_vf_bin_mv: ["VF bin", "mV"],
      gpu_offset_mhz: ["Offset", "MHz"],
      gpu_mem_offset_mhz: ["Memory offset", "MHz"],
    };
    return Object.entries(axes).map(([key, value]) => {
      const [label, unit] = labels[key] ?? [key.replaceAll("_", " "), ""];
      return `${label} ${value}${unit ? ` ${unit}` : ""}`;
    });
  }

  function condemnationPoint(event) {
    if (event?.target_mhz == null && event?.vf_bin_mv == null) return "Unattributed hardware event";
    const clock = event?.target_mhz == null ? "unknown clock" : `${event.target_mhz} MHz`;
    const voltage = event?.vf_bin_mv == null ? "unknown VF bin" : `${event.vf_bin_mv} mV`;
    return `${clock} @ ${voltage}`;
  }

  function condemnationReason(event) {
    const kind = String(event?.kind ?? "stability evidence").replaceAll("-", " ").replaceAll("_", " ");
    return event?.note ? `${kind} · ${event.note}` : kind;
  }
</script>

<section class={`advanced-hub ${theme}`} class:embedded aria-labelledby="advanced-diagnostics-title">
  <header class="hub-header">
    <div>
      <span class="eyebrow">Technical workspace</span>
      <h2 id="advanced-diagnostics-title">Advanced Diagnostics</h2>
      <p>Live Forge activity, automatic protection and read-only game telemetry.</p>
    </div>
    <button class="back-button" type="button" onclick={onClose}>
      <ArrowLeft size={17} strokeWidth={1.8} />
      <span>Back to Forge</span>
    </button>
  </header>

  <div class="diagnostic-tabs" role="tablist" aria-label="Advanced diagnostic tools">
    {#each tabs as tab}
      <button
        type="button"
        role="tab"
        aria-selected={activeTab === tab.id}
        aria-controls={`diagnostic-panel-${tab.id}`}
        class:active={activeTab === tab.id}
        onclick={() => (activeTab = tab.id)}
      >
        <tab.icon size={18} strokeWidth={1.75} />
        <span>{tab.label}</span>
        <small aria-hidden="true">{tab.detail}</small>
      </button>
    {/each}
  </div>

  {#if activeTab === "log"}
    <div
      class="diagnostic-panel live-log-panel"
      id="diagnostic-panel-log"
      role="tabpanel"
      aria-label="Live Log"
    >
      <div class="panel-toolbar">
        <div>
          <span class="panel-kicker">Real-time activity</span>
          <h3>Live technical log</h3>
          <p>Follow every Forge stage and decision as it happens.</p>
        </div>
        <button class="action-button primary" type="button" onclick={onExportLog} disabled={exporting}>
          <FileDown size={17} strokeWidth={1.8} />
          <span>{exporting ? "Exporting…" : "Export full log"}</span>
        </button>
      </div>

      {#if exportMsg}
        <p class="inline-message" class:error={exportFailed} role="status">{exportMsg}</p>
      {/if}

      <div class="terminal-shell">
        <LogTerminal
          title="nidavellir / live forge log"
          status={terminalStatus}
          live={Boolean(powerSweep?.running)}
          lines={logLines.length ? logLines : ["No events yet. Start Forge GPU to see each step here."]}
          runningText={powerSweep?.running ? `${powerSweep.phase}…` : null}
        />
      </div>
      <p class="panel-footnote">
        The live view keeps the latest 240 lines. Export includes the persisted Forge report and recorded dwell evidence.
      </p>
    </div>
  {:else if activeTab === "sentinel"}
    <div
      class="diagnostic-panel sentinel-panel"
      id="diagnostic-panel-sentinel"
      role="tabpanel"
      aria-label="Sentinel"
    >
      <div class="panel-toolbar">
        <div>
          <span class="panel-kicker">Automatic recovery</span>
          <h3>Sentinel</h3>
          <p>Shows the latest protective action taken after a real-world stability event.</p>
        </div>
        <span class={`state-readout ${sentinel?.action ?? "idle"}`}>
          <i aria-hidden="true"></i>
          {sentinelState}
        </span>
      </div>

      <dl class="sentinel-readout">
        <div>
          <dt>Last action</dt>
          <dd>{sentinelSummary}</dd>
        </div>
        <div>
          <dt>Recommendation</dt>
          <dd>{sentinel?.recommendation ?? "No follow-up is needed. Sentinel records recovery actions automatically."}</dd>
        </div>
        <div>
          <dt>Recorded</dt>
          <dd>{sentinel?.ts ?? "—"}</dd>
        </div>
      </dl>

      <section class="condemnation-log" aria-labelledby="condemnation-log-title">
        <div class="blacklist-heading">
          <div>
            <span class="panel-kicker">Condemnation ledger</span>
            <h4 id="condemnation-log-title">Rejected hardware points</h4>
          </div>
          <span class:active={activeCondemnations > 0}>{activeCondemnations}</span>
        </div>
        {#if condemnations.length}
          <ol>
            {#each condemnations as event, index}
              <li class:rehabilitated={event?.rehabilitated}>
                <span class="blacklist-index">{String(index + 1).padStart(2, "0")}</span>
                <div>
                  <div class="ledger-title">
                    <strong>{condemnationPoint(event)}</strong>
                    <span class={`severity ${event?.severity ?? "unknown"}`}>{event?.severity ?? "recorded"}</span>
                    {#if event?.rehabilitated}<span class="rehabilitated-label">Rehabilitated</span>{/if}
                  </div>
                  <small>{condemnationReason(event)}</small>
                  <small class="ledger-meta">{event?.timestamp ?? "Timestamp unavailable"}{event?.run_id ? ` · run ${event.run_id}` : ""}</small>
                </div>
              </li>
            {/each}
          </ol>
        {:else}
          <p class="blacklist-empty">No condemnation events have been published for this hardware.</p>
        {/if}
      </section>

      <section class="blacklist-log" aria-labelledby="blacklist-log-title">
        <div class="blacklist-heading">
          <div>
            <span class="panel-kicker">Durable hardware evidence</span>
            <h4 id="blacklist-log-title">Blacklisted regions</h4>
          </div>
          <span class:active={blacklist.length > 0}>{blacklist.length}</span>
        </div>
        {#if blacklist.length}
          <ol>
            {#each blacklist as region, index}
              {@const axes = blacklistAxes(region)}
              <li>
                <span class="blacklist-index">{String(index + 1).padStart(2, "0")}</span>
                <div>
                  <strong>{axes.length ? axes.join(" · ") : "Hardware-local tuning region"}</strong>
                  <small>Safety radius ±{region?.radius ?? "—"} hardware step{region?.radius === 1 ? "" : "s"}</small>
                </div>
              </li>
            {/each}
          </ol>
        {:else}
          <p class="blacklist-empty">No tuning regions are currently blacklisted for this hardware.</p>
        {/if}
      </section>

      <p class="panel-footnote">
        Sentinel is automatic and has no manual switch here. This panel reports events; it does not change GPU tuning.
      </p>
    </div>
  {:else if activeTab === "game-trace"}
    <div
      class="diagnostic-panel game-trace-panel"
      id="diagnostic-panel-game-trace"
      role="tabpanel"
      aria-label="Game Trace"
    >
      <div class="trace-control">
        <div>
          <span class="panel-kicker">Read-only workload recorder</span>
          <h3>Game Trace</h3>
          <p>Grava telemetria da GPU (potência, clock, tensão, throttle) enquanto você joga, para depois comparar com o teste sintético. Somente leitura.</p>
        </div>
        <button
          class="trace-switch"
          class:on={Boolean(gameTrace?.running)}
          type="button"
          role="switch"
          aria-checked={Boolean(gameTrace?.running)}
          aria-label="Gravação do Game Trace"
          aria-busy={gameTraceBusy}
          onclick={onToggleGameTrace}
          disabled={!traceReady || gameTraceBusy}
        >
          <span class="switch-track" aria-hidden="true"><i></i></span>
          <span>{gameTraceBusy ? "Atualizando…" : gameTrace?.running ? "Parar gravação" : "Iniciar gravação"}</span>
        </button>
      </div>

      {#if gameTraceActionError}
        <p class="inline-message error trace-action-error" role="alert">{gameTraceActionError}</p>
      {/if}

      <p class="trace-note" class:recording={Boolean(gameTrace?.running)}>
        <i aria-hidden="true"></i>
        {traceReady ? traceNote(gameTrace?.note) : "Loading recorder status…"}
      </p>

      {#if gameTrace?.running}
        <p class="trace-live-status">
          {metric(gameTrace.samples)} amostras · {metric(gameTrace.elapsed_s, "s")} · {metric(gameTrace.last_power_w?.toFixed?.(0), " W")} · {metric(gameTrace.last_core_mhz, " MHz")} · {metric(gameTrace.last_volt_mv, " mV")}
        </p>
      {/if}

      <dl class="trace-metrics">
        <div>
          <dt>Samples</dt>
          <dd>{metric(gameTrace?.samples)}</dd>
        </div>
        <div>
          <dt>Elapsed</dt>
          <dd>{gameTrace ? formatElapsed(gameTrace.elapsed_s) : "—"}</dd>
        </div>
        <div>
          <dt>Power</dt>
          <dd>{metric(gameTrace?.last_power_w?.toFixed?.(0), " W")}</dd>
        </div>
        <div>
          <dt>Clock</dt>
          <dd>{metric(gameTrace?.last_core_mhz, " MHz")}</dd>
        </div>
        <div>
          <dt>Voltage</dt>
          <dd>{metric(gameTrace?.last_volt_mv, " mV")}</dd>
        </div>
      </dl>

      <div class="trace-export">
        <div>
          <span class="panel-kicker">Exported JSONL</span>
          {#if gameTrace?.out_path}
            <code>{gameTrace.out_path}</code>
          {:else}
            <p>Start Game Trace to create a timestamped log.</p>
          {/if}
        </div>
        <button
          class="action-button"
          type="button"
          onclick={onOpenGameTraceLog}
          disabled={!gameTrace?.out_path || gameTraceExportBusy}
        >
          <ExternalLink size={17} strokeWidth={1.8} />
          <span>{gameTraceExportBusy ? "Opening…" : "Open exported log"}</span>
        </button>
      </div>

      {#if gameTraceExportMsg}
        <p class="inline-message" role="status">{gameTraceExportMsg}</p>
      {/if}
    </div>
  {:else}
    <div
      class="diagnostic-panel manual-point-panel"
      id="diagnostic-panel-manual-point"
      role="tabpanel"
      aria-label="Manual diagnostic point"
    >
      <div class="panel-toolbar manual-toolbar">
        <div>
          <span class="panel-kicker">Temporary hardware point</span>
          <h3>Manual diagnostic point</h3>
          <p>Apply one authoritative V/F point for a real game test, without starting Forge or a synthetic workload.</p>
        </div>
        <span class:active={Boolean(manualPoint?.active)} class="manual-state">
          <i aria-hidden="true"></i>
          {manualPoint?.active ? "Applied" : "Inactive"}
        </span>
      </div>

      <div class="manual-safety-note">
        <ShieldCheck size={19} strokeWidth={1.75} />
        <p>
          <strong>Temporary and supervised.</strong> This replaces the current GPU curve but never
          becomes a saved profile. Safe Loop remains armed until you return to stock.
        </p>
      </div>

      <div class="manual-layout">
        <section class="manual-setup" aria-labelledby="manual-point-setup-title">
          <span class="panel-kicker">Point selection</span>
          <h4 id="manual-point-setup-title">Choose the clock and VF-bin request</h4>
          <p class="manual-copy">
            The voltage request is resolved to the nearest physical bin on this GPU. Requests more
            than 8 mV from a real bin are refused instead of being guessed. Clock may step down
            under load, but the selected voltage is locked and cannot be exceeded.
          </p>

          <div class="manual-point-fields">
            <label>
              <span>Target clock</span>
              <span class="numeric-input">
                <input
                  type="number"
                  min="300"
                  max="4000"
                  step="15"
                  placeholder="—"
                  bind:value={manualTargetMhz}
                  disabled={manualPoint?.active || detectorLab?.running}
                />
                <small>MHz</small>
              </span>
            </label>
            <label>
              <span>Requested VF bin</span>
              <span class="numeric-input">
                <input
                  type="number"
                  min="500"
                  max="1250"
                  step="1"
                  placeholder="—"
                  bind:value={manualVoltageMv}
                  disabled={manualPoint?.active || detectorLab?.running}
                />
                <small>mV</small>
              </span>
            </label>
          </div>

          {#if manualPointActionError}
            <p class="inline-message error" role="alert">{manualPointActionError}</p>
          {/if}

          <div class="manual-actions">
            {#if manualPoint?.active || detectorLab?.running}
              <button
                class="action-button reset-point"
                type="button"
                onclick={onResetManualPoint}
                disabled={manualPointBusy || detectorLab?.running}
              >
                <RotateCcw size={17} strokeWidth={1.8} />
                <span>{manualPointBusy ? "Returning to stock…" : "Return to stock"}</span>
              </button>
              <button
                class="action-button"
                type="button"
                onclick={() => (activeTab = "game-trace")}
                disabled={detectorLab?.running}
              >
                <Radio size={17} strokeWidth={1.8} />
                <span>Open Game Trace</span>
              </button>
            {:else}
              <button
                class="action-button primary"
                type="button"
                onclick={applyManualPoint}
                disabled={manualPointBusy || !manualPointValid || gpuRebootRequired}
              >
                <Gauge size={17} strokeWidth={1.8} />
                <span>{manualPointBusy ? "Applying…" : "Apply temporary point"}</span>
              </button>
            {/if}
          </div>
          <p class="manual-footnote">
            Applying clears any currently saved GPU profile. Returning to stock ends the diagnostic
            and disarms its recovery intent.
          </p>
        </section>

        <aside class="manual-live" aria-labelledby="manual-point-live-title" aria-live="polite">
          <span class="panel-kicker">Manual point status</span>
          <h4 id="manual-point-live-title">
            {detectorLab?.running
              ? "Detector Lab owns the point"
              : manualPoint?.active
                ? "Manual point verified"
                : "No manual point applied"}
          </h4>
          <p>{manualPoint?.note ?? "Enter a hardware-local point to begin."}</p>
          <dl>
            <div>
              <dt>Requested</dt>
              <dd>
                {manualPoint?.target_mhz == null
                  ? "—"
                  : `${manualPoint.target_mhz} MHz @ ${manualPoint.requested_voltage_mv} mV`}
              </dd>
            </div>
            <div>
              <dt>Physical VF bin</dt>
              <dd>
                {manualPoint?.resolved_voltage_mv == null
                  ? "Resolved before apply"
                  : `${manualPoint.resolved_voltage_mv} mV`}
              </dd>
            </div>
            <div>
              <dt>Point authority</dt>
              <dd class:verified={Boolean(manualPoint?.verified)}>
                {manualPoint?.verified ? "Clock ceiling + voltage lock verified" : "Not applied"}
              </dd>
            </div>
            <div>
              <dt>Next step</dt>
              <dd>
                {detectorLab?.running
                  ? formatLabPhase(detectorLab.stage)
                  : manualPoint?.active
                    ? "Compare a detector or start Game Trace"
                    : "Choose a point"}
              </dd>
            </div>
          </dl>
        </aside>
      </div>

      <section class="detector-lab" aria-labelledby="detector-lab-title">
        <header class="lab-heading">
          <div>
            <span class="panel-kicker">Experimental detector bake-off</span>
            <h4 id="detector-lab-title">Detector Lab</h4>
            <p>
              Compare the authoritative v25 Texture Hop with the denser v14 candidate at the same
              hardware-local point.
            </p>
          </div>
          <span class="lab-contract"><FlaskConical size={15} strokeWidth={1.8} /> Non-publishable</span>
        </header>

        <div class="manual-safety-note lab-safety-note">
          <ShieldCheck size={17} strokeWidth={1.8} />
          <p>
            Results stay in Advanced Diagnostics. They never unlock Apply, qualify a profile or
            write blacklist. A rejected point returns to stock automatically.
          </p>
        </div>

        {#if gpuRebootRequired}
          <p class="inline-message error lab-error" role="alert">
            A GPU driver reset occurred during this Windows boot. Restart Windows before applying a
            point, running Detector Lab or starting Forge again.
          </p>
        {/if}

        <div class="lab-controls">
          <fieldset class="recipe-picker" disabled={detectorLab?.running}>
            <legend>Detector recipe</legend>
            <button
              type="button"
              class:selected={labRecipe === "control_v25"}
              aria-pressed={labRecipe === "control_v25"}
              onclick={() => (labRecipe = "control_v25")}
            >
              <span>v25 control</span>
              <small>Exact-clock coverage under the verified voltage lock.</small>
            </button>
            <button
              type="button"
              class:selected={labRecipe === "dense_v14"}
              aria-pressed={labRecipe === "dense_v14"}
              onclick={() => (labRecipe = "dense_v14")}
            >
              <span>v14 dense</span>
              <small>Short perturbations followed by dense TextureRop canaries.</small>
            </button>
          </fieldset>

          <div class="duration-control">
            <div class="duration-label">
              <div>
                <span class="panel-kicker">GPU test duration</span>
                <strong>{formatElapsed(labDurationS)}</strong>
              </div>
              <small>Stock calibration runs first; v25 also mirrors the full recipe at stock.</small>
            </div>
            <input
              type="range"
              min="15"
              max="600"
              step="15"
              bind:value={labDurationS}
              disabled={detectorLab?.running}
              aria-label="Detector Lab duration in seconds"
            />
          </div>
        </div>

        {#if detectorLabActionError}
          <p class="inline-message error lab-error" role="alert">{detectorLabActionError}</p>
        {/if}

        <div class="lab-runner" class:running={detectorLab?.running}>
          <div class="lab-runner-head">
            <div class="lab-stage">
              {#if detectorLab?.result && detectorLab.result !== "stable"}
                <AlertTriangle size={18} strokeWidth={1.8} />
              {:else if detectorLab?.result === "stable"}
                <CheckCircle2 size={18} strokeWidth={1.8} />
              {:else}
                <Activity size={18} strokeWidth={1.8} />
              {/if}
              <div>
                <span>{formatLabPhase(detectorLab?.stage ?? "ready")}</span>
                <small>
                  {detectorLab?.running
                    ? formatLabPhase(detectorLab.current_phase ?? detectorLab.stage)
                    : detectorResultLabel(detectorLab?.result)}
                </small>
              </div>
            </div>
            <span class="lab-clock">
              {formatElapsed(Math.floor((detectorLab?.elapsed_ms ?? 0) / 1000))}
              {#if detectorLab?.stage === "running"}
                / {formatElapsed(Math.floor((detectorLab?.duration_ms ?? 0) / 1000))}
              {/if}
            </span>
          </div>

          <div
            class="lab-progress"
            class:animated={detectorLab?.stage === "running"}
            role="progressbar"
            aria-valuemin="0"
            aria-valuemax="100"
            aria-valuenow={Math.round(labProgress)}
          >
            <i style={`width: ${labProgress}%`}></i>
          </div>

          <div class="lab-summary">
            <span>{detectorLab?.note ?? "Apply a manual point to enable the laboratory."}</span>
            {#if detectorLab?.current_segment != null}
              <small>Segment {detectorLab.current_segment} · {detectorLab.frames ?? 0} frames</small>
            {/if}
          </div>

          <div class="lab-actions">
            {#if detectorLab?.running}
              <button
                class="action-button reset-point"
                type="button"
                onclick={onStopDetectorLab}
                disabled={detectorLabBusy}
              >
                <Square size={15} strokeWidth={1.9} />
                <span>{detectorLabBusy ? "Stopping…" : "Stop and return to stock"}</span>
              </button>
            {:else}
              <button
                class="action-button primary"
                type="button"
                onclick={startDetectorLab}
                disabled={!detectorLabCanStart}
              >
                <Play size={16} strokeWidth={1.9} />
                <span>Run detector</span>
              </button>
            {/if}
            {#if detectorLab?.out_path}
              <button class="action-button" type="button" onclick={onOpenDetectorLabLog}>
                <ExternalLink size={16} strokeWidth={1.8} />
                <span>Open journal</span>
              </button>
            {/if}
          </div>
        </div>

        {#if detectorLab?.phase_results?.length}
          <details class="lab-results">
            <summary>Segment evidence · {detectorLab.phase_results.length}</summary>
            <ol>
              {#each detectorLab.phase_results as phase, index}
                <li class:failed={phase.result !== "stable"}>
                  <span>{String(index + 1).padStart(2, "0")}</span>
                  <div>
                    <strong>{formatLabPhase(phase.phase)}</strong>
                    <small>{formatElapsed(Math.round(phase.duration_ms / 1000))} · {phase.frames} frames · {phase.checksum_count} checks</small>
                  </div>
                  <em>{formatLabPhase(phase.result)}</em>
                </li>
              {/each}
            </ol>
          </details>
        {/if}
      </section>
    </div>
  {/if}
</section>

<style>
  .advanced-hub {
    --diag-text: #ecebe6;
    --diag-muted: #9da4a8;
    --diag-dim: #737c82;
    --diag-danger: #d77868;
    width: min(1220px, calc(100% - 3rem));
    margin: 1.25rem auto 3rem;
    color: var(--diag-text);
    background: color-mix(in srgb, var(--forge-void) 90%, transparent);
    border-top: 1px solid var(--forge-line-strong);
    border-bottom: 1px solid var(--forge-line);
    box-shadow: 0 18px 54px rgba(0, 0, 0, 0.28);
    -webkit-font-smoothing: antialiased;
    text-rendering: optimizeLegibility;
  }

  .advanced-hub.instrument {
    font-family: "Bahnschrift", "Arial Narrow", sans-serif;
    border-inline: 1px solid var(--forge-line);
  }

  .advanced-hub.workshop {
    width: min(1320px, calc(100% - 4.5rem));
    box-shadow: none;
  }

  .advanced-hub.embedded,
  .advanced-hub.workshop.embedded {
    width: 100%;
    min-height: 0;
    margin: 0;
    border: 0;
    background: transparent;
    box-shadow: none;
  }

  .advanced-hub.instrument.embedded {
    border-inline: 0;
  }

  .advanced-hub.command.embedded .hub-header {
    border-left: 2px solid var(--forge-gold);
  }

  .advanced-hub.instrument.embedded .hub-header {
    border: 1px solid var(--forge-line-strong);
    background: color-mix(in srgb, var(--forge-panel) 48%, transparent);
  }

  .advanced-hub.workshop.embedded .hub-header {
    padding-inline: 0;
  }

  .hub-header,
  .panel-toolbar,
  .trace-control,
  .trace-export {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1.5rem;
  }

  .hub-header {
    padding: 1.5rem 1.65rem 1.25rem;
  }

  .eyebrow,
  .panel-kicker,
  dt {
    color: var(--diag-dim);
    font-size: 0.7rem;
    font-weight: 700;
    letter-spacing: 0.12em;
    text-transform: uppercase;
  }

  h2,
  h3,
  p {
    margin: 0;
  }

  h2 {
    margin-top: 0.25rem;
    font-size: clamp(1.55rem, 2.4vw, 2.15rem);
    font-weight: 580;
    letter-spacing: -0.025em;
    text-wrap: balance;
  }

  h3 {
    margin-top: 0.22rem;
    font-size: 1.12rem;
    font-weight: 600;
    text-wrap: balance;
  }

  .hub-header p,
  .panel-toolbar p,
  .trace-control p,
  .trace-export p {
    margin-top: 0.35rem;
    color: var(--diag-muted);
    font-size: 0.86rem;
    line-height: 1.55;
  }

  button {
    min-height: 42px;
    font: inherit;
    cursor: pointer;
    transition:
      color 150ms ease,
      background-color 150ms ease,
      border-color 150ms ease,
      transform 120ms ease;
  }

  button:active:not(:disabled) {
    transform: scale(0.96);
  }

  button:focus-visible {
    outline: 2px solid var(--forge-blue);
    outline-offset: 3px;
  }

  button:disabled {
    cursor: not-allowed;
    opacity: 0.48;
  }

  .back-button,
  .action-button {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 0.55rem;
    padding: 0.65rem 0.9rem;
    color: var(--diag-text);
    background: transparent;
    border: 1px solid var(--forge-line);
    border-radius: 4px;
  }

  .back-button:hover,
  .action-button:hover:not(:disabled) {
    color: #fff;
    background: var(--forge-panel-raised);
    border-color: var(--forge-line-strong);
  }

  .action-button.primary {
    color: #0c0d0d;
    background: var(--forge-gold);
    border-color: var(--forge-gold);
    font-weight: 650;
  }

  .diagnostic-tabs {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    border-top: 1px solid var(--forge-line);
    border-bottom: 1px solid var(--forge-line);
  }

  .diagnostic-tabs button {
    position: relative;
    display: grid;
    grid-template-columns: auto 1fr auto;
    align-items: center;
    gap: 0.65rem;
    min-height: 58px;
    padding: 0.75rem 1.05rem;
    color: var(--diag-muted);
    text-align: left;
    background: transparent;
    border: 0;
    border-right: 1px solid var(--forge-line);
  }

  .diagnostic-tabs button:last-child {
    border-right: 0;
  }

  .diagnostic-tabs button::after {
    content: "";
    position: absolute;
    right: 0;
    bottom: -1px;
    left: 0;
    height: 2px;
    background: transparent;
    transition: background-color 150ms ease;
  }

  .diagnostic-tabs button:hover,
  .diagnostic-tabs button.active {
    color: var(--diag-text);
    background: color-mix(in srgb, var(--forge-panel) 72%, transparent);
  }

  .diagnostic-tabs button.active::after {
    background: var(--forge-gold);
  }

  .diagnostic-tabs small {
    color: var(--diag-dim);
    font-size: 0.7rem;
    font-variant-numeric: tabular-nums;
  }

  .diagnostic-panel {
    min-height: 430px;
    padding: 1.45rem 1.65rem 1.6rem;
  }

  .panel-toolbar,
  .trace-control {
    margin-bottom: 1.15rem;
  }

  .terminal-shell {
    min-height: 260px;
  }

  .terminal-shell :global(.terminal) {
    border-radius: 4px;
    box-shadow: none;
  }

  .terminal-shell :global(.term-body) {
    max-height: min(48vh, 500px);
    min-height: 280px;
  }

  .inline-message {
    margin: -0.35rem 0 1rem;
    padding-left: 0.75rem;
    color: var(--forge-green);
    font-size: 0.78rem;
    line-height: 1.45;
    overflow-wrap: anywhere;
    border-left: 2px solid currentColor;
  }

  .inline-message.error {
    color: var(--diag-danger);
  }

  .panel-footnote {
    margin-top: 0.9rem;
    color: var(--diag-dim);
    font-size: 0.74rem;
    line-height: 1.5;
  }

  .state-readout,
  .trace-note {
    display: inline-flex;
    align-items: center;
    gap: 0.55rem;
    color: var(--diag-muted);
    font-size: 0.8rem;
  }

  .state-readout i,
  .trace-note i {
    width: 8px;
    height: 8px;
    flex: 0 0 auto;
    background: var(--diag-dim);
    border-radius: 50%;
  }

  .state-readout.bump,
  .trace-note.recording {
    color: var(--forge-gold);
  }

  .state-readout.bump i,
  .trace-note.recording i {
    background: currentColor;
    box-shadow: 0 0 0 4px color-mix(in srgb, currentColor 14%, transparent);
  }

  .state-readout.stock {
    color: var(--diag-danger);
  }

  .state-readout.stock i {
    background: currentColor;
  }

  .sentinel-readout,
  .trace-metrics {
    margin: 0;
    border-top: 1px solid var(--forge-line);
  }

  .sentinel-readout > div {
    display: grid;
    grid-template-columns: minmax(130px, 0.25fr) 1fr;
    gap: 1rem;
    padding: 1.05rem 0;
    border-bottom: 1px solid var(--forge-line);
  }

  dd {
    margin: 0;
    color: var(--diag-text);
    font-size: 0.9rem;
    line-height: 1.5;
  }

  .blacklist-log,
  .condemnation-log {
    margin-top: 1.25rem;
    border-top: 1px solid var(--forge-line);
    border-bottom: 1px solid var(--forge-line);
    padding: 1.05rem 0;
  }

  .blacklist-heading {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
  }

  .blacklist-heading h4 {
    margin: 0.2rem 0 0;
    color: var(--diag-text);
    font-size: 1rem;
    font-weight: 600;
  }

  .blacklist-heading > span {
    display: grid;
    min-width: 40px;
    min-height: 40px;
    place-items: center;
    color: var(--diag-dim);
    background: color-mix(in srgb, var(--forge-panel) 72%, transparent);
    font-variant-numeric: tabular-nums;
    box-shadow: 0 0 0 1px var(--forge-line);
  }

  .blacklist-heading > span.active {
    color: var(--diag-danger);
    box-shadow: 0 0 0 1px color-mix(in srgb, var(--diag-danger) 45%, transparent);
  }

  .blacklist-log ol,
  .condemnation-log ol {
    display: grid;
    max-height: 280px;
    gap: 1px;
    margin: 1rem 0 0;
    padding: 0;
    overflow-y: auto;
    list-style: none;
    background: var(--forge-line);
  }

  .blacklist-log li,
  .condemnation-log li {
    display: grid;
    grid-template-columns: 44px minmax(0, 1fr);
    align-items: center;
    gap: 0.85rem;
    min-height: 58px;
    padding: 0.65rem 0.85rem;
    background: color-mix(in srgb, var(--forge-void) 96%, transparent);
  }

  .blacklist-index {
    color: var(--diag-danger);
    font: 0.72rem/1 "Cascadia Code", "Consolas", ui-monospace, monospace;
    font-variant-numeric: tabular-nums;
  }

  .blacklist-log li > div {
    display: flex;
    min-width: 0;
    flex-direction: column;
    gap: 0.25rem;
  }

  .condemnation-log {
    margin-top: 1.25rem;
  }

  .condemnation-log li > div {
    display: flex;
    min-width: 0;
    flex-direction: column;
    gap: 0.25rem;
  }

  .condemnation-log li.rehabilitated {
    opacity: 0.58;
  }

  .ledger-title {
    display: flex;
    min-width: 0;
    align-items: center;
    gap: 0.5rem;
    flex-wrap: wrap;
  }

  .severity,
  .rehabilitated-label {
    padding: 0.18rem 0.38rem;
    color: #e4b184;
    background: rgba(197, 138, 81, 0.12);
    font-size: 0.58rem;
    font-weight: 800;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    box-shadow: inset 0 0 0 1px rgba(197, 138, 81, 0.3);
  }

  .severity.rigid,
  .severity.critical,
  .severity.hard {
    color: #ef9a8c;
    background: rgba(215, 120, 104, 0.12);
    box-shadow: inset 0 0 0 1px rgba(215, 120, 104, 0.34);
  }

  .rehabilitated-label {
    color: #8fc8a3;
    background: rgba(83, 183, 122, 0.1);
    box-shadow: inset 0 0 0 1px rgba(83, 183, 122, 0.28);
  }

  .ledger-meta {
    font-variant-numeric: tabular-nums;
    opacity: 0.82;
    overflow-wrap: anywhere;
  }

  .blacklist-log strong,
  .condemnation-log strong {
    color: #e2d8d4;
    font-size: 0.82rem;
    font-weight: 550;
    overflow-wrap: anywhere;
  }

  .blacklist-log small,
  .condemnation-log small,
  .blacklist-empty {
    color: var(--diag-dim);
    font-size: 0.74rem;
  }

  .blacklist-empty {
    margin-top: 0.9rem;
  }

  .trace-switch {
    display: inline-flex;
    align-items: center;
    gap: 0.65rem;
    min-width: 190px;
    min-height: 44px;
    justify-content: center;
    padding: 0.55rem 0.8rem;
    color: var(--diag-muted);
    background: transparent;
    border: 1px solid var(--forge-line);
    border-radius: 4px;
  }

  .trace-switch.on {
    color: var(--forge-green);
    border-color: color-mix(in srgb, var(--forge-green) 55%, transparent);
  }

  .switch-track {
    position: relative;
    width: 32px;
    height: 18px;
    flex: 0 0 auto;
    background: var(--forge-graphite);
    border: 1px solid var(--forge-line);
    border-radius: 999px;
  }

  .switch-track i {
    position: absolute;
    top: 3px;
    left: 3px;
    width: 10px;
    height: 10px;
    background: var(--diag-muted);
    border-radius: 50%;
    transition:
      background-color 150ms ease,
      transform 150ms ease;
  }

  .trace-switch.on .switch-track i {
    background: var(--forge-green);
    transform: translateX(14px);
  }

  .trace-note {
    min-height: 24px;
    margin-bottom: 0.9rem;
  }

  .trace-action-error {
    margin-top: -0.35rem;
  }

  .trace-live-status {
    margin: -0.25rem 0 1rem;
    color: var(--forge-green);
    font-size: 0.8rem;
    font-variant-numeric: tabular-nums;
    line-height: 1.55;
    overflow-wrap: anywhere;
  }

  .trace-metrics {
    display: grid;
    grid-template-columns: repeat(5, minmax(0, 1fr));
    border-bottom: 1px solid var(--forge-line);
  }

  .trace-metrics > div {
    padding: 1rem 0.9rem;
    border-right: 1px solid var(--forge-line);
  }

  .trace-metrics > div:first-child {
    padding-left: 0;
  }

  .trace-metrics > div:last-child {
    padding-right: 0;
    border-right: 0;
  }

  .trace-metrics dd {
    margin-top: 0.35rem;
    font-size: 1.15rem;
    font-variant-numeric: tabular-nums;
  }

  .trace-export {
    margin-top: 1.15rem;
    padding-top: 1.15rem;
  }

  .trace-export > div {
    min-width: 0;
  }

  .trace-export code {
    display: block;
    margin-top: 0.42rem;
    color: var(--forge-blue);
    font: 0.76rem/1.5 "Cascadia Code", "Consolas", ui-monospace, monospace;
    overflow-wrap: anywhere;
    word-break: break-all;
  }

  .manual-toolbar {
    margin-bottom: 0.85rem;
  }

  .manual-state {
    display: inline-flex;
    min-height: 32px;
    align-items: center;
    gap: 0.5rem;
    padding: 0.35rem 0.65rem;
    color: var(--diag-muted);
    font-size: 0.7rem;
    font-weight: 750;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    border: 1px solid var(--forge-line);
  }

  .manual-state i {
    width: 7px;
    height: 7px;
    background: currentColor;
    border-radius: 50%;
  }

  .manual-state.active {
    color: var(--forge-green);
    border-color: color-mix(in srgb, var(--forge-green) 42%, var(--forge-line));
  }

  .manual-safety-note {
    display: flex;
    align-items: flex-start;
    gap: 0.7rem;
    margin-bottom: 1.15rem;
    padding: 0.8rem 0.9rem;
    color: var(--forge-blue);
    background: color-mix(in srgb, var(--forge-blue) 6%, transparent);
    border-left: 2px solid currentColor;
  }

  .manual-safety-note :global(svg) {
    flex: 0 0 auto;
    margin-top: 0.08rem;
  }

  .manual-safety-note p {
    color: var(--diag-muted);
    font-size: 0.78rem;
    line-height: 1.5;
    text-wrap: pretty;
  }

  .manual-safety-note strong {
    color: var(--diag-text);
  }

  .manual-layout {
    display: grid;
    grid-template-columns: minmax(0, 1.2fr) minmax(280px, 0.8fr);
    border-top: 1px solid var(--forge-line);
    border-bottom: 1px solid var(--forge-line);
  }

  .manual-setup {
    min-width: 0;
    padding: 1.2rem 1.3rem 1.25rem 0;
  }

  .manual-live {
    min-width: 0;
    padding: 1.2rem 0 1.25rem 1.3rem;
    border-left: 1px solid var(--forge-line);
  }

  .manual-setup h4,
  .manual-live h4 {
    margin: 0.22rem 0 0;
    color: var(--diag-text);
    font-size: 0.98rem;
    font-weight: 620;
    text-wrap: balance;
  }

  .manual-copy,
  .manual-live > p {
    margin-top: 0.55rem;
    color: var(--diag-muted);
    font-size: 0.76rem;
    line-height: 1.55;
    text-wrap: pretty;
  }

  .manual-copy {
    max-width: 64ch;
  }

  .manual-live > p {
    min-height: 3.4em;
  }

  .manual-point-fields {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 0.7rem;
    margin-top: 1rem;
  }

  .manual-point-fields > label {
    display: grid;
    gap: 0.42rem;
    color: var(--diag-dim);
    font-size: 0.69rem;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }

  .numeric-input {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    align-items: center;
    min-height: 44px;
    background: color-mix(in srgb, var(--forge-void) 88%, transparent);
    border: 1px solid var(--forge-line);
  }

  .numeric-input:focus-within {
    border-color: var(--forge-blue);
    box-shadow: 0 0 0 1px color-mix(in srgb, var(--forge-blue) 40%, transparent);
  }

  .numeric-input input {
    min-width: 0;
    padding: 0.65rem 0.75rem;
    color: var(--diag-text);
    background: transparent;
    border: 0;
    outline: 0;
    font: 600 0.95rem/1 "Cascadia Code", "Consolas", ui-monospace, monospace;
    font-variant-numeric: tabular-nums;
  }

  .numeric-input input::placeholder {
    color: color-mix(in srgb, var(--diag-dim) 70%, transparent);
  }

  .numeric-input small {
    padding-right: 0.75rem;
    color: var(--diag-dim);
    font-size: 0.68rem;
  }

  .manual-actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.65rem;
    margin-top: 1rem;
  }

  .manual-actions .action-button {
    min-height: 42px;
    transition:
      color 150ms ease,
      background-color 150ms ease,
      border-color 150ms ease,
      transform 100ms ease;
  }

  .manual-actions .action-button:active:not(:disabled) {
    transform: scale(0.96);
  }

  .action-button.reset-point {
    color: #f2aaa0;
    border-color: color-mix(in srgb, var(--diag-danger) 55%, var(--forge-line));
  }

  .manual-footnote {
    margin-top: 0.7rem;
    color: var(--diag-dim);
    font-size: 0.68rem;
    line-height: 1.45;
    text-wrap: pretty;
  }

  .manual-live dl {
    margin: 0.85rem 0 0;
  }

  .manual-live dl > div {
    display: grid;
    grid-template-columns: 110px minmax(0, 1fr);
    gap: 0.65rem;
    padding: 0.62rem 0;
    border-bottom: 1px solid var(--forge-line);
  }

  .manual-live dt {
    color: var(--diag-dim);
    font-size: 0.68rem;
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }

  .manual-live dd {
    color: var(--diag-muted);
    text-align: right;
    overflow-wrap: anywhere;
    font-size: 0.76rem;
    font-variant-numeric: tabular-nums;
  }

  .manual-live dd.verified {
    color: var(--forge-green);
  }

  .detector-lab {
    margin-top: 1.4rem;
    padding-top: 1.3rem;
    border-top: 1px solid var(--forge-line-strong);
  }

  .lab-heading,
  .lab-runner-head,
  .lab-summary,
  .duration-label,
  .lab-actions {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
  }

  .lab-heading > div,
  .duration-label > div {
    min-width: 0;
  }

  .lab-heading h4 {
    margin: 0.22rem 0 0;
    color: var(--diag-text);
    font-size: 1.02rem;
    font-weight: 620;
    text-wrap: balance;
  }

  .lab-heading p {
    margin-top: 0.45rem;
    color: var(--diag-muted);
    font-size: 0.76rem;
    line-height: 1.5;
    text-wrap: pretty;
  }

  .lab-contract {
    display: inline-flex;
    min-height: 36px;
    flex: 0 0 auto;
    align-items: center;
    gap: 0.45rem;
    padding: 0.4rem 0.65rem;
    color: var(--forge-gold);
    font-size: 0.65rem;
    font-weight: 760;
    letter-spacing: 0.07em;
    text-transform: uppercase;
    border: 1px solid color-mix(in srgb, var(--forge-gold) 40%, var(--forge-line));
  }

  .lab-safety-note {
    margin-top: 1rem;
    margin-bottom: 0;
  }

  .lab-controls {
    display: grid;
    grid-template-columns: minmax(0, 1.15fr) minmax(270px, 0.85fr);
    gap: 1rem;
    margin-top: 1rem;
  }

  .recipe-picker {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 0.65rem;
    min-width: 0;
    margin: 0;
    padding: 0;
    border: 0;
  }

  .recipe-picker legend {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    white-space: nowrap;
    border: 0;
  }

  .recipe-picker button {
    display: flex;
    min-width: 0;
    min-height: 68px;
    flex-direction: column;
    align-items: flex-start;
    justify-content: center;
    gap: 0.35rem;
    padding: 0.7rem 0.8rem;
    color: var(--diag-muted);
    text-align: left;
    background: color-mix(in srgb, var(--forge-void) 88%, transparent);
    border: 1px solid var(--forge-line);
  }

  .recipe-picker button:hover:not(:disabled),
  .recipe-picker button.selected {
    color: var(--diag-text);
    background: color-mix(in srgb, var(--forge-panel) 74%, transparent);
    border-color: color-mix(in srgb, var(--forge-gold) 48%, var(--forge-line));
  }

  .recipe-picker button span {
    font-size: 0.83rem;
    font-weight: 650;
  }

  .recipe-picker button small {
    color: var(--diag-dim);
    font-size: 0.67rem;
    line-height: 1.35;
    text-wrap: pretty;
  }

  .duration-control {
    min-width: 0;
    padding: 0.7rem 0.85rem;
    background: color-mix(in srgb, var(--forge-void) 88%, transparent);
    border: 1px solid var(--forge-line);
  }

  .duration-label {
    align-items: flex-end;
  }

  .duration-label strong {
    display: block;
    margin-top: 0.2rem;
    color: var(--diag-text);
    font: 650 1rem/1 "Cascadia Code", "Consolas", ui-monospace, monospace;
    font-variant-numeric: tabular-nums;
  }

  .duration-label > small {
    max-width: 24ch;
    color: var(--diag-dim);
    font-size: 0.64rem;
    line-height: 1.35;
    text-align: right;
    text-wrap: pretty;
  }

  .duration-control input[type="range"] {
    width: 100%;
    min-height: 28px;
    margin: 0.45rem 0 0.25rem;
    accent-color: var(--forge-gold);
    cursor: pointer;
  }

  .duration-control input[type="range"]:disabled {
    cursor: not-allowed;
  }

  .lab-error {
    margin: 0.85rem 0 0;
  }

  .lab-runner {
    margin-top: 1rem;
    padding: 0.9rem;
    background: color-mix(in srgb, var(--forge-void) 90%, transparent);
    border: 1px solid var(--forge-line);
  }

  .lab-runner.running {
    border-color: color-mix(in srgb, var(--forge-gold) 38%, var(--forge-line));
  }

  .lab-stage {
    display: flex;
    min-width: 0;
    align-items: center;
    gap: 0.65rem;
    color: var(--forge-gold);
  }

  .lab-stage > div {
    display: flex;
    min-width: 0;
    flex-direction: column;
    gap: 0.2rem;
  }

  .lab-stage span {
    color: var(--diag-text);
    font-size: 0.82rem;
    font-weight: 650;
  }

  .lab-stage small,
  .lab-summary small {
    color: var(--diag-dim);
    font-size: 0.67rem;
    font-variant-numeric: tabular-nums;
  }

  .lab-clock {
    flex: 0 0 auto;
    color: var(--diag-muted);
    font: 600 0.78rem/1 "Cascadia Code", "Consolas", ui-monospace, monospace;
    font-variant-numeric: tabular-nums;
  }

  .lab-progress {
    position: relative;
    height: 8px;
    margin-top: 0.8rem;
    overflow: hidden;
    background: var(--forge-graphite);
    border: 1px solid var(--forge-line);
  }

  .lab-progress i {
    position: absolute;
    inset: 0 auto 0 0;
    display: block;
    min-width: 0;
    background: linear-gradient(90deg, #b36a36 0%, var(--forge-gold) 52%, var(--forge-green) 100%);
    transition: width 300ms ease;
  }

  .lab-progress.animated i::after {
    content: "";
    position: absolute;
    inset: 0;
    background: linear-gradient(110deg, transparent 25%, rgba(255, 255, 255, 0.24) 45%, transparent 65%);
    transform: translateX(-100%);
    animation: detector-sheen 1.45s linear infinite;
  }

  .lab-summary {
    align-items: flex-start;
    margin-top: 0.65rem;
  }

  .lab-summary > span {
    color: var(--diag-muted);
    font-size: 0.7rem;
    line-height: 1.45;
    text-wrap: pretty;
  }

  .lab-summary small {
    flex: 0 0 auto;
  }

  .lab-actions {
    justify-content: flex-start;
    flex-wrap: wrap;
    margin-top: 0.85rem;
  }

  .lab-results {
    margin-top: 0.85rem;
    border-top: 1px solid var(--forge-line);
    border-bottom: 1px solid var(--forge-line);
  }

  .lab-results summary {
    min-height: 44px;
    padding: 0.8rem 0;
    color: var(--diag-muted);
    font-size: 0.72rem;
    font-weight: 650;
    font-variant-numeric: tabular-nums;
    cursor: pointer;
  }

  .lab-results ol {
    display: grid;
    gap: 1px;
    margin: 0 0 0.85rem;
    padding: 0;
    list-style: none;
    background: var(--forge-line);
  }

  .lab-results li {
    display: grid;
    grid-template-columns: 34px minmax(0, 1fr) auto;
    align-items: center;
    gap: 0.65rem;
    min-height: 52px;
    padding: 0.55rem 0.7rem;
    background: color-mix(in srgb, var(--forge-void) 96%, transparent);
  }

  .lab-results li > span,
  .lab-results li em,
  .lab-results li small {
    font-variant-numeric: tabular-nums;
  }

  .lab-results li > span {
    color: var(--diag-dim);
    font: 0.68rem/1 "Cascadia Code", "Consolas", ui-monospace, monospace;
  }

  .lab-results li > div {
    display: flex;
    min-width: 0;
    flex-direction: column;
    gap: 0.22rem;
  }

  .lab-results strong {
    color: var(--diag-text);
    font-size: 0.76rem;
    font-weight: 580;
  }

  .lab-results small {
    color: var(--diag-dim);
    font-size: 0.65rem;
  }

  .lab-results em {
    color: var(--forge-green);
    font-size: 0.64rem;
    font-style: normal;
    font-weight: 720;
    text-transform: uppercase;
  }

  .lab-results li.failed em {
    color: var(--diag-danger);
  }

  @keyframes detector-sheen {
    to {
      transform: translateX(100%);
    }
  }

  @media (max-width: 980px) {
    .diagnostic-tabs {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }

    .diagnostic-tabs small {
      display: none;
    }

    .manual-layout {
      grid-template-columns: 1fr;
    }

    .manual-setup {
      padding-right: 0;
    }

    .manual-live {
      padding-left: 0;
      border-top: 1px solid var(--forge-line);
      border-left: 0;
    }

    .lab-controls {
      grid-template-columns: 1fr;
    }
  }

  @media (max-width: 780px) {
    .advanced-hub,
    .advanced-hub.workshop {
      width: calc(100% - 1.5rem);
    }

    .advanced-hub.embedded,
    .advanced-hub.workshop.embedded {
      width: 100%;
    }

    .hub-header,
    .panel-toolbar,
    .trace-control,
    .trace-export {
      align-items: flex-start;
      flex-direction: column;
    }

    .lab-heading,
    .duration-label,
    .lab-summary {
      align-items: flex-start;
      flex-direction: column;
    }

    .duration-label > small {
      max-width: none;
      text-align: left;
    }

    .diagnostic-tabs button {
      grid-template-columns: auto 1fr;
    }

    .trace-metrics {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }

    .trace-metrics > div {
      border-bottom: 1px solid var(--forge-line);
    }

    .sentinel-readout > div {
      grid-template-columns: 1fr;
      gap: 0.35rem;
    }
  }

  @media (max-width: 560px) {
    .manual-actions {
      align-items: flex-start;
      flex-direction: column;
    }

    .manual-point-fields {
      grid-template-columns: 1fr;
    }

    .recipe-picker {
      grid-template-columns: 1fr;
    }

    .lab-runner-head {
      align-items: flex-start;
    }

    .lab-results li {
      grid-template-columns: 28px minmax(0, 1fr);
    }

    .lab-results li em {
      grid-column: 2;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    button,
    .diagnostic-tabs button::after,
    .switch-track i {
      transition: none;
    }

    .lab-progress i {
      transition: none;
    }

    .lab-progress.animated i::after {
      animation: none;
    }
  }
</style>
