<script>
  import { serviceCall, formatDriverStatus } from "../service.js";

  let sensors = $state(null);
  let driverStatus = $state(null);
  let error = $state(null);
  let timer = $state(null);

  function srcLabel(source, quality) {
    if (!source) return "";
    const q = quality ? ` · ${quality}` : "";
    return `${source}${q}`;
  }

  async function refresh() {
    try {
      const [s, d] = await Promise.all([
        serviceCall("ReadSensors"),
        serviceCall("GetDriverStatus"),
      ]);
      sensors = s?.data?.type === "Sensors" ? s.data : null;
      driverStatus = d?.data?.type === "DriverStatus" ? d.data : null;
      error = null;
    } catch (e) {
      error = String(e);
    }
  }

  $effect(() => {
    refresh();
    timer = setInterval(refresh, 2000);
    return () => clearInterval(timer);
  });
</script>

<section class="dash">
  <header class="dash-head">
    <h2>Sensores ao vivo</h2>
    {#if driverStatus}
      <span class="badge">{formatDriverStatus(driverStatus)}</span>
    {/if}
  </header>

  {#if error}
    <p class="err">{error}</p>
  {:else if sensors}
    {#if sensors.motherboard?.vendor}
      <article class="tile board">
        <span class="lab">Placa-mãe</span>
        <p class="val">{sensors.motherboard.vendor} {sensors.motherboard.model}</p>
        <p class="sub">
          {#if sensors.motherboard.superio_chip}
            Super I/O {sensors.motherboard.superio_chip}
          {:else}
            Super I/O não detectado
          {/if}
        </p>
        <p class="sub meta">
          Perfil: {sensors.motherboard.profile_id}
          <span class="dim">({sensors.motherboard.profile_source})</span>
        </p>
        {#if sensors.motherboard.rails?.length}
          <ul class="rails">
            {#each sensors.motherboard.rails as rail}
              <li>
                <span class="rail-name">{rail.label}</span>
                <span class="rail-val">{rail.voltage_mv} mV</span>
                <span class="rail-meta">{rail.role} · {rail.source}</span>
              </li>
            {/each}
          </ul>
        {/if}
      </article>
    {/if}

    <div class="grid">
      <article class="tile">
        <span class="lab">CPU</span>
        <p class="val">{sensors.cpu.utilization_pct.toFixed(1)}% uso</p>
        <p class="sub">{sensors.cpu.clock_mhz ?? "—"} MHz</p>
        {#if sensors.cpu.temperature_c != null}
          <p class="sub">{sensors.cpu.temperature_c.toFixed(1)} °C</p>
          {#if sensors.cpu.temperature_source}
            <p class="sub dim">{srcLabel(sensors.cpu.temperature_source)}</p>
          {/if}
        {/if}
        <p class="sub">
          {sensors.cpu.voltage_mv ? `${sensors.cpu.voltage_mv} mV` : "Vcore N/A"}
        </p>
        {#if sensors.cpu.voltage_source}
          <p class="sub dim">
            {srcLabel(sensors.cpu.voltage_source, sensors.cpu.voltage_quality)}
          </p>
        {/if}
      </article>
      <article class="tile">
        <span class="lab">Memória</span>
        <p class="val">{sensors.memory.used_mb} / {sensors.memory.total_mb} MB</p>
        <p class="sub">{sensors.memory.used_pct.toFixed(1)}% em uso</p>
        <p class="sub">
          {sensors.memory.voltage_mv ? `${sensors.memory.voltage_mv} mV` : "Tensão N/A"}
        </p>
        {#if sensors.memory.voltage_source}
          <p class="sub dim">
            {srcLabel(sensors.memory.voltage_source, sensors.memory.voltage_quality)}
          </p>
        {/if}
      </article>
      <article class="tile">
        <span class="lab">WHEA</span>
        <p class="val">{sensors.whea.error_count} eventos</p>
        {#if sensors.whea.last_error}
          <p class="sub warn">{sensors.whea.last_error}</p>
        {/if}
      </article>
      {#each sensors.gpu as gpu}
        <article class="tile">
          <span class="lab">GPU</span>
          <p class="val">{gpu.name}</p>
          {#if gpu.utilization_pct != null}
            <p class="sub">{gpu.utilization_pct.toFixed(1)}% uso</p>
          {/if}
          {#if gpu.temperature_c != null}
            <p class="sub">{gpu.temperature_c.toFixed(0)} °C</p>
            {#if gpu.temperature_source}
              <p class="sub dim">{gpu.temperature_source}</p>
            {/if}
          {/if}
          {#if gpu.power_w != null}
            <p class="sub">{gpu.power_w.toFixed(1)} W</p>
            {#if gpu.power_source}
              <p class="sub dim">{gpu.power_source}</p>
            {/if}
          {/if}
          {#if gpu.core_clock_mhz}
            <p class="sub">Core {gpu.core_clock_mhz} MHz</p>
          {/if}
          {#if gpu.memory_clock_mhz}
            <p class="sub">VRAM {gpu.memory_clock_mhz} MHz</p>
          {/if}
          {#if gpu.vram_used_mb != null && gpu.vram_total_mb != null}
            <p class="sub">{gpu.vram_used_mb} / {gpu.vram_total_mb} MB VRAM</p>
          {:else if gpu.vram_total_mb != null}
            <p class="sub">{gpu.vram_total_mb} MB VRAM total</p>
          {/if}
        </article>
      {/each}
    </div>
  {:else}
    <p class="wait">Aguardando dados…</p>
  {/if}
</section>

<style>
  .dash {
    --surface: rgba(19, 31, 46, 0.82);
    --border: var(--nord-border-card);
    --muted: var(--nord-mist);
    --text: var(--nord-silver);
    --green: var(--nord-aurora);
  }
  .dash-head {
    display: flex;
    align-items: center;
    gap: 1rem;
    margin-bottom: 1rem;
  }
  .dash-head h2 {
    margin: 0;
    font-size: 0.85rem;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--muted);
  }
  .badge {
    font-size: 0.72rem;
    font-weight: 600;
    background: rgba(10, 16, 28, 0.82);
    border: 1px solid var(--nord-border);
    padding: 0.3rem 0.65rem;
    border-radius: 999px;
    color: var(--muted);
  }
  .board {
    margin-bottom: 0.85rem;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
    gap: 0.85rem;
  }
  .tile {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 12px;
    padding: 1rem 1.1rem;
  }
  .lab {
    display: block;
    font-size: 0.68rem;
    font-weight: 700;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: var(--nord-dim);
    margin-bottom: 0.4rem;
  }
  .val {
    margin: 0;
    font-weight: 600;
    color: var(--text);
  }
  .sub {
    margin: 0.25rem 0 0;
    font-size: 0.82rem;
    color: var(--muted);
  }
  .sub.dim {
    font-size: 0.72rem;
    color: var(--nord-dim);
  }
  .sub.warn {
    color: var(--nord-ember-bright);
  }
  .meta .dim {
    color: var(--nord-dim);
  }
  .rails {
    list-style: none;
    margin: 0.65rem 0 0;
    padding: 0;
    font-size: 0.78rem;
    border-top: 1px solid var(--nord-border);
  }
  .rails li {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 0.15rem 0.5rem;
    padding: 0.35rem 0;
    border-bottom: 1px solid rgba(255, 255, 255, 0.04);
  }
  .rail-name {
    color: var(--text);
  }
  .rail-val {
    color: var(--green);
    font-variant-numeric: tabular-nums;
  }
  .rail-meta {
    grid-column: 1 / -1;
    color: var(--nord-dim);
    font-size: 0.68rem;
  }
  .wait {
    color: var(--nord-dim);
  }
  .err {
    color: var(--nord-danger);
  }
</style>
