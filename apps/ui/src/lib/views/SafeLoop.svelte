<script>
  import { serviceCall } from "../service.js";

  let status = $state(null);
  let error = $state(null);
  let timer = $state(null);

  const STATE_LABEL = {
    idle: "Protected",
    probing: "Probing",
    applying: "Applying",
    dwell: "Observing",
    validated: "Validated",
    unstable: "Needs attention",
    safe_mode: "Safe Mode",
  };

  const CRASH_LABEL = {
    oc_instability: "GPU tuning instability",
    unrelated: "Unrelated",
    unknown: "Unknown",
  };
  const AXIS_LABEL = {
    core_mhz: "Core",
    freq_mhz: "Core",
    voltage_mv: "Voltage",
    mem_offset_mhz: "Memory offset",
    power_w: "Power",
  };

  function stateLabel(s) {
    return STATE_LABEL[s] ?? s ?? "Unknown";
  }

  function axisText(key, value) {
    const label = AXIS_LABEL[key] ?? String(key).replace(/_/g, " ");
    const unit = key.includes("mhz") ? " MHz" : key.includes("mv") ? " mV" : key.endsWith("_w") ? " W" : "";
    const signed = key.includes("offset") && value > 0 ? `+${value}` : value;
    return `${label}: ${signed}${unit}`;
  }

  function pointText(p) {
    if (!p || !p.axes || Object.keys(p.axes).length === 0) return "Stock settings";
    return Object.entries(p.axes)
      .map(([k, v]) => axisText(k, v))
      .join(" / ");
  }

  async function refresh() {
    try {
      const r = await serviceCall("GetSafeLoopStatus");
      status = r?.data?.type === "SafeLoop" ? r.data : null;
      error = null;
    } catch (e) {
      error = String(e);
    }
  }

  $effect(() => {
    refresh();
    timer = setInterval(refresh, 3000);
    return () => clearInterval(timer);
  });
</script>

<section class="safe">
  <header class="safe-head">
    <span class="eyebrow">Safety</span>
    <h2>Safe Loop recovery</h2>
    <p class="lead">
      Safe Loop is Nidavellir's recovery layer for GPU tuning. Before risky GPU steps run, it records a boot flag.
      If the machine crashes before the flag is cleared, the service can recover on the next boot and avoid repeating the unstable region.
    </p>
  </header>

  {#if error}
    <p class="err">{error}</p>
  {:else if status}
    {#if status.safe_mode}
      <div class="alert">
        <strong>Safe Mode is active.</strong> After {status.crash_threshold} consecutive crashes, Nidavellir returned to stock and stopped tuning actions.
      </div>
    {:else if status.boot_flag_armed}
      <div class="alert alert--warn">
        <strong>Recovery is armed.</strong> A GPU tuning step is being validated. If the system crashes, recovery runs on the next boot.
      </div>
    {/if}

    <div class="grid">
      <article class="tile">
        <span class="lab">Protection state</span>
        <p class="val">{stateLabel(status.state)}</p>
      </article>
      <article class="tile">
        <span class="lab">Consecutive crashes</span>
        <p class="val" class:danger={status.consecutive_crashes > 0}>
          {status.consecutive_crashes} / {status.crash_threshold}
        </p>
        <p class="sub">Safe Mode threshold</p>
      </article>
      <article class="tile">
        <span class="lab">Boot flag</span>
        <p class="val">{status.boot_flag_armed ? "Armed" : "Clear"}</p>
      </article>
      <article class="tile">
        <span class="lab">Last validated point</span>
        <p class="val small">{pointText(status.last_validated)}</p>
      </article>
    </div>

    <div class="section">
      <h3 class="section-head">Blocked unstable regions ({status.blacklist.length})</h3>
      {#if status.blacklist.length}
        <ul class="list">
          {#each status.blacklist as region}
            <li>
              <span class="mono">{pointText(region.center)}</span>
              <span class="dim">radius +/-{region.radius}</span>
            </li>
          {/each}
        </ul>
      {:else}
        <p class="empty">No unstable GPU tuning region is currently blocked.</p>
      {/if}
    </div>

    {#if status.recent_crashes?.length}
      <div class="section">
        <h3 class="section-head">Recent recovery signals</h3>
        <ul class="list">
          {#each status.recent_crashes as c}
            <li><span class="mono">{CRASH_LABEL[c] ?? c}</span></li>
          {/each}
        </ul>
      </div>
    {/if}
  {:else}
    <p class="wait">Waiting for the service...</p>
  {/if}
</section>

<style>
  .safe {
    --surface: rgba(19, 31, 46, 0.82);
    --border: var(--nord-border-card);
    --muted: var(--nord-mist);
    --text: var(--nord-silver);
    display: flex;
    flex-direction: column;
    gap: 1.05rem;
  }
  .eyebrow,
  .lab {
    display: block;
    font-size: 0.68rem;
    font-weight: 700;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--nord-dim);
    margin-bottom: 0.35rem;
  }
  .safe-head h2 {
    margin: 0 0 0.45rem;
    font-size: 1.05rem;
    color: var(--text);
  }
  .lead {
    margin: 0;
    font-size: 0.88rem;
    line-height: 1.55;
    color: var(--muted);
    max-width: 74ch;
  }
  .alert {
    border-radius: 10px;
    padding: 0.85rem 1rem;
    font-size: 0.88rem;
    line-height: 1.5;
    background: rgba(191, 97, 106, 0.14);
    border: 1px solid rgba(191, 97, 106, 0.4);
    color: #f3b9bd;
  }
  .alert--warn {
    background: rgba(232, 162, 58, 0.12);
    border-color: rgba(232, 162, 58, 0.4);
    color: var(--nord-ember-bright);
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
    gap: 0.85rem;
  }
  .tile {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 0.9rem 1rem;
    box-shadow: var(--forge-panel-edge);
  }
  .val {
    margin: 0;
    font-weight: 700;
    color: var(--text);
    font-size: 1.02rem;
    font-variant-numeric: tabular-nums;
  }
  .val.small {
    font-size: 0.82rem;
    font-weight: 600;
    overflow-wrap: anywhere;
  }
  .val.danger {
    color: var(--nord-danger);
  }
  .sub {
    margin: 0.25rem 0 0;
    font-size: 0.75rem;
    color: var(--nord-dim);
  }
  .section-head {
    margin: 0 0 0.5rem;
    font-size: 0.72rem;
    font-weight: 800;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--muted);
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
  }
  .list li {
    display: flex;
    justify-content: space-between;
    gap: 1rem;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 9px;
    padding: 0.55rem 0.8rem;
    font-size: 0.82rem;
  }
  .mono {
    font-variant-numeric: tabular-nums;
    color: var(--text);
    overflow-wrap: anywhere;
  }
  .dim {
    color: var(--nord-dim);
    white-space: nowrap;
  }
  .empty,
  .wait {
    color: var(--nord-dim);
    font-size: 0.9rem;
  }
  .err {
    color: var(--nord-danger);
  }
</style>
