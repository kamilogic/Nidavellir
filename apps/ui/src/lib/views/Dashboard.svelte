<script>
  import { serviceCall, formatDriverStatus } from "../service.js";

  let sensors = $state(null);
  let driverStatus = $state(null);
  let error = $state(null);
  let timer = $state(null);

  const gpus = $derived(sensors?.gpu ?? []);

  function srcLabel(source, quality) {
    if (!source) return "";
    const q = quality ? ` / ${quality}` : "";
    return `${source}${q}`;
  }

  function fixed(value, digits = 0) {
    const n = Number(value);
    return Number.isFinite(n) ? n.toFixed(digits) : null;
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
    <div>
      <span class="eyebrow">GPU Diagnostics</span>
      <h2>Live GPU state</h2>
      <p>Read-only telemetry for the current GPU target. Tuning remains inside Forge.</p>
    </div>
    {#if driverStatus}
      <span class="badge">{formatDriverStatus(driverStatus)}</span>
    {/if}
  </header>

  {#if error}
    <p class="err">{error}</p>
  {:else if sensors}
    {#if gpus.length}
      <div class="grid gpu-grid">
        {#each gpus as gpu}
          <article class="tile gpu-tile">
            <span class="lab">GPU</span>
            <p class="val">{gpu.name}</p>
            <div class="metric-grid">
              <div>
                <span>Utilization</span>
                <strong>{gpu.utilization_pct != null ? `${fixed(gpu.utilization_pct, 1)}%` : "N/A"}</strong>
              </div>
              <div>
                <span>Temperature</span>
                <strong>{gpu.temperature_c != null ? `${fixed(gpu.temperature_c)} C` : "N/A"}</strong>
              </div>
              <div>
                <span>Power</span>
                <strong>{gpu.power_w != null ? `${fixed(gpu.power_w, 1)} W` : "N/A"}</strong>
              </div>
              <div>
                <span>Core clock</span>
                <strong>{gpu.core_clock_mhz ? `${gpu.core_clock_mhz} MHz` : "N/A"}</strong>
              </div>
              <div>
                <span>Memory clock</span>
                <strong>{gpu.memory_clock_mhz ? `${gpu.memory_clock_mhz} MHz` : "N/A"}</strong>
              </div>
              <div>
                <span>VRAM</span>
                <strong>
                  {#if gpu.vram_used_mb != null && gpu.vram_total_mb != null}
                    {gpu.vram_used_mb} / {gpu.vram_total_mb} MB
                  {:else if gpu.vram_total_mb != null}
                    {gpu.vram_total_mb} MB
                  {:else}
                    N/A
                  {/if}
                </strong>
              </div>
            </div>
            {#if gpu.temperature_source || gpu.power_source}
              <p class="sub dim">
                {#if gpu.temperature_source}Temp source: {gpu.temperature_source}{/if}
                {#if gpu.temperature_source && gpu.power_source} / {/if}
                {#if gpu.power_source}Power source: {gpu.power_source}{/if}
              </p>
            {/if}
          </article>
        {/each}
      </div>
    {:else}
      <p class="wait">No GPU sensor data is available yet.</p>
    {/if}

    <div class="support-grid">
      <article class="tile">
        <span class="lab">WHEA</span>
        <p class="val">{sensors.whea.error_count} events</p>
        {#if sensors.whea.last_error}
          <p class="sub warn">{sensors.whea.last_error}</p>
        {:else}
          <p class="sub">No recent hardware error surfaced by this sensor pass.</p>
        {/if}
      </article>
      <article class="tile">
        <span class="lab">Diagnostics role</span>
        <p class="val">Read-only support</p>
        <p class="sub">This screen does not tune CPU, RAM, or motherboard settings.</p>
      </article>
    </div>

    <details class="system-context">
      <summary>Supporting system context</summary>
      <div class="grid">
        <article class="tile">
          <span class="lab">CPU context</span>
          <p class="val">{sensors.cpu.utilization_pct.toFixed(1)}% utilization</p>
          <p class="sub">{sensors.cpu.clock_mhz ?? "N/A"} MHz</p>
          {#if sensors.cpu.temperature_c != null}
            <p class="sub">{sensors.cpu.temperature_c.toFixed(1)} C</p>
            {#if sensors.cpu.temperature_source}
              <p class="sub dim">{srcLabel(sensors.cpu.temperature_source)}</p>
            {/if}
          {/if}
          <p class="sub">{sensors.cpu.voltage_mv ? `${sensors.cpu.voltage_mv} mV` : "Vcore N/A"}</p>
        </article>
        <article class="tile">
          <span class="lab">Memory context</span>
          <p class="val">{sensors.memory.used_mb} / {sensors.memory.total_mb} MB</p>
          <p class="sub">{sensors.memory.used_pct.toFixed(1)}% used</p>
          <p class="sub">{sensors.memory.voltage_mv ? `${sensors.memory.voltage_mv} mV` : "Voltage N/A"}</p>
        </article>
        {#if sensors.motherboard?.vendor}
          <article class="tile">
            <span class="lab">Board context</span>
            <p class="val">{sensors.motherboard.vendor} {sensors.motherboard.model}</p>
            <p class="sub">
              {#if sensors.motherboard.superio_chip}
                Super I/O {sensors.motherboard.superio_chip}
              {:else}
                Super I/O not detected
              {/if}
            </p>
            <p class="sub dim">Profile: {sensors.motherboard.profile_id} ({sensors.motherboard.profile_source})</p>
          </article>
        {/if}
      </div>
    </details>
  {:else}
    <p class="wait">Waiting for sensor data...</p>
  {/if}
</section>

<style>
  .dash {
    --surface: var(--forge-panel);
    --border: transparent;
    --muted: var(--nord-mist);
    --text: var(--nord-silver);
    --green: var(--nord-aurora);
    display: flex;
    flex-direction: column;
    gap: 1rem;
  }
  .dash-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 1rem;
  }
  .eyebrow,
  .lab,
  .metric-grid span {
    display: block;
    font-size: 0.68rem;
    font-weight: 700;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--nord-dim);
    margin-bottom: 0.35rem;
  }
  .dash-head h2 {
    margin: 0;
    font-size: 1.05rem;
    color: var(--text);
  }
  .dash-head p {
    margin: 0.35rem 0 0;
    color: var(--muted);
    font-size: 0.86rem;
    line-height: 1.5;
  }
  .badge {
    font-size: 0.72rem;
    font-weight: 700;
    background: var(--forge-panel-raised);
    border: none;
    padding: 0.38rem 0.7rem;
    border-radius: 999px;
    color: var(--muted);
    white-space: nowrap;
  }
  .grid,
  .support-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
    gap: 0.85rem;
  }
  .gpu-grid {
    grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
  }
  .support-grid {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
  .tile {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 0.9rem 1rem;
    box-shadow: var(--forge-panel-edge);
  }
  .gpu-tile {
    background: var(--forge-panel-bg);
    border-color: var(--forge-line);
  }
  .val {
    margin: 0;
    font-weight: 700;
    color: var(--text);
  }
  .metric-grid {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 0.55rem;
    margin-top: 0.85rem;
  }
  .metric-grid div {
    border: 1px solid rgba(255, 255, 255, 0.055);
    border-radius: 8px;
    background: rgba(5, 7, 11, 0.24);
    padding: 0.58rem 0.65rem;
    min-width: 0;
  }
  .metric-grid strong {
    display: block;
    color: var(--text);
    font-size: 0.86rem;
    font-variant-numeric: tabular-nums;
    overflow-wrap: anywhere;
  }
  .sub {
    margin: 0.3rem 0 0;
    font-size: 0.82rem;
    line-height: 1.45;
    color: var(--muted);
  }
  .sub.dim {
    font-size: 0.74rem;
    color: var(--nord-dim);
  }
  .sub.warn {
    color: var(--nord-ember-bright);
  }
  .system-context {
    border: 1px solid var(--forge-line);
    border-radius: 10px;
    padding: 0.8rem 0.9rem;
    background: rgba(5, 7, 11, 0.22);
  }
  .system-context summary {
    display: inline-flex;
    align-items: center;
    gap: 0.45rem;
    cursor: pointer;
    color: var(--muted);
    font-size: 0.76rem;
    font-weight: 800;
    letter-spacing: 0.08em;
    list-style: none;
    text-transform: uppercase;
  }
  .system-context summary::-webkit-details-marker {
    display: none;
  }
  .system-context summary::after {
    content: "";
    width: 0.38rem;
    height: 0.38rem;
    border-right: 1.5px solid currentColor;
    border-bottom: 1.5px solid currentColor;
    opacity: 0.76;
    transform: rotate(45deg) translateY(-0.12rem);
    transition: transform 0.15s ease;
  }
  .system-context[open] summary::after {
    transform: rotate(-135deg) translateY(-0.12rem);
  }
  .system-context .grid {
    margin-top: 0.85rem;
  }
  .wait {
    color: var(--nord-dim);
  }
  .err {
    color: var(--nord-danger);
  }
  @media (max-width: 760px) {
    .dash-head {
      flex-direction: column;
    }
    .support-grid,
    .metric-grid {
      grid-template-columns: 1fr;
    }
  }
</style>
