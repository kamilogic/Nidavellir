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
  const isUndervolt = $derived(Boolean(powerSweep?.is_undervolt));
  const profilesQualified = $derived(!isUndervolt || Boolean(powerSweep?.profiles_qualified));

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
    if (pp) return `${pp.target_clock_mhz ?? pp.clock_mhz} MHz target`;
    return "Awaiting forge data";
  }

  function secondary(m) {
    const pp = powerProfile(m);
    if (pp) return `${profilePower(pp).toFixed(0)} W peak / ${pp.perf_per_watt.toFixed(1)} MHz/W`;
    return "Appears after the first completed Forge GPU run.";
  }

  function profilePower(point) {
    const peak = Number(point?.max_power_w);
    return Number.isFinite(peak) && peak > 0 ? peak : Number(point?.power_w ?? 0);
  }

  function curveAnchor(point) {
    if (point?.vf_table_voltage_mv != null && point?.boundary_voltage_mv != null) {
      return `Apply VF bin: ${point.vf_table_voltage_mv} mV · learned boundary ${point.boundary_voltage_mv} mV · +${point.apply_margin_mv ?? 0} mV margin`;
    }
    if (point?.vf_table_voltage_mv != null) return `VF bin: ${point.vf_table_voltage_mv} mV`;
    return null;
  }

  function achievedClock(point) {
    if (point?.target_clock_mhz == null || point?.clock_mhz == null) return null;
    const p5 = point.p5_clock_mhz != null ? ` · p5 ${point.p5_clock_mhz} MHz` : "";
    return `Measured: ${point.clock_mhz} MHz${p5}`;
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

  function confidenceSummary(point) {
    if (!point) return null;
    const parts = [];
    if (point.confidence != null) {
      const confidence = Number(point.confidence);
      if (Number.isFinite(confidence)) parts.push(`Stability confidence ${confidence.toFixed(2)}`);
    }
    if (point.validation_count != null) {
      const validationCount = Number(point.validation_count);
      if (Number.isFinite(validationCount)) {
        parts.push(`${validationCount} ${validationCount === 1 ? "confirmation" : "confirmations"}`);
      }
    }
    return parts.length ? parts.join(" · ") : null;
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

  function sameProfilePoint(a, b) {
    if (!a || !b) return false;
    const aTarget = a.target_clock_mhz ?? a.clock_mhz;
    const bTarget = b.target_clock_mhz ?? b.clock_mhz;
    const aBin = a.vf_table_voltage_mv ?? a.voltage_mv;
    const bBin = b.vf_table_voltage_mv ?? b.voltage_mv;
    return sameNumber(aTarget, bTarget) && sameNumber(a.clock_mhz, b.clock_mhz) && sameNumber(aBin, bBin);
  }

  const collapseMessage = $derived.by(() => {
    if (powerSweep?.power_bound_collapse) {
      return "Brokkr's ≡ Godforge on this GPU — the measured frontier is power-limited, with no headroom above the efficiency point.";
    }
    if (sameProfilePoint(powerSweep?.godforge, powerSweep?.brokkrs)) {
      return "Brokkr's currently resolves to the same measured point as Godforge. Nidavellir will not manufacture a difference.";
    }
    return null;
  });

  function voltageMatches(p) {
    if (!p || !applied?.core) return false;
    if (verification?.vf_table_voltage_mv != null && p.vf_table_voltage_mv != null) {
      return sameNumber(verification.vf_table_voltage_mv, p.vf_table_voltage_mv);
    }
    return sameNumber(applied.core.voltage_mv, p.voltage_mv);
  }

  function profileState(name, p) {
    const labelMatches = normalize(applied?.label) === normalize(name);
    const profileClock = isUndervolt ? (p?.target_clock_mhz ?? p?.clock_mhz) : p?.clock_mhz;
    const clockMatches = Boolean(applied?.core && p && sameNumber(applied.core.freq_mhz, profileClock));
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
    if (!p || !profilesQualified || state.active || applyingKey) return;
    applyingKey = key;
    try {
      await onApplyPower?.(key);
    } finally {
      applyingKey = null;
    }
  }

  async function applyProfile(m) {
    const state = profileState(m.name, powerProfile(m));
    if (!hasData(m) || !profilesQualified || state.active || applyingKey) return;
    applyingKey = m.key;
    try {
      await onApplyPower?.(m.key);
    } finally {
      applyingKey = null;
    }
  }
</script>

{#if collapseMessage}
  <div class="collapse-note" role="status">
    <strong>Honest profile result</strong>
    <span>{collapseMessage}</span>
  </div>
{/if}

{#if mode === "power"}
  {#if !powerSweep?.running && (powerSweep?.godforge || powerSweep?.brokkrs || powerSweep?.deep_calm)}
    <div class="profiles">
      {#each [["godforge", powerSweep.godforge], ["brokkrs", powerSweep.brokkrs], ["deep_calm", powerSweep.deep_calm]] as [key, p]}
        {@const state = profileState(powerName(key), p)}
        <div class={`profile profile-${key}`} class:active={state.active} class:stale={state.stale}>
          <div class="prof-name">{$t("forge.prof_" + key)}</div>
          {#if p}
            <div class="prof-val">{p.target_clock_mhz ?? p.clock_mhz} MHz target</div>
            <div class="prof-sub">Optimized boost curve</div>
            {#if achievedClock(p)}
              <div class="prof-sub">{achievedClock(p)}</div>
            {/if}
            {#if curveAnchor(p)}
              <div class="prof-sub">{curveAnchor(p)}</div>
            {/if}
            {#if measuredVoltage(p)}
              <div class="prof-sub">{measuredVoltage(p)}</div>
            {/if}
            <div class="prof-sub power-reading">{profilePower(p).toFixed(0)} W peak / {p.perf_per_watt.toFixed(1)} MHz/W</div>
            <div class="prof-sub power-note">Measured saturation peak. Not a hard power limit; other workloads can vary.</div>
            {#if confidenceSummary(p)}
              <div class="prof-sub confidence">{confidenceSummary(p)}</div>
            {/if}
            <button
              class="btn small"
              class:go={!state.active}
              disabled={!profilesQualified || state.active || applyingKey === key}
              onclick={() => applyPowerCard(key, p)}
            >
              {#if !profilesQualified}
                Run Standard to qualify
              {:else if applyingKey === key}
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
            {:else if isUndervolt && point && profilesQualified}
              <StatusBadge label="Qualified" variant="forged" symbol="knowledge" compact />
            {:else if isUndervolt && point}
              <StatusBadge label="Provisional" variant="tempered" symbol="activity" compact />
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
          {#if achievedClock(point)}
            <small>{achievedClock(point)}</small>
          {/if}
          {#if curveAnchor(point)}
            <small>{curveAnchor(point)}</small>
            <small>Not a hard voltage cap. Measured voltage can vary by workload.</small>
          {/if}
          {#if measuredVoltage(point)}
            <small>{measuredVoltage(point)}</small>
          {/if}
          <small class="power-reading">{secondary(item)}</small>
          {#if hasData(item)}
            <small class="power-note">Measured saturation peak. Not a hard power limit; other workloads can vary.</small>
          {/if}
          {#if confidenceSummary(point)}
            <small class="confidence">{confidenceSummary(point)}</small>
          {/if}
        </div>
        {#if hasData(item)}
          <button
            class="btn small"
            class:go={!state.active}
            disabled={!profilesQualified || state.active || applyingKey === item.key}
            onclick={() => applyProfile(item)}
          >
            {#if !profilesQualified}
              Run Standard to qualify
            {:else if applyingKey === item.key}
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
  .collapse-note {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr);
    gap: 0.45rem 0.7rem;
    align-items: baseline;
    margin-top: 0.75rem;
    border-radius: 10px;
    padding: 0.68rem 0.78rem;
    background: rgba(214, 168, 93, 0.08);
    box-shadow:
      0 0 0 1px rgba(214, 168, 93, 0.24),
      0 8px 22px rgba(0, 0, 0, 0.14);
    color: var(--muted);
    font-size: 0.78rem;
    line-height: 1.45;
  }
  .collapse-note strong {
    color: var(--forge-gold);
    font-size: 0.68rem;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }
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
  .confidence {
    color: var(--forge-green) !important;
    font-variant-numeric: tabular-nums;
  }
  .power-reading {
    font-variant-numeric: tabular-nums;
  }
  .power-note {
    text-wrap: pretty;
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
    .collapse-note {
      grid-template-columns: 1fr;
    }
    .profiles {
      grid-template-columns: 1fr;
    }
  }
</style>
