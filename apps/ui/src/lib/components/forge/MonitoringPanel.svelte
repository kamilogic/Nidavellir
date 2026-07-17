<script>
  let { gpu = null, sparks = null, live = false } = $props();

  function fixed(value, digits = 0) {
    const n = Number(value);
    return Number.isFinite(n) ? n.toFixed(digits) : null;
  }

  // Bar height as a percent of the buffer's max, per the plan's simple CSS sparkline.
  function barHeight(v, arr) {
    const max = Math.max(...arr.map(Number).filter(Number.isFinite), 1);
    const n = Number(v);
    if (!Number.isFinite(n) || max <= 0) return 6;
    return Math.min(100, Math.max(6, Math.round((n / max) * 100)));
  }

  const tiles = $derived([
    { key: "core", label: "GPU Clock", accent: "gold", value: gpu?.core_clock_mhz, unit: "MHz", digits: 0, bars: sparks?.core ?? [] },
    { key: "mem", label: "VRAM Speed", accent: "blue", value: gpu?.memory_clock_mhz, unit: "MHz", digits: 0, bars: sparks?.mem ?? [], secondary: gpu?.vram_total_mb != null ? `${(Number(gpu.vram_total_mb) / 1024).toFixed(1)} GB total` : "Capacity unavailable" },
    { key: "temp", label: "Temperature", accent: "red", value: gpu?.temperature_c, unit: "°C", digits: 0, bars: sparks?.temp ?? [] },
    { key: "power", label: "Power", accent: "copper", value: gpu?.power_w, unit: "W", digits: 0, bars: sparks?.power ?? [] },
    { key: "voltage", label: "Core Voltage", accent: "copper", value: gpu?.voltage_mv, unit: "mV", digits: 0, bars: sparks?.voltage ?? [], secondary: gpu?.voltage_mv == null ? "Sensor not exposed" : null },
    { key: "fan", label: "Fan Speed", accent: "teal", value: gpu?.fan_speed_pct, unit: "%", digits: 0, bars: sparks?.fan ?? [], secondary: gpu?.fan_speed_pct == null ? "Sensor not exposed" : "Average duty" },
    { key: "usage", label: "GPU Usage", accent: "green", value: gpu?.utilization_pct, unit: "%", digits: 1, bars: sparks?.usage ?? [] },
  ]);
</script>

<section class="mon-panel">
  <div class="mon-head">
    <span class="mon-kicker">Monitoramento em tempo real</span>
    {#if live}<span class="live-pill">live</span>{/if}
  </div>
  <div class="mon-grid">
    {#each tiles as tile}
      <div class={`mon-tile ${tile.accent}`}>
        <div class="tile-head">
          <span class="tile-dot" aria-hidden="true"></span>
          <span class="tile-label">{tile.label}</span>
        </div>
        <div class="tile-value">
          {#if fixed(tile.value, tile.digits) == null}
            <strong class="na">Not exposed</strong>
          {:else}
            <strong>{fixed(tile.value, tile.digits)}</strong><span class="unit">{tile.unit}</span>
          {/if}
        </div>
        {#if tile.secondary}<small class="tile-secondary">{tile.secondary}</small>{/if}
        <div class="spark" aria-hidden="true">
          {#each tile.bars as v}
            <span class="bar" style={`height:${barHeight(v, tile.bars)}%`}></span>
          {/each}
        </div>
      </div>
    {/each}
  </div>
</section>

<style>
  .mon-panel {
    display: flex;
    flex-direction: column;
    gap: 0.9rem;
    background: var(--forge-panel);
    border-radius: 12px;
    padding: 1rem 1.15rem;
  }
  .mon-head {
    display: flex;
    align-items: center;
    gap: 0.6rem;
  }
  .mon-kicker {
    font-size: 0.7rem;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--forge-muted);
  }
  .live-pill {
    font-size: 0.56rem;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--forge-dim);
    background: var(--forge-panel-raised);
    border-radius: 999px;
    padding: 0.14rem 0.5rem;
  }
  .mon-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(145px, 1fr));
    gap: 0.6rem;
  }
  .mon-tile {
    --tile-accent: var(--forge-muted);
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    min-width: 0;
    background: var(--forge-panel-raised);
    border-radius: 8px;
    padding: 0.65rem 0.72rem;
  }
  .mon-tile.gold {
    --tile-accent: var(--forge-gold);
  }
  .mon-tile.blue {
    --tile-accent: var(--forge-blue);
  }
  .mon-tile.red {
    --tile-accent: var(--forge-red);
  }
  .mon-tile.copper {
    --tile-accent: var(--forge-copper);
  }
  .mon-tile.teal {
    --tile-accent: var(--forge-teal);
  }
  .mon-tile.green {
    --tile-accent: var(--forge-green);
  }
  .tile-head {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    min-width: 0;
  }
  .tile-dot {
    width: 0.42rem;
    height: 0.42rem;
    border-radius: 999px;
    background: var(--tile-accent);
    flex-shrink: 0;
  }
  .tile-label {
    font-size: 0.56rem;
    font-weight: 600;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: var(--forge-dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .tile-value {
    display: flex;
    align-items: baseline;
    gap: 0.18rem;
  }
  .tile-value strong {
    color: var(--forge-text);
    font-size: 1.1rem;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
    line-height: 1;
  }
  .tile-value strong.na {
    color: var(--forge-dim);
    font-size: 0.95rem;
  }
  .tile-secondary {
    min-height: 1rem;
    color: var(--forge-dim);
    font-size: 0.6rem;
    line-height: 1.25;
    overflow-wrap: anywhere;
  }
  .tile-value .unit {
    font-size: 0.62rem;
    font-weight: 600;
    color: var(--forge-muted);
  }
  .spark {
    display: flex;
    align-items: flex-end;
    gap: 2px;
    height: 1.5rem;
    min-height: 1.5rem;
  }
  .bar {
    flex: 1;
    min-width: 2px;
    border-radius: 2px;
    background: var(--tile-accent);
    opacity: 0.8;
  }
  @media (max-width: 900px) {
    .mon-grid {
      grid-template-columns: repeat(3, minmax(0, 1fr));
    }
  }
  @media (max-width: 520px) {
    .mon-grid {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }
  }
</style>
