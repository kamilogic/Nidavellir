<script>
  import { Database } from "@lucide/svelte";
  import { t } from "../../i18n.js";

  let { realSweep = null, powerSweep = null, validation = null, summary = false, compact = false } = $props();

  const stableCount = $derived(realSweep?.tradeoffs?.length ?? 0);
  const measuredPowerPoints = $derived(powerSweep?.points?.length ?? 0);
  const hasKnowledge = $derived(Boolean(stableCount || measuredPowerPoints || realSweep?.validation_note || validation?.result));
  const latestBoundary = $derived(realSweep?.tradeoffs?.[realSweep.tradeoffs.length - 1] ?? null);
  const stablePowerPoints = $derived((powerSweep?.points ?? []).filter((point) => point.stable).length);
  const stableRegionCount = $derived(stableCount || stablePowerPoints);
  const latestPowerStable = $derived.by(() => {
    const points = powerSweep?.points ?? [];
    return [...points].reverse().find((point) => point.stable) ?? null;
  });
  const knownSafeEdgeTarget = $derived.by(() => {
    if (latestBoundary) return `${latestBoundary.freq_mhz} MHz target`;
    if (latestPowerStable) return `${latestPowerStable.clock_mhz} MHz target`;
    return "Not learned yet";
  });
  const knownSafeEdgeAnchor = $derived(
    latestPowerStable?.vf_table_voltage_mv != null ? `Curve anchor: ${latestPowerStable.vf_table_voltage_mv} mV` : null,
  );
</script>

{#if summary}
  <section class="knowledge" class:compact>
    <div>
      <span class="eyebrow">Forge Knowledge</span>
      <h3>
        <Database size={17} strokeWidth={1.85} />
        <span>What Nidavellir has learned</span>
      </h3>
      {#if hasKnowledge}
        <p>{compact ? "Live summary from this GPU's forge data." : "Knowledge is based only on completed validation and sweep results from this GPU."}</p>
      {:else}
        <p>Forge Knowledge is being built from completed forge runs.</p>
      {/if}
    </div>

    <div class="knowledge-grid">
      <article>
        <span>Stable regions</span>
        <strong>{stableRegionCount ? `${stableRegionCount} learned point${stableRegionCount === 1 ? "" : "s"}` : "Unknown"}</strong>
      </article>
      <article>
        <span>Known safe edge</span>
        <strong>{knownSafeEdgeTarget}</strong>
        {#if knownSafeEdgeAnchor}
          <small>{knownSafeEdgeAnchor}</small>
        {/if}
      </article>
      <article>
        <span>Latest validation</span>
        <strong>{validation?.result ?? realSweep?.last_result ?? "Not run yet"}</strong>
      </article>
      <article>
        <span>Power data</span>
        <strong>{measuredPowerPoints ? `${measuredPowerPoints} measured point${measuredPowerPoints === 1 ? "" : "s"}` : "Not measured yet"}</strong>
      </article>
    </div>

    {#if realSweep?.validation_note}
      <p class="note">{realSweep.validation_note}</p>
    {/if}
  </section>
{:else}
  {#if realSweep?.tradeoffs?.length}
    <h5 class="section-head">{$t("forge.realResult")}</h5>
    <ul class="list">
      {#each realSweep.tradeoffs as tp}
        <li><span class="mono">{tp.freq_mhz} MHz</span><span class="mono accent">{tp.vmin_mv} mV</span></li>
      {/each}
    </ul>
  {/if}

  {#if realSweep?.validation_note}
    <p class="note">{realSweep.validation_note}</p>
  {/if}
{/if}

<style>
  .knowledge {
    background: var(--forge-panel-bg);
    border: 1px solid var(--forge-line);
    border-radius: 12px;
    padding: 1rem 1.1rem;
    display: flex;
    flex-direction: column;
    gap: 0.8rem;
    box-shadow: var(--forge-panel-edge);
  }
  .knowledge.compact {
    background: rgba(14, 18, 24, 0.58);
    border-color: rgba(255, 255, 255, 0.045);
    padding: 0.78rem 0.9rem;
    gap: 0.62rem;
    box-shadow: none;
  }
  .eyebrow,
  .knowledge-grid span {
    display: block;
    font-size: 0.68rem;
    font-weight: 700;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--nord-dim);
    margin-bottom: 0.35rem;
  }
  h3 {
    display: inline-flex;
    align-items: center;
    gap: 0.42rem;
    margin: 0;
    color: var(--text);
    font-size: 1rem;
  }
  .compact h3 {
    color: var(--nord-mist);
    font-size: 0.92rem;
  }
  p {
    margin: 0.35rem 0 0;
    color: var(--muted);
    font-size: 0.86rem;
    line-height: 1.5;
  }
  .compact p {
    font-size: 0.8rem;
  }
  .knowledge-grid {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 0.65rem;
  }
  .knowledge-grid article {
    background: rgba(5, 7, 11, 0.28);
    border: 1px solid rgba(255, 255, 255, 0.055);
    border-radius: 8px;
    padding: 0.7rem 0.8rem;
  }
  .compact .knowledge-grid article {
    background: rgba(5, 7, 11, 0.18);
    padding: 0.56rem 0.65rem;
  }
  .knowledge-grid strong {
    display: block;
    color: var(--text);
    font-size: 0.88rem;
    font-variant-numeric: tabular-nums;
  }
  .knowledge-grid small {
    display: block;
    margin-top: 0.2rem;
    color: var(--muted);
    font-size: 0.74rem;
    line-height: 1.35;
  }
  .section-head {
    margin: 0 0 0.5rem;
    font-size: 0.72rem;
    font-weight: 700;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--muted);
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
  .note {
    margin: 0;
    font-size: 0.8rem;
    line-height: 1.5;
    color: var(--nord-ember-bright);
    background: rgba(214, 168, 93, 0.08);
    border: 1px solid rgba(214, 168, 93, 0.25);
    border-radius: 10px;
    padding: 0.7rem 0.9rem;
  }
  @media (max-width: 820px) {
    .knowledge-grid {
      grid-template-columns: repeat(2, 1fr);
    }
  }
  @media (max-width: 520px) {
    .knowledge-grid {
      grid-template-columns: 1fr;
    }
  }
</style>
