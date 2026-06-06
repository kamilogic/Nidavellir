<script>
  import LogTerminal from "./LogTerminal.svelte";

  let { powerSweep = null, powerRunning = false, showLog = false } = $props();

  const phase = $derived(powerSweep?.phase && powerSweep.phase !== "idle" ? powerSweep.phase : "Not running");
  const hasRun = $derived(Boolean(powerSweep && powerSweep.phase !== "idle"));
  const profileCount = $derived(
    [powerSweep?.godforge, powerSweep?.brokkrs, powerSweep?.deep_calm].filter(Boolean).length,
  );
</script>

<div class="forge-all">
  <div class="real-head">
    <h3 class="section-head">Forge Progress</h3>
  </div>
  <p class="sub">Current implementation: core VF forge and profile generation. VRAM optimization is planned for a later pipeline step.</p>
  <div class="progress-summary">
    <span>Current phase</span>
    <strong>{phase}</strong>
    {#if powerSweep?.note}
      <p>{powerSweep.note}</p>
    {:else if profileCount}
      <p>{profileCount} profile{profileCount === 1 ? "" : "s"} generated from the canonical core VF forge path.</p>
    {:else if !hasRun}
      <p>No core VF forge run is active yet. Start Forge GPU when you are ready to let Nidavellir learn this card.</p>
    {/if}
  </div>
  <div class="pipeline-steps" aria-label="Forge pipeline status">
    <span class:active={powerRunning} class:done={hasRun}>Core VF forge</span>
    <span class:done={profileCount > 0}>Profile generation</span>
    <span class="planned">VRAM optimization planned</span>
    <span class="planned">Final validation planned</span>
  </div>
  {#if showLog && powerSweep && powerSweep.phase !== "idle" && (powerSweep.log?.length || powerSweep.running)}
    <LogTerminal
      title="nidavellir / core vf forge"
      status={powerSweep.running ? powerSweep.phase : "done"}
      live={powerSweep.running}
      lines={powerSweep.log ?? []}
      runningText={powerSweep.running ? `${powerSweep.phase}...` : null}
    />
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
    gap: 0.5rem;
    box-shadow: var(--forge-panel-edge);
  }
  .real-head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 1rem;
  }
  .section-head {
    margin: 0 0 0.5rem;
    font-size: 0.72rem;
    font-weight: 700;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--muted);
  }
  .sub {
    margin: 0.25rem 0;
    font-size: 0.82rem;
    color: var(--muted);
  }
  .progress-summary {
    background: rgba(5, 7, 11, 0.28);
    border: 1px solid rgba(255, 255, 255, 0.055);
    border-radius: 8px;
    padding: 0.7rem 0.8rem;
  }
  .progress-summary span {
    display: block;
    font-size: 0.68rem;
    font-weight: 700;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--nord-dim);
    margin-bottom: 0.25rem;
  }
  .progress-summary strong {
    color: var(--text);
    font-size: 0.95rem;
  }
  .progress-summary p {
    margin: 0.35rem 0 0;
    color: var(--muted);
    font-size: 0.84rem;
    line-height: 1.45;
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
  @media (max-width: 760px) {
    .pipeline-steps {
      grid-template-columns: 1fr;
    }
  }
</style>
