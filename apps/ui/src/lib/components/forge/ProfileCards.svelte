<script>
  import { t } from "../../i18n.js";
  import StatusBadge from "./StatusBadge.svelte";

  let {
    mode = "real",
    realProfiles = null,
    powerSweep = null,
    applied = null,
    showPlaceholders = false,
    onApplyCore,
    onApplyPower,
  } = $props();
  let applyingKey = $state(null);

  const meta = [
    {
      key: "godforge",
      realKey: "godforge",
      applyIndex: 0,
      name: "Godforge",
      stance: "Performance first",
      summary: "Pushes the silicon toward its strongest sustainable profile.",
      outcomes: ["Highest sustainable performance", "Higher power consumption", "Higher thermal output"],
    },
    {
      key: "brokkrs",
      realKey: "brokkrs_best",
      applyIndex: 1,
      name: "Brokkr's Best",
      stance: "Balance first",
      summary: "Recommended for most users: strong performance with lower power and heat.",
      recommended: true,
      outcomes: ["Strong gaming performance", "Lower power draw", "Lower temperatures", "Lower fan noise"],
    },
    {
      key: "deep_calm",
      realKey: "deep_calm",
      applyIndex: 2,
      name: "Deep Calm",
      stance: "Efficiency first",
      summary: "Prioritizes lower power, heat and noise over peak numbers.",
      outcomes: ["Maximum efficiency", "Lowest power consumption", "Cooler and quieter operation"],
    },
  ];

  function realProfile(m) {
    return realProfiles?.[m.realKey] ?? null;
  }

  function powerProfile(m) {
    return powerSweep?.[m.key] ?? null;
  }

  function technical(m) {
    const rp = realProfile(m);
    if (rp?.point) return `${rp.point.freq_mhz} MHz @ ${rp.point.voltage_mv} mV`;
    const pp = powerProfile(m);
    if (pp) return `${pp.clock_mhz} MHz @ ${pp.voltage_mv} mV`;
    return "Not forged yet";
  }

  function secondary(m) {
    const pp = powerProfile(m);
    if (pp) return `${pp.power_w.toFixed(0)} W / ${pp.perf_per_watt.toFixed(1)} MHz/W`;
    return "Technical values appear after a completed forge run.";
  }

  function hasData(m) {
    return Boolean(realProfile(m) || powerProfile(m));
  }

  function normalize(s) {
    return String(s ?? "").toLowerCase().replace(/[^a-z0-9]/g, "");
  }

  function pointMatches(m) {
    if (!applied?.core) return false;
    const rp = realProfile(m)?.point;
    const pp = powerProfile(m);
    const freq = rp?.freq_mhz ?? pp?.clock_mhz;
    const voltage = rp?.voltage_mv ?? pp?.voltage_mv;
    return Boolean(freq && voltage && applied.core.freq_mhz === freq && applied.core.voltage_mv === voltage);
  }

  function isApplied(m) {
    return normalize(applied?.label) === normalize(m.name) || pointMatches(m);
  }

  function powerName(key) {
    if (key === "godforge") return "Godforge";
    if (key === "brokkrs") return "Brokkr's Best";
    if (key === "deep_calm") return "Deep Calm";
    return key;
  }

  function isPowerApplied(key, p) {
    if (!p) return false;
    return normalize(applied?.label) === normalize(powerName(key)) ||
      Boolean(applied?.core && applied.core.freq_mhz === p.clock_mhz && applied.core.voltage_mv === p.voltage_mv);
  }

  async function applyPowerCard(key, p) {
    if (!p || isPowerApplied(key, p) || applyingKey) return;
    applyingKey = key;
    try {
      await onApplyPower?.(key);
    } finally {
      applyingKey = null;
    }
  }

  async function applyProfile(m) {
    if (!hasData(m) || isApplied(m) || applyingKey) return;
    applyingKey = m.key;
    try {
      const result = realProfile(m) ? onApplyCore?.(m.applyIndex) : onApplyPower?.(m.key);
      await result;
    } finally {
      applyingKey = null;
    }
  }
</script>

{#if mode === "power"}
  {#if !powerSweep?.running && (powerSweep?.godforge || powerSweep?.brokkrs)}
    <div class="profiles">
      {#each [["godforge", powerSweep.godforge], ["brokkrs", powerSweep.brokkrs]] as [key, p]}
        {@const active = isPowerApplied(key, p)}
        <div class={`profile profile-${key}`} class:active>
          <div class="prof-name">{$t("forge.prof_" + key)}</div>
          {#if p}
            <div class="prof-val">{p.clock_mhz} MHz @ {p.voltage_mv} mV</div>
            <div class="prof-sub">{p.power_w.toFixed(0)} W / {p.perf_per_watt.toFixed(1)} MHz/W</div>
            <button
              class="btn small"
              class:go={!active}
              disabled={active || applyingKey === key}
              onclick={() => applyPowerCard(key, p)}
            >
              {#if active}
                Applied ✓
              {:else if applyingKey === key}
                Applying...
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
{:else if realProfiles || powerSweep || showPlaceholders}
  <div class="profiles">
    {#each meta as item}
      {@const active = isApplied(item)}
        <article class={`profile profile-${item.key}`} class:recommended={item.recommended} class:active>
          <div class="profile-top">
            <div>
              <h4>{item.name}</h4>
              <span class="stance">{item.stance}</span>
            </div>
            <div class="profile-badges">
              {#if item.recommended}
                <StatusBadge label="Recommended" variant="recommended" compact />
              {/if}
              {#if active}
                <StatusBadge label="Active" variant="active" compact />
              {/if}
            </div>
          </div>
        <p class="desc">{realProfile(item)?.description ?? item.summary}</p>
        <div class="expected">
          <span>Expected Result</span>
          <ul>
            {#each item.outcomes as outcome}
              <li>{outcome}</li>
            {/each}
          </ul>
        </div>
        <div class="technical">
          <span>Technical</span>
          <strong>{technical(item)}</strong>
          {#if !realProfile(item)}
            <small>{secondary(item)}</small>
          {/if}
        </div>
        {#if hasData(item)}
          <button
            class="btn small"
            class:go={!active}
            disabled={active || applyingKey === item.key}
            onclick={() => applyProfile(item)}
          >
            {#if active}
              Applied ✓
            {:else if applyingKey === item.key}
              Applying...
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
