<script>
  import { t } from "../../i18n.js";
  import StatusBadge from "./StatusBadge.svelte";

  let {
    mode = "real",
    powerSweep = null,
    applied = null,
    verification = null,
    showPlaceholders = false,
    onApplyPower,
  } = $props();
  let applyingKey = $state(null);

  const meta = [
    {
      key: "godforge",
      name: "Godforge",
      stance: "Performance first",
      summary: "Pushes the silicon toward its strongest sustainable profile.",
      outcomes: ["Highest sustainable performance", "Higher power consumption", "Higher thermal output"],
    },
    {
      key: "brokkrs",
      name: "Brokkr's Best",
      stance: "Balance first",
      summary: "Recommended for most users: strong performance with lower power and heat.",
      recommended: true,
      outcomes: ["Strong gaming performance", "Lower power draw", "Lower temperatures", "Lower fan noise"],
    },
    {
      key: "deep_calm",
      name: "Deep Calm",
      stance: "Efficiency first",
      summary: "Prioritizes lower power, heat and noise over peak numbers.",
      outcomes: ["Maximum efficiency", "Lowest power consumption", "Cooler and quieter operation"],
    },
  ];

  function powerProfile(m) {
    return powerSweep?.[m.key] ?? null;
  }

  function technical(m) {
    const pp = powerProfile(m);
    if (pp) return `${pp.clock_mhz} MHz target`;
    return "Awaiting forge data";
  }

  function secondary(m) {
    const pp = powerProfile(m);
    if (pp) return `${pp.power_w.toFixed(0)} W / ${pp.perf_per_watt.toFixed(1)} MHz/W`;
    return "Appears after the first completed Forge GPU run.";
  }

  function curveAnchor(point) {
    if (point?.vf_table_voltage_mv != null) return `Curve anchor: ${point.vf_table_voltage_mv} mV`;
    return null;
  }

  function measuredVoltage(point) {
    if (!point) return null;
    const avg = point.avg_measured_voltage_mv;
    const min = point.min_measured_voltage_mv;
    const max = point.max_measured_voltage_mv;
    if (avg != null && min != null && max != null) {
      return `Measured voltage under load: ${avg} / ${min} / ${max} mV`;
    }
    if (point.measured_voltage_mv != null) return `Measured voltage under load: ${point.measured_voltage_mv} mV`;
    return null;
  }

  function hasData(m) {
    return Boolean(powerProfile(m));
  }

  function normalize(s) {
    return String(s ?? "").toLowerCase().replace(/[^a-z0-9]/g, "");
  }

  function powerName(key) {
    if (key === "godforge") return "Godforge";
    if (key === "brokkrs") return "Brokkr's Best";
    if (key === "deep_calm") return "Deep Calm";
    return key;
  }

  function sameNumber(a, b) {
    return a != null && b != null && Number(a) === Number(b);
  }

  function voltageMatches(p) {
    if (!p || !applied?.core) return false;
    if (verification?.vf_table_voltage_mv != null && p.vf_table_voltage_mv != null) {
      return sameNumber(verification.vf_table_voltage_mv, p.vf_table_voltage_mv);
    }
    return sameNumber(applied.core.voltage_mv, p.voltage_mv);
  }

  function profileState(name, p) {
    const labelMatches = normalize(applied?.label) === normalize(name);
    const clockMatches = Boolean(applied?.core && p && sameNumber(applied.core.freq_mhz, p.clock_mhz));
    const numericMatches = Boolean(clockMatches && voltageMatches(p));
    const curveMismatch = Boolean(labelMatches && verification?.status === "live_mismatch");
    const active = Boolean(labelMatches && numericMatches && !curveMismatch);
    return {
      active,
      updated: Boolean(labelMatches && p && !numericMatches),
      curveMismatch,
      stale: Boolean(labelMatches && p && (!numericMatches || curveMismatch)),
    };
  }

  async function applyPowerCard(key, p) {
    const state = profileState(powerName(key), p);
    if (!p || state.active || applyingKey) return;
    applyingKey = key;
    try {
      await onApplyPower?.(key);
    } finally {
      applyingKey = null;
    }
  }

  async function applyProfile(m) {
    const state = profileState(m.name, powerProfile(m));
    if (!hasData(m) || state.active || applyingKey) return;
    applyingKey = m.key;
    try {
      await onApplyPower?.(m.key);
    } finally {
      applyingKey = null;
    }
  }
</script>

{#if mode === "power"}
  {#if !powerSweep?.running && (powerSweep?.godforge || powerSweep?.brokkrs)}
    <div class="profiles">
      {#each [["godforge", powerSweep.godforge], ["brokkrs", powerSweep.brokkrs]] as [key, p]}
        {@const state = profileState(powerName(key), p)}
        <div class={`profile profile-${key}`} class:active={state.active} class:stale={state.stale}>
          <div class="prof-name">{$t("forge.prof_" + key)}</div>
          {#if p}
            <div class="prof-val">{p.clock_mhz} MHz target</div>
            <div class="prof-sub">Optimized boost curve</div>
            {#if curveAnchor(p)}
              <div class="prof-sub">{curveAnchor(p)}</div>
            {/if}
            {#if measuredVoltage(p)}
              <div class="prof-sub">{measuredVoltage(p)}</div>
            {/if}
            <div class="prof-sub">{p.power_w.toFixed(0)} W / {p.perf_per_watt.toFixed(1)} MHz/W</div>
            <button
              class="btn small"
              class:go={!state.active}
              disabled={state.active || applyingKey === key}
              onclick={() => applyPowerCard(key, p)}
            >
              {#if applyingKey === key}
                Applying...
              {:else if state.active}
                Applied ✓
              {:else if state.updated}
                Apply Updated Profile
              {:else if state.curveMismatch}
                Reapply
              {:else}
                {$t("forge.apply")}
              {/if}
            </button>
          {:else}
            <div class="prof-sub">-</div>
          {/if}
        </div>
      {/each}
    </div>
  {/if}
{:else if powerSweep || showPlaceholders}
  <div class="profiles">
    {#each meta as item}
      {@const point = powerProfile(item)}
      {@const state = profileState(item.name, point)}
      <article class={`profile profile-${item.key}`} class:recommended={item.recommended} class:active={state.active} class:stale={state.stale}>
        <div class="profile-top">
          <div>
            <h4>{item.name}</h4>
            <span class="stance">{item.stance}</span>
          </div>
          <div class="profile-badges">
            {#if item.recommended}
              <StatusBadge label="Recommended" variant="recommended" symbol="check" compact />
            {/if}
            {#if state.active}
              <StatusBadge label="Active" variant="active" symbol="check" compact />
            {:else if state.updated}
              <StatusBadge label="Updated" variant="tempered" symbol="activity" compact />
            {/if}
            {#if state.curveMismatch}
              <StatusBadge label="Curve mismatch" variant="attention" symbol="attention" compact />
            {/if}
          </div>
        </div>
        <p class="desc">{item.summary}</p>
        <div class="expected">
          <span>Expected behavior</span>
          <ul>
            {#each item.outcomes as outcome}
              <li>{outcome}</li>
            {/each}
          </ul>
        </div>
        <div class="technical">
          <span>Technical</span>
          <strong>{technical(item)}</strong>
          {#if hasData(item)}
            <small>Optimized boost curve</small>
          {/if}
          {#if curveAnchor(point)}
            <small>{curveAnchor(point)}</small>
            <small>Not a hard voltage cap. Measured voltage can vary by workload.</small>
          {/if}
          {#if measuredVoltage(point)}
            <small>{measuredVoltage(point)}</small>
          {/if}
          <small>{secondary(item)}</small>
        </div>
        {#if hasData(item)}
          <button
            class="btn small"
            class:go={!state.active}
            disabled={state.active || applyingKey === item.key}
            onclick={() => applyProfile(item)}
          >
            {#if applyingKey === item.key}
              Applying...
            {:else if state.active}
              Applied ✓
            {:else if state.updated}
              Apply Updated Profile
            {:else if state.curveMismatch}
              Reapply
            {:else}
              {$t("forge.apply")}
            {/if}
          </button>
        {/if}
      </article>
    {/each}
  </div>
{/if}

<style>
  .profiles {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 0.85rem;
    margin-top: 0.75rem;
  }
  .profile {
    --profile-accent: var(--forge-steel);
    --profile-accent-soft: rgba(156, 170, 189, 0.1);
    position: relative;
    overflow: hidden;
    border: 1px solid var(--forge-line);
    border-radius: 8px;
    padding: 0.92rem 0.95rem;
    background:
      linear-gradient(180deg, var(--profile-accent-soft), rgba(8, 11, 16, 0.42)),
      var(--forge-panel-bg);
    box-shadow: var(--forge-panel-edge);
    display: flex;
    flex-direction: column;
    gap: 0.72rem;
    transition:
      border-color 0.15s ease,
      background 0.15s ease,
      box-shadow 0.15s ease;
  }
  .profile::before {
    content: "";
    position: absolute;
    inset: 0 0 auto;
    height: 2px;
    background: linear-gradient(90deg, transparent, var(--profile-accent), transparent);
    opacity: 0.72;
  }
  .profile-godforge {
    --profile-accent: var(--forge-copper);
    --profile-accent-soft: rgba(185, 117, 75, 0.11);
  }
  .profile-brokkrs {
    --profile-accent: var(--forge-gold);
    --profile-accent-soft: rgba(214, 168, 93, 0.12);
  }
  .profile-deep_calm {
    --profile-accent: var(--forge-blue);
    --profile-accent-soft: rgba(126, 173, 190, 0.09);
  }
  .profile.recommended {
    border-color: rgba(214, 168, 93, 0.44);
    box-shadow: var(--forge-shadow-active);
  }
  .profile.stale {
    border-color: rgba(214, 168, 93, 0.5);
    box-shadow:
      inset 0 0 0 1px rgba(214, 168, 93, 0.08),
      var(--forge-panel-edge);
  }
  .profile.active {
    border-color: rgba(157, 191, 145, 0.62);
    background:
      linear-gradient(180deg, rgba(157, 191, 145, 0.13), rgba(8, 11, 16, 0.44)),
      var(--forge-panel-bg);
    box-shadow:
      inset 0 0 0 1px rgba(157, 191, 145, 0.16),
      0 16px 34px rgba(0, 0, 0, 0.26);
  }
  .profile-top {
    display: flex;
    justify-content: space-between;
    gap: 0.6rem;
    align-items: flex-start;
  }
  .profile h4 {
    margin: 0;
    color: var(--profile-accent);
    font-size: 0.98rem;
    letter-spacing: 0.02em;
  }
  .stance,
  .expected span,
  .technical span,
  .technical small {
    color: var(--nord-dim);
    font-size: 0.72rem;
  }
  .profile-badges {
    display: flex;
    justify-content: flex-end;
    gap: 0.35rem;
    flex-wrap: wrap;
  }
  .prof-name {
    font-weight: 700;
    color: var(--profile-accent);
    font-size: 0.85rem;
    letter-spacing: 0.02em;
  }
  .prof-val {
    margin-top: 0.3rem;
    color: var(--text);
    font-variant-numeric: tabular-nums;
  }
  .prof-sub {
    color: var(--muted);
    font-size: 0.78rem;
    margin: 0.15rem 0 0.5rem;
    font-variant-numeric: tabular-nums;
  }
  .desc {
    margin: 0;
    font-size: 0.82rem;
    color: var(--muted);
    line-height: 1.45;
  }
  .expected {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }
  ul {
    margin: 0;
    padding-left: 1.02rem;
    color: var(--text);
    font-size: 0.82rem;
    line-height: 1.5;
  }
  li::marker {
    color: var(--profile-accent);
  }
  .technical {
    border-top: 1px solid var(--border);
    padding-top: 0.62rem;
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
    margin-top: auto;
  }
  .technical strong {
    color: var(--text);
    font-variant-numeric: tabular-nums;
    font-size: 0.86rem;
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
    transition:
      border-color 0.15s ease,
      color 0.15s ease,
      background 0.15s ease;
  }
  .btn.go {
    background: rgba(214, 168, 93, 0.12);
    color: var(--profile-accent);
    border-color: color-mix(in srgb, var(--profile-accent), transparent 50%);
  }
  .btn.small {
    padding: 0.35rem 0.8rem;
    font-size: 0.78rem;
    margin-top: 0.5rem;
  }
  .btn:disabled {
    cursor: default;
    opacity: 0.82;
  }
  .profile.active .btn {
    background: rgba(157, 191, 145, 0.16);
    border-color: rgba(157, 191, 145, 0.46);
    color: var(--forge-green);
  }
  @media (max-width: 640px) {
    .profiles {
      grid-template-columns: 1fr;
    }
  }
</style>
