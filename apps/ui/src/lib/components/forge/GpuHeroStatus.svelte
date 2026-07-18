<script>
  import { Check, ChevronDown, CircleCheck, Play, RotateCcw, ShieldCheck, TriangleAlert } from "@lucide/svelte";
  import StatusBadge from "./StatusBadge.svelte";
  import gpuHero from "../../assets/gpu-hero.png";

  let {
    error = null,
    applied = null,
    hardware = null,
    powerSweep = null,
    safeLoop = null,
    powerRunning = false,
    hasProfiles = false,
    hasKnowledge = false,
    verification = null,
    theme = "command",
    forgeMode = "standard",
    onStartPower,
    onForgeModeChange,
    onReset,
    onFullReset,
    onRecoverContinue,
  } = $props();
  let modePicker = $state(null);

  const primaryGpu = $derived(hardware?.gpu?.[0] ?? null);
  const gpuName = $derived(primaryGpu?.model ?? "NVIDIA GPU");
  const vramGb = $derived(primaryGpu?.vram_mb ? Math.round(primaryGpu.vram_mb / 1024) : null);
  const gpuSubtitle = $derived.by(() => {
    const parts = [];
    if (primaryGpu?.vendor) parts.push(primaryGpu.vendor);
    if (vramGb) parts.push(`${vramGb} GB`);
    return parts.length ? parts.join(" · ") : "GPU detectada";
  });
  // Only Boost Clock / Memory / VRAM have backing hardware fields; the rest render "—".
  // Memory also shows an effective GDDR6 data rate (clock x4) alongside the raw MHz, like the reference design.
  const specs = $derived([
    { label: "Stock Clock", value: "—" },
    { label: "Boost Clock", value: primaryGpu?.max_core_clock_mhz ? `${primaryGpu.max_core_clock_mhz} MHz` : "—" },
    {
      label: "Memory",
      value: primaryGpu?.max_memory_clock_mhz
        ? `${primaryGpu.max_memory_clock_mhz} MHz (${Math.round((primaryGpu.max_memory_clock_mhz * 4) / 1000)} Gbps)`
        : "—",
    },
    { label: "VRAM", value: vramGb ? `${vramGb} GB` : "—" },
    { label: "TDP", value: "—" },
    { label: "Power Cap", value: "—" },
  ]);
  const stateOrder = [
    { label: "Raw", variant: "raw" },
    { label: "Forging", variant: "forging" },
    { label: "Tempered", variant: "tempered" },
    { label: "Refined", variant: "refined" },
    { label: "Forged", variant: "forged" },
  ];
  const forgeState = $derived.by(() => {
    if (powerRunning) return "Forging";
    if (applied?.core && hasProfiles) return "Forged";
    if (hasProfiles) return "Refined";
    if (hasKnowledge) return "Tempered";
    return "Raw";
  });
  const forgeStateClass = $derived(`state-${forgeState.toLowerCase()}`);
  const forgeStateVariant = $derived(forgeState.toLowerCase());
  const forgeStateIndex = $derived(stateOrder.findIndex((state) => state.label === forgeState));
  const forgeStateSymbol = $derived.by(() => {
    if (forgeState === "Forging") return "activity";
    if (forgeState === "Forged" || forgeState === "Refined") return "check";
    return null;
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
  const safetySymbol = $derived(safetyVariant === "attention" ? "attention" : "shield");
  const currentProfile = $derived(applied?.label ?? "Stock");
  const hasAppliedTuning = $derived(Boolean(applied?.core || applied?.mem_offset_mhz));
  const hasForgeRun = $derived(Boolean(powerSweep && powerSweep.phase !== "idle"));
  const hasResettableState = $derived(powerRunning || hasAppliedTuning || hasProfiles || hasForgeRun);
  const verificationText = $derived.by(() => {
    if (!verification) return "Curve verification: Not checked";
    if (verification.status === "verified_curve") return "Curve verification: Verified";
    if (verification.status === "live_mismatch") return "Curve verification: Mismatch";
    return "Curve verification: Unavailable";
  });
  const verificationClass = $derived.by(() => {
    if (!verification) return "unchecked";
    if (verification.status === "verified_curve") return "verified";
    if (verification.status === "live_mismatch") return "mismatch";
    return "unavailable";
  });
  const technicalSummary = $derived.by(() => {
    const parts = [];
    if (applied?.core) parts.push(`${applied.core.freq_mhz} MHz target`);
    if (applied?.mem_offset_mhz) parts.push(`Memory +${applied.mem_offset_mhz} MHz`);
    return parts.length ? parts.join(" / ") : "Stock clocks active";
  });
  const curveAnchorSummary = $derived(
    verification?.vf_table_voltage_mv != null ? `Curve anchor: ${verification.vf_table_voltage_mv} mV` : null,
  );
  const measuredVoltageSummary = $derived.by(() => {
    const avg = verification?.avg_measured_voltage_mv;
    const min = verification?.min_measured_voltage_mv;
    const max = verification?.max_measured_voltage_mv;
    if (avg != null && min != null && max != null) {
      return `Measured voltage under load: ${avg} / ${min} / ${max} mV`;
    }
    return null;
  });

  // Recommended-action state machine (merged in from the former RecommendedAction.svelte card).
  const profilesQualified = $derived(!powerSweep?.is_undervolt || Boolean(powerSweep?.profiles_qualified));
  const needsAttention = $derived(Boolean(safeLoop?.safe_mode || safeLoop?.state === "unstable"));
  const currentPhase = $derived(powerSweep?.phase && powerSweep.phase !== "idle" ? powerSweep.phase : null);
  const isInterrupted = $derived(powerSweep?.phase === "interrupted");
  const canResetState = $derived(Boolean(onReset) && (needsAttention || hasAppliedTuning || hasProfiles || hasForgeRun));
  const canFullResetState = $derived(Boolean(onFullReset) && (needsAttention || hasAppliedTuning || hasProfiles || hasForgeRun));
  const canRecoverContinue = $derived(Boolean(onRecoverContinue) && (needsAttention || isInterrupted) && !powerRunning);
  const actionTitle = $derived.by(() => {
    if (isInterrupted) return "Forge interrupted";
    if (needsAttention) return "Needs Attention";
    if (!hasProfiles && !applied?.core) return "Raw GPU Detected";
    if (hasProfiles && !profilesQualified && !applied?.core) return "Profiles need qualification";
    if (hasProfiles && !applied?.core) return "Profiles are ready";
    if (hasProfiles) return "Profile applied";
    return "Ready to forge";
  });
  const actionBody = $derived.by(() => {
    if (isInterrupted && needsAttention) {
      return "The previous Forge was interrupted and recovery is latched. Recover & continue resets to stock, clears recovery, preserves learned observations, then starts the selected Forge mode.";
    }
    if (isInterrupted) {
      return "The previous Forge run did not finish cleanly. Continue with the selected mode to reuse saved learning, or use Full reset only if you want to discard it.";
    }
    if (needsAttention) {
      return "Nidavellir detected a safety condition. Recover & continue clears the recovery latch while keeping Forge learning available for the next run.";
    }
    if (!hasProfiles && !applied?.core) {
      return "Nidavellir has detected your NVIDIA GPU. The current Forge GPU action runs the implemented core VF forge and profile generation path.";
    }
    if (hasProfiles && !profilesQualified && !applied?.core) {
      return "The previous run ended without the complete proof. Run Standard again or choose Long explicitly; Apply stays locked until qualification is complete.";
    }
    if (hasProfiles && !applied?.core) {
      return "Nidavellir has generated profiles. Choose one below, or refine the core VF profiles by running the forge again.";
    }
    return "Your applied profile will be re-applied automatically on boot with Safe Loop protection. You can refine core VF profiles at any time.";
  });
  const primaryLabel = $derived(isInterrupted ? "Continue Forge" : hasProfiles ? "Refine Profiles" : "Forge GPU");
  const actionDotClass = $derived.by(() => {
    if (isInterrupted || needsAttention) return "danger";
    if (hasProfiles && applied?.core) return "green";
    return "gold";
  });
  const firstRunSteps = ["Safe Loop check", "Multi-clock discovery", "Stability confidence", "Profile creation"];
  const forgeModes = [
    {
      id: "standard",
      label: "Standard",
      summaryLabel: "Std",
      meta: "up to 1 hour",
      title: "Bounded Texture Hop qualification",
      description: "Runs the aggressive Texture Hop detector and compact Endurance proof. At one hour it stops safely and keeps incomplete profiles locked.",
    },
    {
      id: "long",
      label: "Long",
      summaryLabel: "Long",
      meta: "may exceed 1 hour",
      title: "Exhaustive qualification",
      description: "Keeps the full five-minute Texture Hop and twenty-minute thermal Endurance proof. This is the only mode allowed to run beyond one hour.",
    },
    {
      id: "clean",
      label: "Clean run",
      summaryLabel: "Clean",
      meta: "experimental",
      title: "Organic search — no historical memory",
      description: "Uses the Standard one-hour budget, but starts organically: pre-run learning is archived and only failures from this run steer it. Sentinel and Safe Loop remain active.",
    },
  ];
  const selectedMode = $derived(forgeModes.find((mode) => mode.id === forgeMode) ?? forgeModes[0]);

  function selectMode(mode) {
    if (powerRunning) return;
    onForgeModeChange?.(mode);
    modePicker?.removeAttribute("open");
  }

  function startSelectedMode() {
    if (needsAttention || powerRunning) return;
    onStartPower?.(forgeMode);
  }

  function recoverSelectedMode() {
    if (!canRecoverContinue) return;
    onRecoverContinue?.(forgeMode);
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

{#if error}
  <p class="err">{error}</p>
{/if}

<section class={`gpu-hero ${forgeStateClass}`}>
  <div class="id-strip">
    {#if theme === "command"}
      <img class="command-gpu-art" src={gpuHero} alt="" aria-hidden="true" />
    {/if}
    <div class="id-left">
      <span class="gpu-swatch" aria-hidden="true"></span>
      <div class="id-copy">
        <h2>{gpuName}</h2>
        <span class="id-sub">{gpuSubtitle}</span>
      </div>
    </div>
    <div class="id-right">
      <span class={`prof-dot ${forgeStateVariant}`} aria-hidden="true"></span>
      <span class="prof-active">Profile ativo: <strong>{currentProfile}</strong></span>
    </div>
  </div>

  <div class="specs-row">
    {#each specs as chip}
      <div class="spec-chip">
        <span class="spec-kicker">{chip.label}</span>
        <span class="spec-val">{chip.value}</span>
      </div>
    {/each}
  </div>

  <div class="hero-secondary">
    <div class="hero-states">
      <StatusBadge label={forgeState} variant={forgeStateVariant} symbol={forgeStateSymbol} compact />
      <StatusBadge label={safetyState} variant={safetyVariant} symbol={safetySymbol} compact />
    </div>

    <div class="banner-row">
      <div class="banner-copy">
        <h3><span class={`status-dot ${actionDotClass}`} aria-hidden="true"></span>{actionTitle}</h3>
        <p>{actionBody}</p>
      </div>

      {#if powerRunning}
        <div class="banner-actions">
          <span class="running-hint">Forging is in progress — see the panel below to stop.</span>
        </div>
      {:else if needsAttention}
        <div class="banner-actions">
          {#if canRecoverContinue}
            <div class="action-group" use:dismissPicker>
              <button class="btn action-primary" onclick={recoverSelectedMode}>
                <Play size={15} strokeWidth={1.9} />
                <span>Recover & continue</span>
              </button>
              <details class="mode-picker" bind:this={modePicker}>
                <summary
                  class="mode-summary"
                  aria-label={`Select forge mode for recovery. Current mode: ${selectedMode.label}`}
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
                  <p class="mode-safety">Clears recovery first · preserves learned observations</p>
                </div>
              </details>
            </div>
          {:else}
            <StatusBadge label="Review Safety" variant="attention" symbol="attention" />
          {/if}
          {#if canResetState}
            <button class="btn reset-all" onclick={onReset}>
              <RotateCcw size={14} strokeWidth={1.9} />
              <span>Reset all</span>
            </button>
          {/if}
          {#if canFullResetState}
            <button class="btn full-reset" onclick={onFullReset}>
              <RotateCcw size={14} strokeWidth={1.9} />
              <span>Full reset</span>
            </button>
          {/if}
        </div>
      {:else}
        <div class="banner-actions">
          <div class="action-group" use:dismissPicker>
            <button class="btn action-primary" onclick={startSelectedMode}>
              <Play size={15} strokeWidth={1.9} />
              <span>{primaryLabel}</span>
            </button>
            <details class="mode-picker" bind:this={modePicker}>
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
          {#if canResetState}
            <button class="btn reset-all" onclick={onReset}>
              <RotateCcw size={14} strokeWidth={1.9} />
              <span>Reset all</span>
            </button>
          {/if}
          {#if canFullResetState}
            <button class="btn full-reset" onclick={onFullReset}>
              <RotateCcw size={14} strokeWidth={1.9} />
              <span>Full reset</span>
            </button>
          {/if}
        </div>
      {/if}
    </div>

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
      </div>
    {/if}

    <div class="state-rail" aria-label="Forge state progression">
      {#each stateOrder as state, index}
        <span
          class={`rail-step ${state.variant}`}
          class:active={state.label === forgeState}
          class:complete={index < forgeStateIndex}
          class:future={index > forgeStateIndex}
        >
          <i></i>
          <span>{state.label}</span>
        </span>
      {/each}
    </div>

    <div class="profile-summary">
      <span class="lab">Current Profile</span>
      <strong>{currentProfile}</strong>
      <small>{technicalSummary}</small>
      {#if applied?.core}
        <small>Optimized boost curve</small>
      {/if}
      {#if curveAnchorSummary}
        <small>{curveAnchorSummary}</small>
        <small>Not a hard voltage cap. Measured voltage can vary by workload.</small>
      {/if}
      {#if measuredVoltageSummary}
        <small>{measuredVoltageSummary}</small>
      {/if}
      <small class={`verification ${verificationClass}`}>
        {#if verificationClass === "verified"}
          <CircleCheck size={13} strokeWidth={1.9} />
        {:else if verificationClass === "mismatch"}
          <TriangleAlert size={13} strokeWidth={1.9} />
        {:else}
          <ShieldCheck size={13} strokeWidth={1.9} />
        {/if}
        <span>{verificationText}</span>
      </small>
      {#if applied?.message}
        <small class="applied-msg">{applied.message}</small>
      {/if}
    </div>
  </div>
</section>

<style>
  .gpu-hero {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }
  .id-strip {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
    flex-wrap: wrap;
    background: var(--forge-panel);
    border-radius: 12px;
    padding: 1rem 1.15rem;
  }
  .id-left {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    min-width: 0;
  }
  .gpu-swatch {
    flex-shrink: 0;
    width: 2.15rem;
    height: 2.15rem;
    border-radius: 8px;
    background: var(--forge-gold);
  }
  .id-copy {
    min-width: 0;
  }
  h2 {
    margin: 0;
    color: var(--forge-text);
    font-size: 1.06rem;
    font-weight: 700;
    line-height: 1.2;
  }
  .id-sub {
    display: block;
    margin-top: 0.12rem;
    font-size: 0.69rem;
    font-weight: 400;
    color: var(--forge-muted);
    line-height: 1.3;
  }
  .id-right {
    display: inline-flex;
    align-items: center;
    gap: 0.42rem;
    font-size: 0.75rem;
    font-weight: 600;
    color: var(--forge-muted);
    white-space: nowrap;
  }
  .id-right strong {
    color: var(--forge-text);
    font-weight: 700;
  }
  .prof-dot {
    width: 0.5rem;
    height: 0.5rem;
    border-radius: 999px;
    background: var(--forge-dim);
    flex-shrink: 0;
  }
  .prof-dot.forging {
    background: var(--forge-blue);
  }
  .prof-dot.refined {
    background: var(--forge-green);
  }
  .prof-dot.forged {
    background: var(--forge-gold);
  }
  .prof-dot.tempered {
    background: var(--forge-copper);
  }
  .specs-row {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
  }
  .spec-chip {
    display: flex;
    flex-direction: column;
    gap: 0.18rem;
    min-width: 85px;
    background: var(--forge-panel-raised);
    border-radius: 8px;
    padding: 0.56rem 0.75rem;
  }
  .spec-kicker {
    font-size: 0.56rem;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--forge-dim);
  }
  .spec-val {
    font-size: 0.88rem;
    font-weight: 600;
    color: var(--forge-text);
    font-variant-numeric: tabular-nums;
  }
  .hero-secondary {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    background: var(--forge-panel);
    border-radius: 12px;
    padding: 0.95rem 1.05rem;
  }
  .hero-states {
    display: flex;
    gap: 0.36rem;
    flex-wrap: wrap;
  }
  .banner-row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: 1rem;
    align-items: start;
  }
  .banner-copy h3 {
    display: inline-flex;
    align-items: center;
    gap: 0.5rem;
    margin: 0;
    color: var(--text);
    font-size: 1.05rem;
  }
  .status-dot {
    width: 0.55rem;
    height: 0.55rem;
    border-radius: 999px;
    background: var(--forge-gold);
    flex-shrink: 0;
  }
  .status-dot.green {
    background: var(--forge-green);
  }
  .status-dot.danger {
    background: var(--forge-red);
  }
  .banner-copy p {
    margin: 0.4rem 0 0;
    color: var(--muted);
    font-size: 0.85rem;
    line-height: 1.5;
    max-width: 58ch;
  }
  .err {
    color: var(--nord-danger);
    font-size: 0.9rem;
  }
  .banner-actions {
    --action-button-height: 2.5rem;
    display: flex;
    justify-content: flex-end;
    align-items: flex-start;
    gap: 0.5rem;
    flex-wrap: wrap;
  }
  .running-hint {
    color: var(--muted);
    font-size: 0.78rem;
    font-style: italic;
    white-space: nowrap;
  }
  .btn {
    display: inline-flex;
    box-sizing: border-box;
    align-items: center;
    justify-content: center;
    gap: 0.42rem;
    height: var(--action-button-height);
    border: 1px solid var(--border);
    border-radius: 9px;
    padding: 0 1.1rem;
    font-weight: 600;
    font-size: 0.85rem;
    cursor: pointer;
    background: rgba(8, 11, 16, 0.66);
    color: var(--text);
    white-space: nowrap;
  }
  .action-group {
    display: inline-grid;
    grid-template-columns: auto auto;
    height: var(--action-button-height);
    border-radius: 10px;
    overflow: hidden;
  }
  .action-group .btn {
    height: var(--action-button-height);
    border: 0;
    padding-inline: 0.82rem;
    background: var(--forge-gold);
  }
  .action-primary {
    border-radius: 9px 0 0 9px;
    color: var(--forge-ink);
    font-weight: 700;
  }
  .action-primary :global(svg) {
    margin-left: 1px;
  }
  .btn.reset-all {
    height: var(--action-button-height);
    color: #f3c9a6;
    border-color: rgba(214, 168, 93, 0.36);
    background: rgba(214, 124, 93, 0.11);
  }
  .btn.reset-all:hover,
  .btn.reset-all:focus-visible {
    color: var(--forge-gold);
    border-color: rgba(214, 168, 93, 0.58);
    background: rgba(214, 168, 93, 0.15);
  }
  .btn.full-reset {
    height: var(--action-button-height);
    color: #f3b9bd;
    border-color: rgba(191, 97, 106, 0.4);
    background: rgba(191, 97, 106, 0.11);
  }
  .btn.full-reset:hover,
  .btn.full-reset:focus-visible {
    color: #ffd1d5;
    border-color: rgba(191, 97, 106, 0.62);
    background: rgba(191, 97, 106, 0.17);
  }
  .mode-picker {
    position: relative;
  }
  .mode-summary {
    display: flex;
    box-sizing: border-box;
    justify-content: center;
    min-width: 3.25rem;
    height: var(--action-button-height);
    align-items: center;
    gap: 0.22rem;
    border-left: 1px solid rgba(11, 15, 20, 0.28);
    border-radius: 0 9px 9px 0;
    padding: 0 0.5rem;
    background: var(--forge-gold);
    color: var(--forge-ink);
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
    background: color-mix(in srgb, var(--forge-gold), black 10%);
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
    border-top: 1px solid rgba(255, 255, 255, 0.06);
    padding-top: 0.75rem;
  }
  .first-run > span {
    display: block;
    font-size: 0.68rem;
    font-weight: 700;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--nord-dim);
    margin-bottom: 0.35rem;
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
  .state-rail {
    display: grid;
    grid-template-columns: repeat(5, minmax(0, 1fr));
    gap: 0.32rem;
    padding-top: 0.58rem;
    border-top: 1px solid rgba(189, 166, 126, 0.11);
    overflow-x: auto;
  }
  .rail-step {
    --rail-color: var(--forge-muted);
    display: flex;
    flex-direction: column;
    gap: 0.18rem;
    color: var(--forge-dim);
    font-size: 0.54rem;
    font-weight: 800;
    letter-spacing: 0.06em;
    line-height: 1.2;
    text-transform: uppercase;
  }
  .rail-step i {
    display: block;
    height: 2px;
    border-radius: 999px;
    background: rgba(255, 255, 255, 0.06);
  }
  .rail-step.raw {
    --rail-color: var(--forge-steel);
  }
  .rail-step.forging {
    --rail-color: var(--forge-blue);
  }
  .rail-step.tempered {
    --rail-color: var(--forge-copper);
  }
  .rail-step.refined {
    --rail-color: var(--forge-green);
  }
  .rail-step.forged {
    --rail-color: var(--forge-gold);
  }
  .rail-step.complete,
  .rail-step.active {
    color: var(--rail-color);
  }
  .rail-step.complete i {
    background: rgba(157, 191, 145, 0.18);
  }
  .rail-step.active i {
    background: var(--rail-color);
    box-shadow: 0 0 10px rgba(214, 168, 93, 0.12);
  }
  .rail-step.future {
    opacity: 0.42;
  }
  .profile-summary {
    min-width: 0;
    border-radius: 8px;
    background: var(--forge-panel-raised);
    padding: 0.56rem 0.68rem;
  }
  .lab {
    display: block;
    color: var(--nord-dim);
    font-size: 0.62rem;
    font-weight: 800;
    letter-spacing: 0.08em;
    line-height: 1.1;
    text-transform: uppercase;
  }
  .profile-summary strong {
    display: block;
    margin-top: 0.22rem;
    color: var(--text);
    font-size: 0.96rem;
    line-height: 1.2;
  }
  .profile-summary small {
    display: block;
    margin-top: 0.18rem;
    color: var(--muted);
    font-size: 0.76rem;
    font-variant-numeric: tabular-nums;
    line-height: 1.3;
  }
  .profile-summary .applied-msg {
    color: var(--nord-dim);
  }
  .profile-summary .verification {
    display: inline-flex;
    align-items: center;
    gap: 0.32rem;
    color: var(--nord-dim);
  }
  .profile-summary .verification.verified {
    color: var(--forge-green);
  }
  .profile-summary .verification.mismatch {
    color: var(--forge-red);
  }
  .profile-summary .verification.unavailable {
    color: var(--forge-copper);
  }
  @media (max-width: 760px) {
    .id-strip {
      align-items: flex-start;
    }
    .banner-row {
      grid-template-columns: 1fr;
    }
    .banner-actions {
      justify-content: flex-start;
    }
    .step-chips {
      grid-template-columns: 1fr;
    }
    .mode-menu {
      right: auto;
      left: 0;
      width: min(19rem, calc(100vw - 3rem));
    }
  }
</style>
