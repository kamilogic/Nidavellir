<script>
  // Fixed domains so the curve is shown with headroom, like MSI Afterburner.
  const F_MIN = 500, F_MAX = 3500, F_GRID = 100, F_LABEL = 500;
  const V_MIN = 700, V_MAX = 1250, V_GRID = 25;

  let { points = [], overlay = null, height = 440 } = $props();

  const W = 960;
  const padL = 56, padR = 18, padT = 16, padB = 48;
  const uid = Math.random().toString(36).slice(2);
  const bandGradientId = `bifrost-band-gradient-${uid}`;
  const pixelPatternId = `bifrost-pixels-${uid}`;

  const H = $derived(height);
  const sx = (v) => padL + ((v - V_MIN) / (V_MAX - V_MIN)) * (W - padL - padR);
  const sy = (f) => H - padB - ((f - F_MIN) / (F_MAX - F_MIN)) * (H - padT - padB);

  const inDomain = (p) =>
    p.voltage_mv >= V_MIN && p.voltage_mv <= V_MAX && p.freq_mhz >= F_MIN && p.freq_mhz <= F_MAX;

  const pts = $derived([...points].filter(inDomain).sort((a, b) => a.voltage_mv - b.voltage_mv));

  const targetMhz = $derived.by(() => {
    const f = Number(overlay?.targetMhz);
    return Number.isFinite(f) && f >= F_MIN && f <= F_MAX ? f : null;
  });

  const anchorMv = $derived.by(() => {
    const v = Number(overlay?.anchorMv);
    return overlay?.showBand && overlay?.anchorPrecise && Number.isFinite(v) && v >= V_MIN && v <= V_MAX
      ? v
      : null;
  });

  const baselinePath = $derived.by(() => {
    if (!pts.length) return "";
    const first = pts[0];
    let d = `M${sx(first.voltage_mv).toFixed(1)},${sy(first.freq_mhz).toFixed(1)}`;
    for (const p of pts.slice(1)) d += ` L${sx(p.voltage_mv).toFixed(1)},${sy(p.freq_mhz).toFixed(1)}`;
    return d;
  });

  const targetLine = $derived(targetMhz == null ? null : { y: sy(targetMhz) });

  const bifrostBand = $derived.by(() => {
    if (targetMhz == null || anchorMv == null) return null;
    const x = sx(anchorMv);
    const width = W - padR - x;
    if (width < 8) return null;
    const y = sy(targetMhz);
    return { x, y, width, height: 24, yTop: y - 12 };
  });

  const anchorMarker = $derived.by(() => {
    if (targetMhz == null || anchorMv == null) return null;
    return { x: sx(anchorMv), y: sy(targetMhz) };
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

  let hovered = $state(null);
</script>

<svg class="vfchart" viewBox={`0 0 ${W} ${H}`} role="img" aria-label="Voltage/Frequency curve">
  <defs>
    <linearGradient id={bandGradientId} x1="0" y1="0" x2="1" y2="0">
      <stop offset="0%" stop-color="#7eadbe" stop-opacity="0.26" />
      <stop offset="42%" stop-color="#d6a85d" stop-opacity="0.7" />
      <stop offset="72%" stop-color="#a084d2" stop-opacity="0.28" />
      <stop offset="100%" stop-color="#7eadbe" stop-opacity="0.18" />
    </linearGradient>
    <pattern id={pixelPatternId} width="22" height="12" patternUnits="userSpaceOnUse">
      <rect x="1" y="2" width="3" height="3" fill="#d6a85d" opacity="0.62" />
      <rect x="8" y="7" width="2" height="2" fill="#7eadbe" opacity="0.5" />
      <rect x="14" y="3" width="4" height="2" fill="#b9754b" opacity="0.42" />
      <rect x="19" y="8" width="2" height="2" fill="#a084d2" opacity="0.34" />
    </pattern>
  </defs>

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
  <text class="axis-title" x={W - padR} y={H - 8} text-anchor="end">Curve mV</text>

  {#if baselinePath}
    <path class="baseline-curve" d={baselinePath} />
  {/if}

  {#if targetLine}
    <line class="target-line" x1={padL} y1={targetLine.y} x2={W - padR} y2={targetLine.y}>
      <title>{targetMhz} MHz target</title>
    </line>
  {/if}

  {#if bifrostBand}
    <g class="bifrost-band">
      <rect
        class="band-base"
        x={bifrostBand.x}
        y={bifrostBand.yTop}
        width={bifrostBand.width}
        height={bifrostBand.height}
        rx="4"
      />
      <rect
        class="band-pixels"
        x={bifrostBand.x}
        y={bifrostBand.yTop}
        width={bifrostBand.width}
        height={bifrostBand.height}
        rx="4"
        fill={`url(#${pixelPatternId})`}
      />
      <line
        class="band-core"
        x1={bifrostBand.x}
        y1={bifrostBand.y}
        x2={W - padR}
        y2={bifrostBand.y}
        stroke={`url(#${bandGradientId})`}
      />
      <title>Expected operating range from curve anchor across higher curve bins. Not a hard voltage cap.</title>
    </g>
  {/if}

  {#if anchorMarker}
    <line class="anchor-line" x1={anchorMarker.x} y1={padT} x2={anchorMarker.x} y2={H - padB} />
    <circle
      class="anchor-dot"
      cx={anchorMarker.x}
      cy={anchorMarker.y}
      r="5"
      role="presentation"
      onmouseenter={() => (hovered = { voltage_mv: anchorMv, freq_mhz: targetMhz })}
      onmouseleave={() => (hovered = null)}
    >
      <title>{targetMhz} MHz target - V/F bin: {anchorMv} mV - not a hard voltage cap</title>
    </circle>
  {/if}

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

  {#each pts as p}
    <circle
      class="pt"
      class:beyond={anchorMv != null && p.voltage_mv > anchorMv}
      cx={sx(p.voltage_mv)}
      cy={sy(p.freq_mhz)}
      r="2.6"
      role="presentation"
      onmouseenter={() => (hovered = p)}
      onmouseleave={() => (hovered = null)}
    >
      <title>{p.freq_mhz} MHz table point - V/F bin: {p.voltage_mv} mV</title>
    </circle>
  {/each}
</svg>

<style>
  .vfchart {
    width: 100%;
    height: auto;
    display: block;
    background:
      linear-gradient(180deg, rgba(9, 12, 17, 0.76), rgba(6, 8, 12, 0.92)),
      rgba(10, 16, 28, 0.55);
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
  .baseline-curve {
    fill: none;
    stroke: rgba(126, 173, 190, 0.5);
    stroke-width: 1.7;
    stroke-linejoin: round;
    stroke-linecap: round;
  }
  .target-line {
    stroke: rgba(214, 168, 93, 0.55);
    stroke-width: 1.2;
    stroke-dasharray: 8 7;
  }
  .bifrost-band {
    opacity: 0.94;
  }
  .band-base {
    fill: rgba(12, 14, 18, 0.78);
    stroke: rgba(214, 168, 93, 0.16);
    stroke-width: 1;
  }
  .band-pixels {
    opacity: 0.58;
  }
  .band-core {
    stroke-width: 2.5;
    stroke-linecap: round;
  }
  .anchor-line {
    stroke: rgba(214, 168, 93, 0.55);
    stroke-width: 1;
    stroke-dasharray: 3 5;
  }
  .anchor-dot {
    fill: var(--forge-gold);
    stroke: #0a101c;
    stroke-width: 1.5;
  }
  .pt.beyond {
    fill: var(--nord-frost-dim);
    opacity: 0.42;
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
  @media (prefers-reduced-motion: no-preference) {
    .band-pixels {
      animation: bifrost-breathe 14s ease-in-out infinite;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .pt,
    .band-pixels {
      animation: none;
      transition: none;
    }
  }
  @keyframes bifrost-breathe {
    0%,
    100% {
      opacity: 0.48;
    }
    50% {
      opacity: 0.68;
    }
  }
</style>
