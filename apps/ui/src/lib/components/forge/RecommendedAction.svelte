<script>
  import StatusBadge from "./StatusBadge.svelte";

  let {
    applied = null,
    forge = null,
    forgeRunning = false,
    realProfiles = null,
    powerSweep = null,
    safeLoop = null,
    onStartForge,
    onStopForge,
    onApplyRecommended,
  } = $props();

  const hasProfiles = $derived(Boolean(realProfiles || powerSweep?.brokkrs));
  const needsAttention = $derived(Boolean(safeLoop?.safe_mode || safeLoop?.state === "unstable"));
  const currentPhase = $derived(forge?.phase && forge.phase !== "idle" ? forge.phase : null);
  const title = $derived.by(() => {
    if (needsAttention) return "Needs Attention";
    if (forgeRunning) return "Forging in progress";
    if (!hasProfiles && !applied?.core) return "Raw GPU Detected";
    if (hasProfiles && !applied?.core) return "Profiles are ready";
    return "Profile applied";
  });
  const body = $derived.by(() => {
    if (needsAttention) return "Nidavellir detected a safety condition that should be reviewed before more tuning.";
    if (forgeRunning) return `Nidavellir is currently testing your GPU${currentPhase ? `: ${currentPhase}` : ""}.`;
    if (!hasProfiles && !applied?.core) {
      return "Nidavellir has detected your NVIDIA GPU, but has not forged it yet.";
    }
    if (hasProfiles && !applied?.core) {
      return "Nidavellir has enough profile data to recommend a daily-use profile.";
    }
    return "Your applied profile will be re-applied automatically on boot with Safe Loop protection.";
  });
</script>

<section class="next-action">
  <div>
    <span class="eyebrow">Recommended next step</span>
    <h3>{title}</h3>
    <p>{body}</p>
  </div>

  {#if forgeRunning}
    <button class="btn stop" onclick={onStopForge}>Stop forging</button>
  {:else if needsAttention}
    <StatusBadge label="Review Safety" variant="attention" />
  {:else if hasProfiles && !applied?.core}
    <button class="btn go" onclick={onApplyRecommended}>Apply Brokkr's Best</button>
  {:else if !applied?.core}
    <button class="btn go" onclick={onStartForge}>Forge GPU</button>
  {:else}
    <StatusBadge label="Ready for Daily Use" variant="protected" />
  {/if}

  {#if !hasProfiles && !applied?.core && !forgeRunning}
    <div class="first-run">
      <span>What will happen</span>
      <ol>
        <li>Check stability</li>
        <li>Learn safe operating regions</li>
        <li>Create three transparent profiles</li>
        <li>Recommend one for daily use</li>
      </ol>
      <p>Safety: Safe Loop protection will be active before risky steps.</p>
    </div>
  {/if}
</section>

<style>
  .next-action {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: 1rem;
    align-items: start;
    background: var(--forge-panel-bg);
    border: 1px solid var(--forge-line);
    border-radius: 12px;
    padding: 1rem 1.1rem;
    box-shadow: var(--forge-panel-edge);
  }
  .eyebrow,
  .first-run span {
    display: block;
    font-size: 0.68rem;
    font-weight: 700;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--nord-dim);
    margin-bottom: 0.35rem;
  }
  h3 {
    margin: 0;
    color: var(--text);
    font-size: 1.1rem;
  }
  p {
    margin: 0.4rem 0 0;
    color: var(--muted);
    font-size: 0.88rem;
    line-height: 1.5;
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
    white-space: nowrap;
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
  .first-run {
    grid-column: 1 / -1;
    border-top: 1px solid var(--border);
    padding-top: 0.85rem;
  }
  ol {
    margin: 0.3rem 0 0;
    padding-left: 1.25rem;
    color: var(--text);
    line-height: 1.6;
    font-size: 0.86rem;
  }
  @media (max-width: 640px) {
    .next-action {
      grid-template-columns: 1fr;
    }
  }
</style>
