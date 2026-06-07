<script>
  import { Gauge, Maximize2, ShieldCheck } from "@lucide/svelte";
  import { t } from "../../i18n.js";
  import VfChart from "../VfChart.svelte";

  let {
    realCurve = null,
    validation = null,
    curveOverlay = null,
    advanced = $bindable(false),
    expanded = $bindable(false),
    onReadRealCurve,
    onStartValidation,
  } = $props();

  const overlaySource = $derived.by(() => {
    if (!curveOverlay?.anchorPrecise) return null;
    if (curveOverlay.anchorSource === "verified_vf_bin") return "Verified profile anchor";
    if (curveOverlay.anchorSource === "verification_vf_bin") return "Profile verification anchor";
    if (curveOverlay.anchorSource === "profile_vf_bin") return "Profile anchor";
    if (curveOverlay.anchorSource === "curve_read_plateau") return "Detected curve plateau";
    return "Curve anchor";
  });

  const overlaySummary = $derived.by(() => {
    if (!curveOverlay?.targetMhz) return null;
    const target = `Target: ${curveOverlay.targetMhz} MHz`;
    if (curveOverlay.anchorPrecise && curveOverlay.anchorMv != null) {
      return `${target} · Curve anchor: ${curveOverlay.anchorMv} mV`;
    }
    return target;
  });

  const overlayNote = $derived.by(() => {
    if (!curveOverlay?.targetMhz) return null;
    if (curveOverlay.showBand) {
      return "Optimized boost curve. Expected operating range is shown from the curve anchor across higher curve bins. Not a hard voltage cap. Measured voltage can vary by workload.";
    }
    return "Optimized boost curve. No deterministic curve anchor is available, so the chart shows the target line only. Not a hard voltage cap. Measured voltage can vary by workload.";
  });
</script>

<div class="vf-panel">
  <div class="real-head">
    <h3 class="section-head">
      <Gauge size={14} strokeWidth={1.85} />
      <span>{$t("forge.realTitle")}</span>
    </h3>
    <label class="adv-toggle">
      <input type="checkbox" bind:checked={advanced} /> {$t("forge.advanced")}
    </label>
  </div>
  <div class="real-actions">
    <button class="btn" onclick={onReadRealCurve}>
      <Gauge size={15} strokeWidth={1.9} />
      <span>{$t("forge.readCurve")}</span>
    </button>
    <button class="btn go" onclick={onStartValidation} disabled={validation?.running}>
      <ShieldCheck size={15} strokeWidth={1.9} />
      <span>{validation?.running ? $t("forge.validating") : $t("forge.validate")}</span>
    </button>
    {#if realCurve?.real}
      <button class="btn ghost" onclick={() => (expanded = true)}>
        <Maximize2 size={15} strokeWidth={1.9} />
        <span>{$t("forge.expand")}</span>
      </button>
    {/if}
  </div>

  {#if realCurve}
    {#if realCurve.real}
      {#if overlaySummary}
        <p class="point accent">
          {overlaySummary}
        </p>
        {#if overlaySource}
          <p class="sub curve-source">{overlaySource}</p>
        {/if}
        <p class="sub curve-note">{overlayNote}</p>
      {/if}
      <VfChart points={realCurve.points} overlay={curveOverlay} height={300} />
      <p class="sub vf-method" class:ok={realCurve.vf_curve_supported}>
        {$t(realCurve.vf_curve_supported ? "forge.vfElastic" : "forge.vfFallback")}
      </p>
      {#if advanced}
        <p class="sub">{$t("forge.curvePoints", { name: realCurve.name, n: realCurve.points.length })}</p>
        <ul class="list">
          {#each realCurve.points.filter((_, i) => i % 4 === 0) as p}
            <li><span class="mono">V/F bin {p.voltage_mv} mV</span><span class="mono accent">{p.freq_mhz} MHz</span></li>
          {/each}
        </ul>
      {/if}
    {:else}
      <p class="err">{realCurve.name}</p>
    {/if}
  {/if}

  {#if validation}
    <div class="val-box">
      {#if validation.error}
        <p class="err">{validation.error}</p>
      {/if}
      {#if validation.total_stages}
        <div class="stages">
          {#each Array(validation.total_stages) as _, i}
            {@const done = validation.stages[i]}
            {@const active = validation.running && i === validation.stage_index}
            <div class="stage" class:active class:done>
              <span class="stage-ic">
                {#if done}{done.result === "stable" ? "ok" : "x"}{:else if active}<span class="spin">*</span>{:else}.{/if}
              </span>
              <span class="stage-name">{done?.name ?? (active ? validation.current_stage : $t("forge.stageN", { n: i + 1 }))}</span>
              {#if done}
                <span class="stage-meta" class:danger={done.result !== "stable"}>
                  {$t("stage." + done.result)} / {done.mismatches} mm / {done.elapsed_ms} ms
                </span>
              {/if}
            </div>
          {/each}
        </div>
      {/if}
      {#if validation.running}
        <p class="sub">{$t("forge.running")}</p>
      {:else if validation.result}
        <p class="point" class:danger={validation.result !== "stable"} class:accent={validation.result === "stable"}>
          {$t("forge.result", { r: $t("val." + validation.result) })}
        </p>
        {#if validation.adapter}
          <p class="sub">{validation.adapter}</p>
        {/if}
      {/if}
    </div>
  {/if}
</div>

<style>
  .vf-panel {
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
  }
  .real-head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 1rem;
  }
  .section-head {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    margin: 0 0 0.5rem;
    font-size: 0.72rem;
    font-weight: 700;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--muted);
  }
  .adv-toggle {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    font-size: 0.75rem;
    color: var(--muted);
    cursor: pointer;
    user-select: none;
  }
  .real-actions {
    display: flex;
    gap: 0.6rem;
    margin-bottom: 0.6rem;
    flex-wrap: wrap;
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
  }
  .btn.go {
    background: rgba(214, 168, 93, 0.13);
    color: var(--forge-gold);
    border-color: rgba(214, 168, 93, 0.42);
  }
  .btn.ghost {
    background: transparent;
    color: var(--muted);
  }
  .btn:disabled {
    opacity: 0.55;
    cursor: default;
  }
  .point {
    margin: 0.3rem 0;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    color: var(--text);
  }
  .point.accent {
    color: var(--accent);
  }
  .point.danger {
    color: var(--nord-danger);
  }
  .sub {
    margin: 0.25rem 0;
    font-size: 0.82rem;
    color: var(--muted);
  }
  .vf-method.ok {
    color: var(--nord-success, #a3be8c);
  }
  .curve-note {
    color: var(--nord-dim);
  }
  .curve-source {
    color: var(--nord-mist);
  }
  .err {
    color: var(--nord-danger);
    font-size: 0.9rem;
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(150px, 1fr));
    gap: 0.4rem;
  }
  .list li {
    display: flex;
    justify-content: space-between;
    gap: 0.75rem;
    background: rgba(5, 7, 11, 0.26);
    border: 1px solid var(--forge-line);
    border-radius: 9px;
    padding: 0.5rem 0.75rem;
    font-size: 0.82rem;
  }
  .mono {
    font-variant-numeric: tabular-nums;
    color: var(--text);
  }
  .mono.accent {
    color: var(--accent);
  }
  .val-box {
    margin-top: 0.6rem;
  }
  .stages {
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
    margin: 0.5rem 0;
  }
  .stage {
    display: grid;
    grid-template-columns: 1.4rem 1fr auto;
    align-items: center;
    gap: 0.5rem;
    padding: 0.45rem 0.7rem;
    border-radius: 9px;
    border: 1px solid var(--forge-line);
    background: rgba(5, 7, 11, 0.26);
    font-size: 0.82rem;
    opacity: 0.7;
  }
  .stage.active,
  .stage.done {
    opacity: 1;
  }
  .stage.active {
    border-color: rgba(214, 168, 93, 0.42);
  }
  .stage-ic {
    text-align: center;
    color: var(--accent);
    font-weight: 700;
  }
  .stage-name {
    color: var(--text);
  }
  .stage-meta {
    font-variant-numeric: tabular-nums;
    color: var(--muted);
    font-size: 0.75rem;
  }
  .stage-meta.danger {
    color: var(--nord-danger);
  }
  .spin {
    display: inline-block;
    animation: spin 1s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
