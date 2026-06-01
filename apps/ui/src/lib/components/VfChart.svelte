<script>
  // Fixed domains so the curve is shown with headroom, like MSI Afterburner.
  const F_MIN = 500, F_MAX = 3500, F_GRID = 100, F_LABEL = 500;
  const V_MIN = 700, V_MAX = 1250, V_GRID = 25;

  let { points = [], plateau = null, height = 440 } = $props();

  const W = 960;
  const padL = 56, padR = 18, padT = 16, padB = 48;

  const H = $derived(height);
  const sx = (v) => padL + ((v - V_MIN) / (V_MAX - V_MIN)) * (W - padL - padR);
  const sy = (f) => H - padB - ((f - F_MIN) / (F_MAX - F_MIN)) * (H - padT - padB);

  const inDomain = (p) =>
    p.voltage_mv >= V_MIN && p.voltage_mv <= V_MAX && p.freq_mhz >= F_MIN && p.freq_mhz <= F_MAX;

  // Points within the visible domain, sorted by voltage.
  const pts = $derived([...points].filter(inDomain).sort((a, b) => a.voltage_mv - b.voltage_mv));

  // Is there a chosen undervolt limit (plateau) to lock the curve at?
  const hasPlateau = $derived(plateau && inDomain(plateau));

  // The APPLIED curve. With an undervolt limit we lock the voltage at the
  // plateau: the curve follows stock up to that voltage, then is FLAT at the
  // locked frequency for every higher voltage (it does NOT keep climbing — the
  // GPU is clamped). Without a limit it's the stock curve, extended flat at the
  // last read point just to reach the right edge.
  const path = $derived.by(() => {
    if (!pts.length) return "";
    const first = pts[0];
    let d = `M${sx(V_MIN).toFixed(1)},${sy(first.freq_mhz).toFixed(1)}`;
    if (hasPlateau) {
      const pv = plateau.voltage_mv, pf = plateau.freq_mhz;
      for (const p of pts) {
        if (p.voltage_mv < pv) d += ` L${sx(p.voltage_mv).toFixed(1)},${sy(p.freq_mhz).toFixed(1)}`;
      }
      d += ` L${sx(pv).toFixed(1)},${sy(pf).toFixed(1)}`;       // join the locked point
      d += ` L${sx(V_MAX).toFixed(1)},${sy(pf).toFixed(1)}`;    // flat after the limit
      return d;
    }
    const last = pts[pts.length - 1];
    for (const p of pts) d += ` L${sx(p.voltage_mv).toFixed(1)},${sy(p.freq_mhz).toFixed(1)}`;
    d += ` L${sx(V_MAX).toFixed(1)},${sy(last.freq_mhz).toFixed(1)}`;
    return d;
  });

  // Faint "stock continuation" above the limit — where the curve WOULD go
  // unclamped, shown dimmed so it's clear the limit flattened it (not lost it).
  const stockTail = $derived.by(() => {
    if (!hasPlateau || !pts.length) return "";
    const tail = pts.filter((p) => p.voltage_mv >= plateau.voltage_mv);
    if (!tail.length) return "";
    let d = `M${sx(plateau.voltage_mv).toFixed(1)},${sy(plateau.freq_mhz).toFixed(1)}`;
    for (const p of tail) d += ` L${sx(p.voltage_mv).toFixed(1)},${sy(p.freq_mhz).toFixed(1)}`;
    return d;
  });

  const vLines = $derived.by(() => {
    const out = [];
    for (let v = V_MIN; v <= V_MAX; v += V_GRID) out.push({ x: sx(v), v });
    return out;
  });
  const hLines = $derived.by(() => {
    const out = [];
    for (let f = F_MIN; f <= F_MAX; f += F_GRID) out.push({ y: sy(f), f, label: f % F_LABEL === 0 });
    return out;
  });
  const dot = $derived(
    plateau && inDomain(plateau) ? { cx: sx(plateau.voltage_mv), cy: sy(plateau.freq_mhz) } : null,
  );

  // MSI-style guide lines + axis value boxes for the hovered point.
  let hovered = $state(null);
</script>

<svg class="vfchart" viewBox={`0 0 ${W} ${H}`} role="img" aria-label="Voltage/Frequency curve">
  {#each hLines as l}
    <line class="grid" class:major={l.label} x1={padL} y1={l.y} x2={W - padR} y2={l.y} />
    {#if l.label}
      <text class="axis" x={padL - 8} y={l.y + 3} text-anchor="end">{l.f}</text>
    {/if}
  {/each}
  {#each vLines as l}
    <line class="grid major" x1={l.x} y1={padT} x2={l.x} y2={H - padB} />
    <text class="axis vlabel" x={l.x} y={H - padB + 16} text-anchor="middle">{l.v}</text>
  {/each}
  <text class="axis-title" x={padL} y={11}>MHz</text>
  <text class="axis-title" x={W - padR} y={H - 8} text-anchor="end">mV</text>

  {#if stockTail}
    <path class="stock-tail" d={stockTail} />
  {/if}
  {#if path}
    <path class="curve-line" d={path} />
  {/if}

  <!-- Guide lines + axis value boxes for the hovered point (MSI-style) -->
  {#if hovered}
    {@const hx = sx(hovered.voltage_mv)}
    {@const hy = sy(hovered.freq_mhz)}
    <line class="guide" x1={hx} y1={hy} x2={hx} y2={H - padB} />
    <line class="guide" x1={padL} y1={hy} x2={hx} y2={hy} />
    <rect class="axbox" x={hx - 22} y={H - padB + 2} width="44" height="15" rx="3" />
    <text class="axval" x={hx} y={H - padB + 13} text-anchor="middle">{hovered.voltage_mv}</text>
    <rect class="axbox" x={2} y={hy - 8} width="48" height="15" rx="3" />
    <text class="axval" x={26} y={hy + 3} text-anchor="middle">{hovered.freq_mhz}</text>
  {/if}

  <!-- Actual curve data points; hover shows the value + guides -->
  {#each pts as p}
    <circle
      class="pt"
      class:beyond={hasPlateau && p.voltage_mv > plateau.voltage_mv}
      cx={sx(p.voltage_mv)}
      cy={sy(p.freq_mhz)}
      r="2.6"
      role="presentation"
      onmouseenter={() => (hovered = p)}
      onmouseleave={() => (hovered = null)}
    >
      <title>{p.freq_mhz} MHz @ {p.voltage_mv} mV</title>
    </circle>
  {/each}

  {#if dot}
    <circle
      class="plateau-dot"
      cx={dot.cx}
      cy={dot.cy}
      r="5"
      role="presentation"
      onmouseenter={() => (hovered = plateau)}
      onmouseleave={() => (hovered = null)}
    >
      <title>Plateau: {plateau.freq_mhz} MHz @ {plateau.voltage_mv} mV</title>
    </circle>
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
  .vlabel {
    font-size: 9px;
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
  .stock-tail {
    fill: none;
    stroke: var(--nord-frost-dim);
    stroke-width: 1.5;
    stroke-dasharray: 4 4;
    opacity: 0.45;
  }
  .pt.beyond {
    fill: var(--nord-frost-dim);
    opacity: 0.4;
  }
  .pt {
    fill: var(--nord-frost-bright);
    stroke: #0a101c;
    stroke-width: 0.75;
    transition: r 0.1s ease;
    cursor: crosshair;
  }
  .pt:hover {
    r: 5;
    fill: var(--nord-ember-bright);
  }
  .plateau-dot {
    fill: var(--nord-ember-bright);
    stroke: #0a101c;
    stroke-width: 1.5;
  }
  .guide {
    stroke: var(--nord-frost-bright);
    stroke-width: 1;
    stroke-dasharray: 3 3;
    opacity: 0.7;
  }
  .axbox {
    fill: var(--nord-frost-bright);
  }
  .axval {
    fill: #0a101c;
    font-size: 10px;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
  }
</style>
