<script>
  import { CircleCheck, RotateCcw, ShieldCheck, TriangleAlert } from "@lucide/svelte";
  import StatusBadge from "./StatusBadge.svelte";

  let {
    error = null,
    applied = null,
    hardware = null,
    safeLoop = null,
    powerRunning = false,
    hasProfiles = false,
    hasKnowledge = false,
    verification = null,
    onReset,
  } = $props();

  const primaryGpu = $derived(hardware?.gpu?.[0] ?? null);
  const gpuName = $derived(primaryGpu?.model ?? "NVIDIA GPU");
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
    if (applied?.core) parts.push(`${applied.core.freq_mhz} MHz @ ${applied.core.voltage_mv} mV`);
    if (applied?.mem_offset_mhz) parts.push(`Memory +${applied.mem_offset_mhz} MHz`);
    return parts.length ? parts.join(" / ") : "Stock clocks active";
  });
  const heroSummary = $derived.by(() => {
    if (forgeState === "Forged") return "Nidavellir has forged this GPU and selected a daily profile.";
    if (forgeState === "Forging") return "Nidavellir is testing this GPU and learning safe operating regions.";
    if (forgeState === "Refined") return "Nidavellir has profiles ready for review.";
    if (forgeState === "Tempered") return "Nidavellir has early stability knowledge for this GPU.";
    return "Nidavellir has detected this GPU and is ready to begin forging.";
  });
</script>

{#if error}
  <p class="err">{error}</p>
{/if}

<section class={`gpu-hero ${forgeStateClass}`}>
  <div class="hero-main">
    <div class="hero-copy">
      <span class="eyebrow">GPU Forge Home</span>
      <div class="title-row">
        <h2>{gpuName}</h2>
        <div class="hero-states">
          <StatusBadge label={forgeState} variant={forgeStateVariant} symbol={forgeStateSymbol} compact />
          <StatusBadge label={safetyState} variant={safetyVariant} symbol={safetySymbol} compact />
        </div>
      </div>
      <p class="lead">{heroSummary}</p>
    </div>

    <div class="profile-summary">
      <span class="lab">Current Profile</span>
      <strong>{currentProfile}</strong>
      <small>{technicalSummary}</small>
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

    <button class="btn reset" onclick={onReset}>
      <RotateCcw size={15} strokeWidth={1.85} />
      <span>Reset to stock</span>
    </button>
  </div>

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
</section>

<style>
  .gpu-hero {
    position: relative;
    overflow: hidden;
    background: var(--forge-panel-bg);
    border: 1px solid var(--forge-line);
    border-radius: 10px;
    padding: 0.85rem 0.95rem;
    box-shadow: var(--forge-panel-edge);
  }
  .gpu-hero::before {
    content: "";
    position: absolute;
    inset: 0 0 auto;
    height: 1px;
    background: linear-gradient(90deg, transparent, rgba(214, 168, 93, 0.36), transparent);
    opacity: 0.7;
  }
  .hero-main {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(210px, 280px) auto;
    gap: 0.85rem;
    align-items: center;
  }
  .title-row {
    display: flex;
    align-items: center;
    gap: 0.65rem;
    flex-wrap: wrap;
  }
  h2 {
    margin: 0;
    color: var(--text);
    font-size: 1.22rem;
    font-weight: 800;
    line-height: 1.15;
  }
  .eyebrow {
    display: block;
    margin-bottom: 0.25rem;
    font-size: 0.68rem;
    font-weight: 700;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--nord-dim);
  }
  .lead {
    margin: 0.32rem 0 0;
    font-size: 0.82rem;
    line-height: 1.45;
    color: var(--muted);
    max-width: 58ch;
  }
  .hero-states {
    display: flex;
    gap: 0.36rem;
    flex-wrap: wrap;
  }
  .err {
    color: var(--nord-danger);
    font-size: 0.9rem;
  }
  .profile-summary {
    min-width: 0;
    border: 1px solid rgba(255, 255, 255, 0.055);
    border-radius: 8px;
    background: rgba(5, 7, 11, 0.22);
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
  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 0.42rem;
    border: 1px solid var(--forge-line);
    border-radius: 8px;
    padding: 0.5rem 0.78rem;
    font-weight: 600;
    font-size: 0.78rem;
    cursor: pointer;
    background: rgba(8, 11, 16, 0.66);
    color: var(--text);
    white-space: nowrap;
    transition:
      border-color 0.15s ease,
      color 0.15s ease,
      background 0.15s ease;
  }
  .btn:hover {
    border-color: var(--forge-line-strong);
    color: var(--forge-gold);
  }
  .state-rail {
    display: grid;
    grid-template-columns: repeat(5, minmax(0, 1fr));
    gap: 0.32rem;
    margin-top: 0.62rem;
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
  @media (max-width: 760px) {
    .hero-main {
      grid-template-columns: 1fr;
      align-items: stretch;
    }
    .btn {
      width: fit-content;
    }
  }
</style>
