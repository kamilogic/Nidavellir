<script>
  import { Play, Square } from "@lucide/svelte";
  import StatusBadge from "./StatusBadge.svelte";

  let {
    applied = null,
    powerSweep = null,
    powerRunning = false,
    safeLoop = null,
    onStartPower,
    onStopPower,
  } = $props();

  const hasProfiles = $derived(Boolean(powerSweep?.godforge || powerSweep?.brokkrs || powerSweep?.deep_calm));
  const needsAttention = $derived(Boolean(safeLoop?.safe_mode || safeLoop?.state === "unstable"));
  const currentPhase = $derived(powerSweep?.phase && powerSweep.phase !== "idle" ? powerSweep.phase : null);
  const title = $derived.by(() => {
    if (needsAttention) return "Needs Attention";
    if (powerRunning) return "Core VF forge in progress";
    if (!hasProfiles && !applied?.core) return "Raw GPU Detected";
    if (hasProfiles && !applied?.core) return "Profiles are ready";
    if (hasProfiles) return "Profile applied";
    return "Ready to forge";
  });
  const body = $derived.by(() => {
    if (needsAttention) return "Nidavellir detected a safety condition that should be reviewed before more tuning.";
    if (powerRunning) {
      return `Nidavellir is running the implemented core VF forge and profile generation path${currentPhase ? `: ${currentPhase}` : ""}.`;
    }
    if (!hasProfiles && !applied?.core) {
      return "Nidavellir has detected your NVIDIA GPU. The current Forge GPU action runs the implemented core VF forge and profile generation path.";
    }
    if (hasProfiles && !applied?.core) {
      return "Nidavellir has generated profiles. Choose one below, or refine the core VF profiles by running the forge again.";
    }
    return "Your applied profile will be re-applied automatically on boot with Safe Loop protection. You can refine core VF profiles at any time.";
  });
  const primaryLabel = $derived(hasProfiles ? "Refine Profiles" : "Forge GPU");
</script>

<section class="next-action">
  <div>
    <span class="eyebrow">Recommended next step</span>
    <h3>{title}</h3>
    <p>{body}</p>
  </div>

  {#if powerRunning}
    <button class="btn stop" onclick={onStopPower}>
      <Square size={14} strokeWidth={1.9} />
      <span>Stop forging</span>
    </button>
  {:else if needsAttention}
    <StatusBadge label="Review Safety" variant="attention" symbol="attention" />
  {:else}
    <button class="btn go" onclick={onStartPower}>
      <Play size={15} strokeWidth={1.9} />
      <span>{primaryLabel}</span>
    </button>
  {/if}

  {#if !hasProfiles && !applied?.core && !powerRunning}
    <div class="first-run">
      <span>Current implemented path</span>
      <ol>
        <li>Check Safe Loop readiness before risky steps</li>
        <li>Learn core VF behavior under load</li>
        <li>Create three transparent profiles</li>
        <li>Recommend one for daily use</li>
      </ol>
      <p>Planned later: VRAM optimization, VRAM validation and final whole-package validation.</p>
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
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 0.42rem;
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
