<script>
  let {
    values = [],
    color = "#8fbf3d",
    fill = "rgba(143, 191, 61, 0.08)",
    height = 54,
  } = $props();

  let canvas;
  let width = $state(240);

  function render() {
    if (!canvas) return;
    const ratio = Math.max(1, globalThis.devicePixelRatio ?? 1);
    canvas.width = Math.max(1, Math.round(width * ratio));
    canvas.height = Math.max(1, Math.round(height * ratio));
    const context = canvas.getContext("2d");
    if (!context) return;
    context.setTransform(ratio, 0, 0, ratio, 0, 0);
    context.clearRect(0, 0, width, height);

    const points = values.map(Number).filter(Number.isFinite);
    if (points.length < 2) return;
    const min = Math.min(...points);
    const max = Math.max(...points);
    const span = Math.max(1, max - min);
    const pad = 3;
    const x = (index) => pad + (index / (points.length - 1)) * (width - pad * 2);
    const y = (value) => height - pad - ((value - min) / span) * (height - pad * 2);

    context.beginPath();
    context.moveTo(x(0), y(points[0]));
    for (let index = 1; index < points.length; index += 1) {
      context.lineTo(x(index), y(points[index]));
    }
    context.lineTo(x(points.length - 1), height);
    context.lineTo(x(0), height);
    context.closePath();
    context.fillStyle = fill;
    context.fill();

    context.beginPath();
    context.moveTo(x(0), y(points[0]));
    for (let index = 1; index < points.length; index += 1) {
      context.lineTo(x(index), y(points[index]));
    }
    context.strokeStyle = color;
    context.lineWidth = 1.45;
    context.lineJoin = "round";
    context.lineCap = "round";
    context.stroke();
  }

  function observe(node) {
    const resize = new ResizeObserver(([entry]) => {
      width = Math.max(1, Math.round(entry.contentRect.width));
    });
    resize.observe(node);
    return { destroy: () => resize.disconnect() };
  }

  $effect(() => {
    values;
    color;
    fill;
    width;
    height;
    render();
  });
</script>

<div class="spark" style={`height:${height}px`} use:observe aria-hidden="true">
  <canvas bind:this={canvas}></canvas>
</div>

<style>
  .spark,
  canvas {
    display: block;
    width: 100%;
  }

  canvas {
    height: 100%;
  }
</style>
