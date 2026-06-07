<script>
  // Fixed domains so the curve is shown with headroom, like MSI Afterburner.
  const F_MIN = 500, F_MAX = 3500, F_GRID = 100, F_LABEL = 500;
  const V_MIN = 700, V_MAX = 1250, V_GRID = 25;

  let { points = [], overlay = null, height = 440 } = $props();

  const W = 960;
  const padL = 56, padR = 18, padT = 16, padB = 48;
  const CELL_W = 7, CELL_H = 3, CELL_GAP_X = 7, CELL_GAP_Y = 1;

  const H = $derived(height);
  const matrixRows = $derived(H >= 500 ? 9 : 7);
  const matrixHeight = $derived(matrixRows * CELL_H + (matrixRows - 1) * CELL_GAP_Y);
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
    const bandHeight = matrixHeight + 8;
    return { x, y, width, height: bandHeight, yTop: y - bandHeight / 2 };
  });

  const bifrostCells = $derived.by(() => {
    if (!bifrostBand) return [];
    const pitchX = CELL_W + CELL_GAP_X;
    const cols = Math.max(1, Math.floor((bifrostBand.width - 8) / pitchX));
    const gridWidth = cols * CELL_W + Math.max(0, cols - 1) * CELL_GAP_X;
    const startX = bifrostBand.x + Math.max(4, (bifrostBand.width - gridWidth) / 2);
    const startY = bifrostBand.yTop + (bifrostBand.height - matrixHeight) / 2;
    const center = (matrixRows - 1) / 2;
    const out = [];
    for (let col = 0; col < cols; col += 1) {
      for (let row = 0; row < matrixRows; row += 1) {
        const edgeDist = Math.abs(row - center) / Math.max(1, center);
        const phase = (col * 5 + row * 3) % 16;
        const accent = (col * 11 + row * 7) % 47 === 0;
        out.push({
          x: startX + col * pitchX,
          y: startY + row * (CELL_H + CELL_GAP_Y),
          row,
          col,
          phase,
          accent,
          bright: edgeDist < 0.22,
          mid: edgeDist < 0.58,
          edge: edgeDist > 0.78,
          delay: `-${(phase * 0.42).toFixed(2)}s`,
        });
      }
    }
    return out;
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
      <line class="band-boundary" x1={bifrostBand.x} y1={bifrostBand.yTop} x2={W - padR} y2={bifrostBand.yTop} />
      <line
        class="band-boundary"
        x1={bifrostBand.x}
        y1={bifrostBand.yTop + bifrostBand.height}
        x2={W - padR}
        y2={bifrostBand.yTop + bifrostBand.height}
      />
      <g class="band-matrix" aria-hidden="true">
        {#each bifrostCells as cell}
          <rect
            class="matrix-cell"
            class:bright={cell.bright}
            class:mid={cell.mid}
            class:edge={cell.edge}
            class:accent={cell.accent}
            x={cell.x}
            y={cell.y}
            width={CELL_W}
            height={CELL_H}
            rx="1"
            style={`--phase-delay: ${cell.delay}`}
          />
        {/each}
      </g>
      <line
        class="band-core"
        x1={bifrostBand.x}
        y1={bifrostBand.y}
        x2={W - padR}
        y2={bifrostBand.y}
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
  .band-boundary {
    stroke: rgba(214, 168, 93, 0.32);
    stroke-width: 1;
    stroke-linecap: round;
  }
  .band-matrix {
    opacity: 0.96;
  }
  .matrix-cell {
    --idle-opacity: 0.18;
    --glow-opacity: 0.34;
    --peak-opacity: 0.46;
    fill: #27313d;
    opacity: var(--idle-opacity);
    animation: bifrost-cell-flow 8.8s ease-in-out infinite;
    animation-delay: var(--phase-delay);
  }
  .matrix-cell.mid {
    --idle-opacity: 0.26;
    --glow-opacity: 0.46;
    --peak-opacity: 0.62;
    fill: #486878;
  }
  .matrix-cell.bright {
    --idle-opacity: 0.38;
    --glow-opacity: 0.64;
    --peak-opacity: 0.82;
    fill: #d6a85d;
  }
  .matrix-cell.edge {
    --idle-opacity: 0.1;
    --glow-opacity: 0.18;
    --peak-opacity: 0.28;
    fill: #202933;
  }
  .matrix-cell.accent {
    --idle-opacity: 0.3;
    --glow-opacity: 0.48;
    --peak-opacity: 0.62;
    fill: #79bdc4;
  }
  .matrix-cell.bright.accent {
    --idle-opacity: 0.34;
    --glow-opacity: 0.54;
    --peak-opacity: 0.7;
    fill: #a084d2;
  }
  .band-core {
    stroke: rgba(214, 168, 93, 0.72);
    stroke-width: 2.5;
    stroke-linecap: round;
    opacity: 0.72;
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
  @media (prefers-reduced-motion: reduce) {
    .pt,
    .matrix-cell {
      animation: none;
      transition: none;
    }
    .matrix-cell {
      opacity: var(--idle-opacity);
    }
    .matrix-cell.mid {
      opacity: var(--glow-opacity);
    }
    .matrix-cell.bright {
      opacity: var(--glow-opacity);
    }
    .matrix-cell.edge {
      opacity: var(--idle-opacity);
    }
  }
  @keyframes bifrost-cell-flow {
    0%,
    100% {
      opacity: var(--idle-opacity);
      filter: saturate(0.82);
    }
    34% {
      opacity: var(--glow-opacity);
      filter: saturate(1);
    }
    50% {
      opacity: var(--peak-opacity);
      filter: saturate(1.1);
    }
    66% {
      opacity: var(--glow-opacity);
      filter: saturate(0.92);
    }
  }
</style>
