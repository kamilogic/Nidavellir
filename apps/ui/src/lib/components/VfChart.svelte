<script>
  // Fixed domains so the curve is shown with headroom, like MSI Afterburner.
  const F_MIN = 500, F_MAX = 3500, F_GRID = 100, F_LABEL = 500;
  const V_MIN = 700, V_MAX = 1250, V_GRID = 25;

  let { points = [], overlay = null, height = 440 } = $props();

  const W = 960;
  const padL = 56, padR = 18, padT = 16, padB = 48;
  const CELL_W = 3.8, CELL_H = 2.4, CELL_GAP_X = 4.8, CELL_GAP_Y = 1.2, MAX_MATRIX_COLS = 76;

  const H = $derived(height);
  const matrixRows = $derived(H >= 500 ? 13 : 11);
  const matrixHeight = $derived(matrixRows * CELL_H + (matrixRows - 1) * CELL_GAP_Y);
  const sx = (v) => padL + ((v - V_MIN) / (V_MAX - V_MIN)) * (W - padL - padR);
  const sy = (f) => H - padB - ((f - F_MIN) / (F_MAX - F_MIN)) * (H - padT - padB);
  const voltageAtX = (x) => V_MIN + ((x - padL) / (W - padL - padR)) * (V_MAX - V_MIN);
  const clamp = (v, min = 0, max = 1) => Math.max(min, Math.min(max, v));
  const smoothstep = (v) => {
    const t = clamp(v);
    return t * t * (3 - 2 * t);
  };
  const cellHash = (col, row, salt = 0) => {
    const n = Math.sin((col + 1) * 127.1 + (row + 1) * 311.7 + (salt + 1) * 74.7) * 43758.5453;
    return n - Math.floor(n);
  };

  const inDomain = (p) =>
    p.voltage_mv >= V_MIN && p.voltage_mv <= V_MAX && p.freq_mhz >= F_MIN && p.freq_mhz <= F_MAX;

  const pts = $derived([...points].filter(inDomain).sort((a, b) => a.voltage_mv - b.voltage_mv));

  const targetMhz = $derived.by(() => {
    const f = Number(overlay?.targetMhz);
    return Number.isFinite(f) && f >= F_MIN && f <= F_MAX ? f : null;
  });

  const anchorMv = $derived.by(() => {
    const v = Number(overlay?.anchorMv);
    return Number.isFinite(v) && v >= V_MIN && v <= V_MAX ? v : null;
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
    const startMv = pts[0]?.voltage_mv ?? V_MIN;
    const x = sx(startMv);
    const width = W - padR - x;
    if (width < 8) return null;
    return { x, width, startMv };
  });

  function curveFreqAt(voltage) {
    if (!pts.length) return targetMhz;
    if (voltage <= pts[0].voltage_mv) return pts[0].freq_mhz;
    for (let i = 1; i < pts.length; i += 1) {
      const prev = pts[i - 1];
      const next = pts[i];
      if (voltage <= next.voltage_mv) {
        const span = Math.max(1, next.voltage_mv - prev.voltage_mv);
        const t = clamp((voltage - prev.voltage_mv) / span);
        return prev.freq_mhz + (next.freq_mhz - prev.freq_mhz) * t;
      }
    }
    return pts[pts.length - 1].freq_mhz;
  }

  function cellPalette({ edgeDist, progress, sectionEnergy, accentCell, goldCell, warmCell }) {
    const rowEnergy = Math.max(0, 1 - Math.pow(edgeDist, 1.75));
    const startFade = smoothstep(progress / 0.08);
    const endFade = 1 - smoothstep((progress - 0.88) / 0.12) * 0.34;
    const energy = clamp(rowEnergy * Math.max(0.34, startFade) * endFade * sectionEnergy);
    const edge = edgeDist > 0.78;
    const core = edgeDist < 0.22;
    const idle = edge ? 0.035 + energy * 0.06 : 0.07 + energy * 0.12 + (core ? 0.035 : 0);
    const glow = edge ? 0.12 + energy * 0.18 : 0.22 + energy * 0.36 + (core ? 0.06 : 0);
    const peak = edge ? 0.18 + energy * 0.26 : 0.38 + energy * 0.5 + (core ? 0.08 : 0);

    if (accentCell) {
      return {
        rest: "#253344",
        glow: "#5b96a8",
        peak: "#a084d2",
        idle: idle + 0.015,
        glowOpacity: glow + 0.04,
        peakOpacity: Math.min(0.72, peak + 0.06),
      };
    }
    if (goldCell || warmCell) {
      return {
        rest: "#523d2a",
        glow: "#b88548",
        peak: "#f0c979",
        idle: idle + (warmCell ? 0.07 : 0.025),
        glowOpacity: glow + (warmCell ? 0.11 : 0.045),
        peakOpacity: Math.min(0.88, peak + (warmCell ? 0.15 : 0.07)),
      };
    }
    return {
      rest: "#202b36",
      glow: "#486878",
      peak: "#7eadbe",
      idle,
      glowOpacity: glow,
      peakOpacity: Math.min(0.7, peak),
    };
  }

  const bifrostMatrix = $derived.by(() => {
    if (!bifrostBand) return null;
    const pitchX = CELL_W + CELL_GAP_X;
    const cols = Math.max(1, Math.min(MAX_MATRIX_COLS, Math.floor((bifrostBand.width - 8) / pitchX)));
    const livePitchX = cols > 1 ? Math.max(pitchX, (bifrostBand.width - 8 - CELL_W) / (cols - 1)) : 0;
    const startX = bifrostBand.x + 4;
    const center = (matrixRows - 1) / 2;
    const cells = [];
    const core = [];
    const upper = [];
    const lower = [];

    for (let col = 0; col < cols; col += 1) {
      const progress = cols > 1 ? col / (cols - 1) : 0;
      const x = startX + col * livePitchX;
      const voltage = voltageAtX(x);
      const curveFreq = curveFreqAt(voltage);
      const preAnchor = voltage < anchorMv;
      const preSpan = Math.max(1, anchorMv - bifrostBand.startMv);
      const postSpan = Math.max(1, V_MAX - anchorMv);
      const preProgress = clamp((voltage - bifrostBand.startMv) / preSpan);
      const postProgress = clamp((voltage - anchorMv) / postSpan);
      const anchorFocus = 1 - smoothstep(Math.abs(voltage - anchorMv) / 74);
      const settle = smoothstep((voltage - (anchorMv - 32)) / 148);
      const postEnergy = smoothstep(postProgress / 0.34);
      const sectionEnergy = clamp(
        preAnchor
          ? 0.3 + preProgress * 0.24 + anchorFocus * 0.34
          : 0.78 + postEnergy * 0.2 + anchorFocus * 0.18,
        0.28,
        1,
      );
      const rowSpread = clamp(
        preAnchor
          ? 0.48 + preProgress * 0.18 + anchorFocus * 0.18
          : 0.76 + postEnergy * 0.2 + anchorFocus * 0.08,
        0.46,
        1.08,
      );
      const halfEnvelope = (matrixHeight / 2 + 7) * rowSpread;
      const centerFreq = curveFreq + (targetMhz - curveFreq) * settle;
      const centerY = sy(centerFreq);
      core.push({ x, y: centerY });
      upper.push({ x, y: centerY - halfEnvelope });
      lower.push({ x, y: centerY + halfEnvelope });

      for (let row = 0; row < matrixRows; row += 1) {
        const edgeDist = Math.abs(row - center) / Math.max(1, center);
        const centerCell = edgeDist < 0.18;
        const phaseSeed = cellHash(col, row, 1);
        const colorSeed = cellHash(col, row, 2);
        const accentSeed = cellHash(col, row, 3);
        const accentCell = edgeDist < 0.58 && accentSeed > (preAnchor ? 0.996 : 0.984);
        const anchorGold = anchorFocus > 0.28 && edgeDist < 0.42 && colorSeed > 0.48;
        const targetGold = !preAnchor && edgeDist < 0.38 && colorSeed > 0.84;
        const goldCell = anchorGold || targetGold;
        const phase = Math.floor(phaseSeed * 48);
        const warmCell = centerCell && (!preAnchor || anchorFocus > 0.36);
        const palette = cellPalette({ edgeDist, progress, sectionEnergy, accentCell, goldCell, warmCell });
        const offset = (row - center) * (CELL_H + CELL_GAP_Y) * rowSpread;
        const delay = `-${(phase * 0.098).toFixed(3)}s`;
        cells.push({
          x,
          y: centerY + offset,
          row,
          col,
          center: centerCell,
          mid: edgeDist < 0.62,
          edge: edgeDist > 0.78,
          gold: goldCell,
          accent: accentCell,
          style: [
            `--phase-delay: ${delay}`,
            `--rest-fill: ${palette.rest}`,
            `--glow-fill: ${palette.glow}`,
            `--peak-fill: ${palette.peak}`,
            `--idle-opacity: ${palette.idle.toFixed(3)}`,
            `--glow-opacity: ${palette.glowOpacity.toFixed(3)}`,
            `--peak-opacity: ${palette.peakOpacity.toFixed(3)}`,
            `--static-opacity: ${((palette.idle + palette.glowOpacity) / 2).toFixed(3)}`,
          ].join("; "),
        });
      }
    }

    const linePath = (items) =>
      items.map((p, i) => `${i === 0 ? "M" : "L"}${p.x.toFixed(1)},${p.y.toFixed(1)}`).join(" ");
    const envelopePath = `${linePath(upper)} ${[...lower]
      .reverse()
      .map((p) => `L${p.x.toFixed(1)},${p.y.toFixed(1)}`)
      .join(" ")} Z`;

    return {
      cells,
      corePath: linePath(core),
      upperPath: linePath(upper),
      lowerPath: linePath(lower),
      envelopePath,
      cols,
    };
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

  {#if bifrostBand && bifrostMatrix}
    <g class="bifrost-band">
      <path class="band-glow" d={bifrostMatrix.envelopePath} />
      <path class="band-base" d={bifrostMatrix.envelopePath} />
      <path class="band-boundary upper" d={bifrostMatrix.upperPath} />
      <path class="band-boundary lower" d={bifrostMatrix.lowerPath} />
      <g class="band-matrix" aria-hidden="true">
        {#each bifrostMatrix.cells as cell}
          <rect
            class="matrix-cell bifrost-cell"
            class:center={cell.center}
            class:mid={cell.mid}
            class:edge={cell.edge}
            class:gold={cell.gold}
            class:accent={cell.accent}
            x={cell.x}
            y={cell.y}
            width={CELL_W}
            height={CELL_H}
            rx="1"
            style={cell.style}
          />
        {/each}
      </g>
      <path
        class="band-core-glow"
        d={bifrostMatrix.corePath}
      />
      <path
        class="band-core"
        d={bifrostMatrix.corePath}
      />
      <title>Optimized boost curve flow. Not a hard voltage cap.</title>
    </g>
  {/if}

  {#if anchorMarker}
    <circle class="anchor-halo" cx={anchorMarker.x} cy={anchorMarker.y} r="20" role="presentation" />
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
    stroke: rgba(136, 192, 208, 0.045);
    stroke-width: 1;
  }
  .grid.major {
    stroke: rgba(136, 192, 208, 0.105);
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
    stroke: rgba(214, 168, 93, 0.28);
    stroke-width: 1.35;
    stroke-linecap: round;
    filter: drop-shadow(0 0 4px rgba(126, 173, 190, 0.18));
  }
  .bifrost-band {
    opacity: 0.98;
  }
  .band-glow {
    fill: rgba(50, 72, 86, 0.18);
    stroke: rgba(214, 168, 93, 0.08);
    stroke-width: 4.4;
    stroke-linejoin: round;
    filter: drop-shadow(0 0 7px rgba(126, 173, 190, 0.14));
  }
  .band-base {
    fill: rgba(37, 48, 61, 0.3);
    stroke: none;
  }
  .band-boundary {
    fill: none;
    stroke: rgba(126, 173, 190, 0.28);
    stroke-width: 1;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .band-boundary.upper,
  .band-boundary.lower {
    opacity: 0.66;
  }
  .band-matrix {
    opacity: 1;
  }
  .matrix-cell {
    fill: var(--rest-fill);
    opacity: var(--idle-opacity);
  }
  .bifrost-cell {
    animation: bifrost-cell-pulse 4.5s cubic-bezier(0.45, 0, 0.25, 1) infinite;
    animation-delay: var(--phase-delay);
    animation-fill-mode: both;
  }
  .band-core-glow {
    fill: none;
    stroke: rgba(126, 173, 190, 0.3);
    stroke-width: 5.5;
    stroke-linecap: round;
    stroke-linejoin: round;
    opacity: 0.52;
    filter: drop-shadow(0 0 7px rgba(214, 168, 93, 0.16));
  }
  .band-core {
    fill: none;
    stroke: rgba(231, 188, 107, 0.74);
    stroke-width: 2.1;
    stroke-linecap: round;
    stroke-linejoin: round;
    opacity: 0.9;
  }
  .anchor-halo {
    fill: rgba(214, 168, 93, 0.1);
    stroke: rgba(126, 173, 190, 0.22);
    stroke-width: 1;
    filter: drop-shadow(0 0 8px rgba(214, 168, 93, 0.2));
    pointer-events: none;
  }
  .anchor-line {
    stroke: rgba(214, 168, 93, 0.66);
    stroke-width: 1;
    stroke-dasharray: 3 5;
  }
  .anchor-dot {
    fill: var(--forge-gold);
    stroke: #0a101c;
    stroke-width: 1.8;
    filter: drop-shadow(0 0 5px rgba(214, 168, 93, 0.36));
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
    .bifrost-cell {
      animation: none;
      transition: none;
    }
    .bifrost-cell {
      fill: var(--glow-fill);
      opacity: var(--static-opacity);
    }
  }
  @keyframes bifrost-cell-pulse {
    0%,
    38%,
    100% {
      fill: var(--rest-fill);
      opacity: var(--idle-opacity);
    }
    50% {
      fill: var(--glow-fill);
      opacity: var(--glow-opacity);
    }
    60% {
      fill: var(--peak-fill);
      opacity: var(--peak-opacity);
    }
    72% {
      fill: var(--glow-fill);
      opacity: var(--glow-opacity);
    }
  }
</style>
