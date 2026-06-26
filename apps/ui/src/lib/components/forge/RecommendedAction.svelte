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
  const firstRunSteps = [
    "Safe Loop check",
    "Core VF learning",
    "Profile creation",
    "Daily recommendation",
  ];
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
      <span>Forge path</span>
      <div class="step-chips" aria-label="Forge GPU path">
        {#each firstRunSteps as step, index}
          <span class="step-chip">
            <i>{index + 1}</i>
            <strong>{step}</strong>
          </span>
        {/each}
      </div>
      <p>VRAM optimization and final whole-package validation stay planned later, after the core curve is forged.</p>
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
  .step-chips {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: 0.45rem;
    margin-top: 0.45rem;
  }
  .step-chip {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr);
    align-items: center;
    gap: 0.5rem;
    min-height: 2.5rem;
    border: 1px solid rgba(255, 255, 255, 0.055);
    border-radius: 8px;
    background: rgba(5, 7, 11, 0.24);
    padding: 0.5rem 0.6rem;
  }
  .step-chip strong {
    color: var(--text);
    font-size: 0.78rem;
    line-height: 1.25;
  }
  .step-chip i {
    width: 1.28rem;
    height: 1.28rem;
    border-radius: 999px;
    background: rgba(214, 168, 93, 0.14);
    color: var(--forge-gold);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font-size: 0.66rem;
    font-style: normal;
    font-weight: 800;
  }
  @media (max-width: 640px) {
    .next-action {
      grid-template-columns: 1fr;
    }
    .step-chips {
      grid-template-columns: 1fr;
    }
  }
</style>
