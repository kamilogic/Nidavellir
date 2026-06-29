<script>
  import { Check, ChevronDown, Play, Square } from "@lucide/svelte";
  import StatusBadge from "./StatusBadge.svelte";

  let {
    applied = null,
    powerSweep = null,
    powerRunning = false,
    safeLoop = null,
    forgeMode = "standard",
    onStartPower,
    onStopPower,
    onForgeModeChange,
  } = $props();
  let modePicker = $state(null);

  const hasProfiles = $derived(Boolean(powerSweep?.godforge || powerSweep?.brokkrs || powerSweep?.deep_calm));
  const profilesQualified = $derived(!powerSweep?.is_undervolt || Boolean(powerSweep?.profiles_qualified));
  const needsAttention = $derived(Boolean(safeLoop?.safe_mode || safeLoop?.state === "unstable"));
  const currentPhase = $derived(powerSweep?.phase && powerSweep.phase !== "idle" ? powerSweep.phase : null);
  const title = $derived.by(() => {
    if (needsAttention) return "Needs Attention";
    if (powerRunning) return "Core VF forge in progress";
    if (!hasProfiles && !applied?.core) return "Raw GPU Detected";
    if (hasProfiles && !profilesQualified && !applied?.core) return "Profiles need qualification";
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
    if (hasProfiles && !profilesQualified && !applied?.core) {
      return "Fast found provisional profile points. Run Standard or Long to qualify their sustained stability before Apply is unlocked.";
    }
    if (hasProfiles && !applied?.core) {
      return "Nidavellir has generated profiles. Choose one below, or refine the core VF profiles by running the forge again.";
    }
    return "Your applied profile will be re-applied automatically on boot with Safe Loop protection. You can refine core VF profiles at any time.";
  });
  const primaryLabel = $derived(hasProfiles ? "Refine Profiles" : "Forge GPU");
  const firstRunSteps = [
    "Safe Loop check",
    "Multi-clock discovery",
    "Stability confidence",
    "Profile creation",
  ];
  const forgeModes = [
    {
      id: "fast",
      label: "Fast",
      summaryLabel: "Fast",
      meta: "≈20–30m fresh",
      title: "10 s discovery · preview only",
      description: "Traverses the full physical frontier with short 10-second dwells. It discovers provisional points quickly, but Apply stays locked until Standard or Long qualifies them.",
    },
    {
      id: "standard",
      label: "Standard",
      summaryLabel: "Std",
      meta: "≈55–75m fresh",
      title: "10 s discovery + 2 × 60 s qualification",
      description: "Traverses the same full frontier, then requires two independent 60-second reset/reapply passes at every selected boundary. Learned GPUs usually resume faster.",
    },
    {
      id: "long",
      label: "Long",
      summaryLabel: "Long",
      meta: "≈90–120m fresh",
      title: "10 s discovery + 3 × 120 s qualification",
      description: "Traverses the same full frontier, then runs three independent two-minute passes per selected boundary for the strongest initial confidence.",
    },
  ];
  const selectedMode = $derived(forgeModes.find((mode) => mode.id === forgeMode) ?? forgeModes[1]);

  function selectMode(mode) {
    if (needsAttention || powerRunning) return;
    onForgeModeChange?.(mode);
    modePicker?.removeAttribute("open");
  }

  function startSelectedMode() {
    onStartPower?.(forgeMode);
  }

  function handlePickerKeydown(event) {
    if (event.key === "Escape" && modePicker?.open) {
      event.preventDefault();
      modePicker.open = false;
      modePicker.querySelector("summary")?.focus();
      return;
    }
    if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
    event.preventDefault();
    const items = Array.from(modePicker?.querySelectorAll(".mode-item") ?? []);
    if (!modePicker.open) modePicker.open = true;
    const current = items.indexOf(document.activeElement);
    const next =
      current < 0 ? (["ArrowUp", "End"].includes(event.key) ? items.length - 1 : 0) :
      event.key === "Home" ? 0 :
      event.key === "End" ? items.length - 1 :
      event.key === "ArrowDown" ? (current + 1) % items.length :
      (current - 1 + items.length) % items.length;
    queueMicrotask(() => items[next]?.focus());
  }

  function dismissPicker(node) {
    const dismiss = (event) => {
      if (!node.contains(event.target)) modePicker?.removeAttribute("open");
    };
    document.addEventListener("pointerdown", dismiss);
    document.addEventListener("focusin", dismiss);
    return {
      destroy() {
        document.removeEventListener("pointerdown", dismiss);
        document.removeEventListener("focusin", dismiss);
      },
    };
  }
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
    <div class="action-group" use:dismissPicker>
      <button class="btn action-primary" onclick={startSelectedMode}>
        <Play size={15} strokeWidth={1.9} />
        <span>{primaryLabel}</span>
      </button>
      <details
        class="mode-picker"
        bind:this={modePicker}
      >
        <summary
          class="mode-summary"
          aria-label={`Select forge mode. Current mode: ${selectedMode.label}`}
          title={`${selectedMode.title}. ${selectedMode.description}`}
          onkeydown={handlePickerKeydown}
        >
          <span>{selectedMode.summaryLabel}</span>
          <ChevronDown size={14} strokeWidth={2} />
        </summary>
        <div class="mode-menu" role="menu" tabindex="-1" onkeydown={handlePickerKeydown}>
          {#each forgeModes as mode}
            <button
              type="button"
              class="mode-item"
              class:selected={forgeMode === mode.id}
              role="menuitemradio"
              aria-checked={forgeMode === mode.id}
              onclick={() => selectMode(mode.id)}
            >
              <span class="mode-copy">
                <strong>{mode.label}<small>{mode.meta}</small></strong>
                <span>{mode.title}</span>
              </span>
              <span class="mode-check" class:visible={forgeMode === mode.id}>
                <Check size={14} strokeWidth={2.1} />
              </span>
            </button>
          {/each}
          <p class="mode-safety">Supervised and fail-closed · Nothing is auto-applied</p>
        </div>
      </details>
    </div>
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
      <p>Fast creates a provisional map. Standard or Long qualifies the selected boundaries before Apply is unlocked. VRAM optimization and final whole-package validation stay planned later.</p>
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
  .btn.stop {
    background: rgba(191, 97, 106, 0.16);
    color: #f3b9bd;
    border-color: rgba(191, 97, 106, 0.45);
  }
  .action-group {
    display: inline-grid;
    grid-template-columns: auto auto;
    border-radius: 10px;
    box-shadow:
      0 0 0 1px rgba(214, 168, 93, 0.42),
      0 8px 20px rgba(0, 0, 0, 0.16);
  }
  .action-group .btn {
    min-height: 2.5rem;
    border: 0;
    padding-inline: 0.82rem;
    background: rgba(214, 168, 93, 0.13);
  }
  .action-primary {
    border-radius: 9px 0 0 9px;
    color: var(--forge-gold);
  }
  .action-primary :global(svg) {
    margin-left: 1px;
  }
  .mode-picker {
    position: relative;
  }
  .mode-summary {
    display: flex;
    justify-content: center;
    min-width: 3.25rem;
    min-height: 2.5rem;
    align-items: center;
    gap: 0.22rem;
    border-left: 1px solid rgba(214, 168, 93, 0.28);
    border-radius: 0 9px 9px 0;
    padding: 0.5rem;
    background: rgba(214, 168, 93, 0.13);
    color: var(--forge-text);
    font-size: 0.72rem;
    font-weight: 700;
    cursor: pointer;
    list-style: none;
    transition-property: background-color, color;
    transition-duration: 150ms;
    transition-timing-function: ease-out;
  }
  .mode-summary::-webkit-details-marker {
    display: none;
  }
  .mode-summary:hover,
  .mode-picker[open] .mode-summary {
    background: rgba(214, 168, 93, 0.19);
  }
  .mode-summary :global(svg) {
    transition: rotate 150ms cubic-bezier(0.2, 0, 0, 1);
  }
  .mode-picker[open] .mode-summary :global(svg) {
    rotate: 180deg;
  }
  .mode-menu {
    position: absolute;
    z-index: 10;
    top: calc(100% + 0.45rem);
    right: 0;
    width: 19rem;
    border-radius: 12px;
    padding: 4px;
    background: rgba(14, 19, 27, 0.98);
    box-shadow:
      0 0 0 1px rgba(255, 255, 255, 0.09),
      0 18px 42px rgba(0, 0, 0, 0.42);
  }
  .mode-item {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    align-items: center;
    gap: 0.6rem;
    width: 100%;
    min-height: 3.1rem;
    border: 0;
    border-radius: 8px;
    padding: 0.52rem 0.62rem;
    text-align: left;
    color: var(--muted);
    background: transparent;
    cursor: pointer;
    transition-property: background-color, color, scale;
    transition-duration: 150ms;
    transition-timing-function: cubic-bezier(0.2, 0, 0, 1);
  }
  .mode-item:hover,
  .mode-item:focus-visible {
    color: var(--text);
    background: rgba(255, 255, 255, 0.055);
  }
  .mode-item.selected {
    color: var(--forge-gold);
    background: rgba(214, 168, 93, 0.1);
  }
  .mode-copy strong,
  .mode-copy > span {
    display: block;
  }
  .mode-copy strong {
    font-size: 0.8rem;
  }
  .mode-copy small {
    margin-left: 0.36rem;
    color: var(--nord-dim);
    font-size: 0.58rem;
    font-weight: 700;
    text-transform: uppercase;
  }
  .mode-copy > span {
    margin-top: 0.14rem;
    color: var(--nord-dim);
    font-size: 0.66rem;
  }
  .mode-check {
    display: inline-flex;
    opacity: 0;
    scale: 0.25;
    filter: blur(4px);
    transition-property: opacity, scale, filter;
    transition-duration: 200ms;
    transition-timing-function: cubic-bezier(0.2, 0, 0, 1);
  }
  .mode-check.visible {
    opacity: 1;
    scale: 1;
    filter: blur(0);
  }
  .mode-safety {
    margin: 4px 0 0;
    border-top: 1px solid rgba(255, 255, 255, 0.07);
    padding: 0.48rem 0.62rem 0.42rem;
    color: var(--nord-dim);
    font-size: 0.65rem;
    line-height: 1.4;
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
    .action-group {
      justify-self: start;
    }
    .mode-menu {
      right: auto;
      left: 0;
      width: min(19rem, calc(100vw - 3rem));
    }
  }
</style>
