<script>
  import {
    ArrowLeft,
    ExternalLink,
    FileDown,
    Radio,
    ShieldCheck,
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
    sentinelState = "No events",
    sentinelSummary = "No automatic recovery action recorded.",
    gameTrace = null,
    gameTraceBusy = false,
    gameTraceActionError = "",
    gameTraceExportBusy = false,
    gameTraceExportMsg = "",
    onExportLog,
    onToggleGameTrace,
    onOpenGameTraceLog,
    onClose,
  } = $props();

  let activeTab = $state("log");

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
      detail: sentinelState,
      icon: ShieldCheck,
    },
    {
      id: "game-trace",
      label: "Game Trace",
      detail: gameTrace?.running ? `Gravando · ${gameTrace.samples ?? 0}` : "Parado",
      icon: Radio,
    },
  ]);

  const terminalStatus = $derived(
    powerSweep?.running ? powerSweep.phase : logLines.length ? "ready" : "idle",
  );
  const traceReady = $derived(Boolean(gameTrace));

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

      <p class="panel-footnote">
        Sentinel is automatic and has no manual switch here. This panel reports events; it does not change GPU tuning.
      </p>
    </div>
  {:else}
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
    grid-template-columns: repeat(3, minmax(0, 1fr));
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

    .diagnostic-tabs button {
      grid-template-columns: auto 1fr;
    }

    .diagnostic-tabs small {
      display: none;
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

  @media (prefers-reduced-motion: reduce) {
    button,
    .diagnostic-tabs button::after,
    .switch-track i {
      transition: none;
    }
  }
</style>
