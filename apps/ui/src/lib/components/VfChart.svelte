<script>
  // Fixed domains so the curve is shown with headroom, like MSI Afterburner.
  const F_MIN = 500, F_MAX = 3500, F_GRID = 100, F_LABEL = 500;
  const V_MIN = 700, V_MAX = 1250, V_GRID = 25, V_LABEL = 100;

  let { points = [], plateau = null, height = 440 } = $props();

  const W = 960;
  const padL = 56, padR = 18, padT = 16, padB = 36;

  const H = $derived(height);
  const sx = (v) => padL + ((v - V_MIN) / (V_MAX - V_MIN)) * (W - padL - padR);
  const sy = (f) => H - padB - ((f - F_MIN) / (F_MAX - F_MIN)) * (H - padT - padB);

  const inDomain = (p) => p.voltage_mv >= V_MIN && p.voltage_mv <= V_MAX && p.freq_mhz >= F_MIN && p.freq_mhz <= F_MAX;

  const path = $derived(
    points
      .filter(inDomain)
      .map((p, i) => `${i ? "L" : "M"}${sx(p.voltage_mv).toFixed(1)},${sy(p.freq_mhz).toFixed(1)}`)
      .join(" "),
  );

  const vLines = $derived.by(() => {
    const out = [];
    for (let v = V_MIN; v <= V_MAX; v += V_GRID) out.push({ x: sx(v), v, label: v % V_LABEL === 0 });
    return out;
  });
  const hLines = $derived.by(() => {
    const out = [];
    for (let f = F_MIN; f <= F_MAX; f += F_GRID) out.push({ y: sy(f), f, label: f % F_LABEL === 0 });
    return out;
  });
  const dot = $derived(plateau && inDomain(plateau) ? { cx: sx(plateau.voltage_mv), cy: sy(plateau.freq_mhz) } : null);
</script>

<svg class="vfchart" viewBox={`0 0 ${W} ${H}`} role="img" aria-label="Voltage/Frequency curve">
  {#each hLines as l}
    <line class="grid" class:major={l.label} x1={padL} y1={l.y} x2={W - padR} y2={l.y} />
    {#if l.label}
      <text class="axis" x={padL - 8} y={l.y + 3} text-anchor="end">{l.f}</text>
    {/if}
  {/each}
  {#each vLines as l}
    <line class="grid" class:major={l.label} x1={l.x} y1={padT} x2={l.x} y2={H - padB} />
    {#if l.label}
      <text class="axis" x={l.x} y={H - 12} text-anchor="middle">{l.v}</text>
    {/if}
  {/each}
  <text class="axis-title" x={padL} y={11}>MHz</text>
  <text class="axis-title" x={W - padR} y={H - 12} text-anchor="end">mV</text>
  {#if path}
    <path class="curve-line" d={path} />
  {/if}
  {#if dot}
    <circle class="plateau-dot" cx={dot.cx} cy={dot.cy} r="5" />
  {/if}
</svg>

<style>
  .vfchart {
    width: 100%;
    height: auto;
    display: block;
    background: rgba(10, 16, 28, 0.55);
    border: 1px solid var(--nord-border-card);
    border-radius: 10px;
  }
  .grid {
    stroke: rgba(136, 192, 208, 0.07);
    stroke-width: 1;
  }
  .grid.major {
    stroke: rgba(136, 192, 208, 0.16);
  }
  .axis {
    fill: var(--nord-dim);
    font-size: 11px;
    font-variant-numeric: tabular-nums;
  }
  .axis-title {
    fill: var(--nord-mist);
    font-size: 11px;
    font-weight: 700;
  }
  .curve-line {
    fill: none;
    stroke: var(--nord-aurora);
    stroke-width: 2.5;
    stroke-linejoin: round;
    stroke-linecap: round;
  }
  .plateau-dot {
    fill: var(--nord-ember-bright);
    stroke: #0a101c;
    stroke-width: 1.5;
  }
</style>
