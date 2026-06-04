<script>
  import { t } from "../../i18n.js";
  import LogTerminal from "./LogTerminal.svelte";

  let { forge = null, forgeRunning = false, showLog = false, onRequestStart, onStop } = $props();

  const phase = $derived(forge?.phase && forge.phase !== "idle" ? forge.phase : "Not running");
  const hasRun = $derived(Boolean(forge && forge.phase !== "idle"));
</script>

<div class="forge-all">
  <div class="real-head">
    <h3 class="section-head">{$t("forge.forgeAll")}</h3>
    {#if forgeRunning}
      <button class="btn stop" onclick={onStop}>{$t("forge.stopForge")}</button>
    {:else}
      <button class="btn go" onclick={onRequestStart}>{$t("forge.runForge")}</button>
    {/if}
  </div>
  <p class="sub">{$t("forge.forgeAllDesc")}</p>
  <div class="progress-summary">
    <span>Current phase</span>
    <strong>{phase}</strong>
    {#if forge?.note}
      <p>{forge.note}</p>
    {:else if !hasRun}
      <p>No forge run is active yet. Start Forge GPU when you are ready to let Nidavellir learn this card.</p>
    {/if}
  </div>
  {#if showLog && forge && forge.phase !== "idle" && (forge.log?.length || forge.running)}
    <LogTerminal
      title="nidavellir / forge"
      status={forge.running ? forge.phase : "done"}
      live={forge.running}
      lines={forge.log ?? []}
      runningText={forge.running ? `${forge.phase}...` : null}
    />
  {/if}
</div>

<p class="sub apply-hint">{$t("forge.orderHint")}</p>

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
  .apply-hint {
    margin: 0.1rem 0 0.4rem;
    font-size: 0.75rem;
    color: var(--nord-dim);
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
</style>
