<script>
  import { invoke } from "@tauri-apps/api/core";

  let hwData = $state(null);
  let sensorData = $state(null);
  let sweepProgress = $state(null);
  let hwLoading = $state(true);
  let sensorLoading = $state(true);
  let sweepLoading = $state(false);
  let error = $state(null);
  let activeTab = $state("hardware");
  let autoRefresh = $state(true);
  let refreshInterval = $state(null);
  let hwRefreshInterval = $state(null);
  let sweepPoll = $state(null);
  let profiles = $state(null);
  let profileLoading = $state(false);

  // Sweep config (editable)
  let sweepParam = $state("CpuCoreVoltage");
  let rangeStart = $state(-100);
  let rangeEnd = $state(50);
  let stepSize = $state(5);
  let testDuration = $state(30);
  let cpuThreads = $state(4);

  const paramLabels = {
    CpuCoreVoltage: "CPU Core Voltage (mV)",
    CpuCacheVoltage: "CPU Cache Voltage (mV)",
    CpuPowerLimit: "CPU Power Limit PL1 (W)",
    CpuTurboRatio: "CPU Turbo Ratio (x)",
  };

  const paramDefaults = {
    CpuCoreVoltage: [-100, 50, 5],
    CpuCacheVoltage: [-100, 50, 5],
    CpuPowerLimit: [65, 150, 5],
    CpuTurboRatio: [40, 55, 1],
  };

  function updateDefaults() {
    const d = paramDefaults[sweepParam];
    if (d) { rangeStart = d[0]; rangeEnd = d[1]; stepSize = d[2]; }
  }

  async function loadHw() {
    hwLoading = true;
    try { hwData = await invoke("detect_hardware"); }
    catch (e) { error = String(e); }
    finally { hwLoading = false; }
  }

  async function loadSensors() {
    if (!sensorData) sensorLoading = true;
    try { sensorData = await invoke("read_sensors"); }
    catch (e) { error = String(e); }
    finally { sensorLoading = false; }
  }

  function toggleAutoRefresh() {
    autoRefresh = !autoRefresh;
    if (autoRefresh) {
      loadSensors();
      refreshInterval = setInterval(loadSensors, 2000);
    } else {
      clearInterval(refreshInterval);
      refreshInterval = null;
    }
  }

  async function startSweep() {
    sweepLoading = true;
    try {
      await invoke("start_sweep", {
        config: {
          param: sweepParam,
          range_start: rangeStart,
          range_end: rangeEnd,
          step: stepSize,
          test_duration_secs: testDuration,
          cpu_threads: cpuThreads,
        },
      });
      sweepPoll = setInterval(pollSweep, 1000);
    } catch (e) { error = String(e); }
    finally { sweepLoading = false; }
  }

  async function stopSweep() {
    clearInterval(sweepPoll);
    sweepPoll = null;
    await invoke("stop_sweep");
  }

  async function resetSweep() {
    clearInterval(sweepPoll);
    sweepPoll = null;
    await invoke("reset_sweep");
    sweepProgress = null;
  }

  async function pollSweep() {
    try {
      const p = await invoke("get_sweep_progress");
      sweepProgress = p;
      if (p.state === "Completed" || p.state === "Stopped" || p.state === "Failed") {
        clearInterval(sweepPoll);
        sweepPoll = null;
      }
    } catch (e) {
      clearInterval(sweepPoll);
      sweepPoll = null;
    }
  }

  async function generateProfiles() {
    profileLoading = true;
    try { profiles = await invoke("generate_profiles"); }
    catch (e) { error = String(e); }
    finally { profileLoading = false; }
  }

  async function loadSavedProfiles() {
    try { profiles = await invoke("get_profiles"); }
    catch (_) { /* no saved profiles yet */ }
  }

  async function applyProfile(profile) {
    try { await invoke("apply_profile", { profile }); }
    catch (e) { error = String(e); }
  }

  $effect(() => {
    loadHw();
    loadSavedProfiles();
    hwRefreshInterval = setInterval(loadHw, 5000);
    return () => {
      if (refreshInterval) clearInterval(refreshInterval);
      if (hwRefreshInterval) clearInterval(hwRefreshInterval);
      if (sweepPoll) clearInterval(sweepPoll);
    };
  });

  // Start/stop sensor refresh when switching tabs or toggling auto-refresh
  $effect(() => {
    const shouldRun = activeTab === "sensors" && autoRefresh;
    if (shouldRun && !refreshInterval) {
      loadSensors();
      refreshInterval = setInterval(loadSensors, 2000);
    } else if (!shouldRun && refreshInterval) {
      clearInterval(refreshInterval);
      refreshInterval = null;
    }
  });
</script>

<div class="container">
  <header>
    <h1>Nidavellir ⚒️</h1>
    <p>Forging your silicon to its maximum potential</p>
  </header>

  <nav class="tabs">
    <button class="tab" class:active={activeTab === "hardware"} onclick={() => activeTab = "hardware"}>Hardware</button>
    <button class="tab" class:active={activeTab === "sensors"} onclick={() => activeTab = "sensors"}>Sensors</button>
    <button class="tab" class:active={activeTab === "sweep"} onclick={() => activeTab = "sweep"}>Sweep</button>
  </nav>

  {#if activeTab === "hardware"}
    {#if hwLoading}
      <div class="loading">Detecting hardware...</div>
    {:else if hwData}
      <div class="grid">
        <div class="card">
          <h2>CPU</h2>
          <table><tbody>
            <tr><td>Vendor</td><td>{hwData.cpu.vendor}</td></tr>
            <tr><td>Model</td><td>{hwData.cpu.model}</td></tr>
            <tr><td>Cores</td><td>{hwData.cpu.cores}</td></tr>
            <tr><td>Threads</td><td>{hwData.cpu.threads}</td></tr>
            <tr><td>Base Freq</td><td>{hwData.cpu.base_freq_mhz} MHz</td></tr>
            <tr><td>Max Freq</td><td>{hwData.cpu.max_freq_mhz} MHz</td></tr>
          </tbody></table>
        </div>
        <div class="card">
          <h2>GPU</h2>
          {#each hwData.gpu as gpu, i}
            <table><tbody>
              <tr><td>Vendor</td><td>{gpu.vendor}</td></tr>
              <tr><td>Model</td><td>{gpu.model}</td></tr>
              <tr><td>VRAM</td><td>{gpu.vram_mb} MB</td></tr>
            </tbody></table>
            {#if i < hwData.gpu.length - 1}<hr>{/if}
          {/each}
        </div>
        <div class="card">
          <h2>RAM</h2>
          <table><tbody>
            <tr><td>Total</td><td>{(hwData.ram.total_mb / 1024).toFixed(0)} GB</td></tr>
          </tbody></table>
          {#if hwData.ram.modules.length > 0}
            <hr><h3>Modules</h3>
            {#each hwData.ram.modules as mod, i}
              <table><tbody>
                <tr><td>Size</td><td>{mod.size_mb} MB</td></tr>
                <tr><td>Speed</td><td>{mod.speed_mts} MT/s</td></tr>
              </tbody></table>
              {#if i < hwData.ram.modules.length - 1}<hr>{/if}
            {/each}
          {/if}
        </div>
        <div class="card">
          <h2>Motherboard</h2>
          <table><tbody>
            <tr><td>Vendor</td><td>{hwData.motherboard.vendor}</td></tr>
            <tr><td>Model</td><td>{hwData.motherboard.model}</td></tr>
            <tr><td>BIOS</td><td>{hwData.motherboard.bios_version}</td></tr>
          </tbody></table>
        </div>
      </div>
      <button class="btn" onclick={loadHw}>Refresh</button>
    {/if}
  {:else if activeTab === "sensors"}
    <div class="sensor-bar">
      <span class="sensor-label">Auto-refresh</span>
      <button class="toggle" class:on={autoRefresh} onclick={toggleAutoRefresh}>
        {autoRefresh ? "ON" : "OFF"}
      </button>
      {#if !autoRefresh}
        <button class="btn" onclick={loadSensors}>Refresh</button>
      {/if}
    </div>
    {#if sensorLoading}
      <div class="loading">Reading sensors...</div>
    {:else if sensorData}
      <div class="grid">
        <div class="card">
          <h2>CPU</h2>
          <table><tbody>
            <tr><td>Utilization</td><td>{sensorData.cpu.utilization_pct.toFixed(1)}%</td></tr>
            <tr><td>Clock</td><td>{sensorData.cpu.clock_mhz ?? "—"} MHz</td></tr>
            <tr><td>Vcore</td><td>{sensorData.cpu.voltage_mv ? sensorData.cpu.voltage_mv + " mV" : "N/A (admin req.)"}</td></tr>
          </tbody></table>
        </div>
        <div class="card">
          <h2>Memory</h2>
          <table><tbody>
            <tr><td>Used</td><td>{(sensorData.memory.used_mb / 1024).toFixed(1)} GB / {(sensorData.memory.total_mb / 1024).toFixed(1)} GB</td></tr>
            <tr><td>Usage</td><td>{sensorData.memory.used_pct.toFixed(1)}%</td></tr>
            <tr><td>Voltage</td><td>{sensorData.memory.voltage_mv ? (sensorData.memory.voltage_mv / 1000).toFixed(3) + " V" : "N/A"}</td></tr>
          </tbody></table>
        </div>
        {#if sensorData.superio_voltages && sensorData.superio_voltages.length > 0}
        <div class="card">
          <h2>Voltages (Super I/O)</h2>
          <table><tbody>
            {#each sensorData.superio_voltages as v}
              <tr><td>VIN{v.channel} {v.name}</td><td>{v.voltage_v.toFixed(3)} V</td></tr>
            {/each}
          </tbody></table>
        </div>
        {/if}
        <div class="card">
          <h2>Boot Status</h2>
          <table><tbody>
            <tr><td>Previous crash</td><td>{sensorData.boot_status.previous_boot_crashed ? "Yes ⚠️" : "No ✅"}</td></tr>
          </tbody></table>
        </div>
        <div class="card whea-card">
          <h2>WHEA</h2>
          <table><tbody>
            <tr><td>Errors</td><td>{sensorData.whea.error_count}</td></tr>
          </tbody></table>
          {#if sensorData.whea.events && sensorData.whea.events.length > 0}
            <div class="whea-list">
              {#each sensorData.whea.events as evt, i}
                <div class="whea-event">
                  <div class="whea-meta">
                    <span class="whea-idx">#{i + 1}</span>
                    {#if evt.event_id}<span class="whea-id">ID {evt.event_id}</span>{/if}
                    {#if evt.timestamp}<span class="whea-time">{evt.timestamp}</span>{/if}
                  </div>
                  <div class="whea-desc">{evt.description ?? "No description available"}</div>
                </div>
              {/each}
            </div>
          {:else if sensorData.whea.error_count === 0}
            <p class="whea-ok">No WHEA errors recorded</p>
          {/if}
        </div>
      </div>
    {/if}
  {:else}
    <div class="sweep-config">
      <h3>Sweep Configuration</h3>
      <label class="field">
        <span>Parameter</span>
        <select bind:value={sweepParam} onchange={updateDefaults}>
          {#each Object.keys(paramLabels) as k}
            <option value={k}>{paramLabels[k]}</option>
          {/each}
        </select>
      </label>
      <div class="row">
        <label class="field">
          <span>Start</span>
          <input type="number" bind:value={rangeStart} />
        </label>
        <label class="field">
          <span>End</span>
          <input type="number" bind:value={rangeEnd} />
        </label>
        <label class="field">
          <span>Step</span>
          <input type="number" bind:value={stepSize} step="any" />
        </label>
      </div>
      <div class="row">
        <label class="field">
          <span>Duration (s)</span>
          <input type="number" bind:value={testDuration} min="5" />
        </label>
        <label class="field">
          <span>CPU Threads</span>
          <input type="number" bind:value={cpuThreads} min="1" max="64" />
        </label>
      </div>
      <div class="sweep-actions">
        {#if !sweepProgress || sweepProgress.state === "Idle" || sweepProgress.state === "Completed" || sweepProgress.state === "Stopped"}
          <button class="btn primary" onclick={startSweep} disabled={sweepLoading}>
            {sweepLoading ? "Starting..." : "Start Sweep"}
          </button>
        {:else if sweepProgress.state === "Running"}
          <button class="btn danger" onclick={stopSweep}>Stop</button>
        {/if}
        {#if sweepProgress && (sweepProgress.state === "Completed" || sweepProgress.state === "Stopped")}
          <button class="btn" onclick={resetSweep}>Reset</button>
        {/if}
      </div>
    </div>

    {#if sweepProgress && sweepProgress.state !== "Idle"}
      <div class="card">
        <h2>Sweep Status</h2>
        <table><tbody>
          <tr><td>State</td><td>{sweepProgress.state}</td></tr>
          <tr><td>Progress</td><td>{sweepProgress.current_step} / {sweepProgress.total_steps}</td></tr>
          {#if sweepProgress.steps.length > 0}
            <tr><td>Completed Steps</td><td>{sweepProgress.steps.length}</td></tr>
            <tr><td>Best Score</td><td>{sweepProgress.best_score.toExponential(3)}</td></tr>
            <tr><td>Best Value</td><td>{sweepProgress.best_value ?? "—"}</td></tr>
          {/if}
        </tbody></table>
      </div>

      {#if sweepProgress.steps.length > 0}
        <div class="card">
          <h2>Step Results</h2>
          <div class="table-scroll">
            <table>
              <thead>
                <tr>
                  <td>Step</td>
                  <td>Value</td>
                  <td>Throughput</td>
                  <td>CPU%</td>
                  <td>WHEA</td>
                  <td>Stable</td>
                  <td>Score</td>
                </tr>
              </thead>
              <tbody>
                {#each sweepProgress.steps as r}
                  <tr class:best={r.score === sweepProgress.best_score && r.score > 0}>
                    <td>{r.step.index + 1}</td>
                    <td>{r.step.value.toFixed(1)}</td>
                    <td>{r.throughput.toExponential(3)}</td>
                    <td>{r.cpu_utilization.toFixed(1)}</td>
                    <td>{r.whea_errors}</td>
                    <td>{r.stable ? "✓" : "✗"}</td>
                    <td>{r.score.toExponential(3)}</td>
                  </tr>
                {/each}
              </tbody>
            </table>
          </div>
        </div>

        {#if sweepProgress.state === "Completed" || sweepProgress.state === "Stopped"}
          <div class="sweep-actions">
            <button class="btn primary" onclick={generateProfiles} disabled={profileLoading}>
              {profileLoading ? "Generating..." : "Generate Profiles"}
            </button>
          </div>
        {/if}
      {/if}
    {/if}

    {#if profiles}
      <h3 class="section-title">Generated Profiles</h3>
      <div class="profile-grid">
        {#each profiles.profiles as p}
          <div class="profile-card">
            <div class="profile-header">
              <span class="profile-name">{p.name}</span>
            </div>
            <p class="profile-desc">{p.notes}</p>
            <table>
              <tbody>
                <tr><td>Perf</td><td>~{p.expected_performance_pct}%</td></tr>
                <tr><td>Vcore Offset</td><td>{p.tuning.cpu_voltage_offset_mv} mV</td></tr>
                <tr><td>PL1</td><td>{p.tuning.pl1_watts} W</td></tr>
                <tr><td>PL2</td><td>{p.tuning.pl2_watts} W</td></tr>
                <tr><td>Turbo Ratio</td><td>{p.tuning.turbo_ratio_limit}x</td></tr>
                <tr><td>C-States</td><td>{p.tuning.c_states_enabled ? "Enabled" : "Disabled"}</td></tr>
              </tbody>
            </table>
          </div>
        {/each}
      </div>
    {/if}
  {/if}
</div>

<style>
  .container { max-width: 900px; margin: 0 auto; padding: 2rem; font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; color: #e0e0e0; background: #1a1a2e; min-height: 100vh; }
  header { text-align: center; margin-bottom: 1rem; }
  header h1 { margin: 0; font-size: 2rem; color: #f0c040; }
  header p { margin: 0.3rem 0 0; color: #888; font-size: 0.9rem; }
  .tabs { display: flex; gap: 0; margin-bottom: 1.5rem; border-radius: 6px; overflow: hidden; }
  .tab { flex: 1; padding: 0.6rem; background: #16213e; border: 1px solid #0f3460; color: #888; cursor: pointer; font-size: 0.95rem; }
  .tab.active { background: #0f3460; color: #f0c040; font-weight: 600; }
  .loading { text-align: center; padding: 3rem; color: #888; font-size: 1.1rem; }
  .grid { display: grid; grid-template-columns: 1fr 1fr; gap: 1rem; }
  .card { background: #16213e; border-radius: 8px; padding: 1.2rem; border: 1px solid #0f3460; }
  .card h2 { margin: 0 0 0.8rem; font-size: 1.1rem; color: #f0c040; border-bottom: 1px solid #0f3460; padding-bottom: 0.4rem; }
  .card h3 { margin: 0.5rem 0; font-size: 0.95rem; color: #aaa; }
  .card hr { border: none; border-top: 1px solid #0f3460; margin: 0.6rem 0; }
  table { width: 100%; border-collapse: collapse; font-size: 0.9rem; }
  td { padding: 0.25rem 0; }
  td:first-child { color: #888; width: 40%; }
  td:last-child { color: #e0e0e0; font-weight: 500; }
  thead td { color: #888; font-weight: 600; font-size: 0.8rem; border-bottom: 1px solid #0f3460; }
  .best td { background: rgba(240, 192, 64, 0.1); }
  .sensor-bar { display: flex; align-items: center; gap: 0.8rem; margin-bottom: 1rem; font-size: 0.9rem; }
  .sensor-label { color: #888; }
  .toggle { padding: 0.3rem 0.8rem; border-radius: 4px; border: 1px solid #0f3460; background: #16213e; color: #888; cursor: pointer; font-weight: 600; }
  .toggle.on { background: #0f3460; color: #f0c040; border-color: #f0c040; }
  .sweep-config { background: #16213e; border-radius: 8px; padding: 1.2rem; border: 1px solid #0f3460; margin-bottom: 1rem; }
  .sweep-config h3 { margin: 0 0 1rem; font-size: 1rem; color: #f0c040; }
  .field { display: flex; flex-direction: column; gap: 0.3rem; flex: 1; }
  .field span { font-size: 0.8rem; color: #888; }
  .field select, .field input { padding: 0.4rem; border-radius: 4px; border: 1px solid #0f3460; background: #1a1a2e; color: #e0e0e0; font-size: 0.9rem; }
  .row { display: flex; gap: 0.8rem; margin-bottom: 0.8rem; }
  .sweep-actions { display: flex; gap: 0.5rem; margin-top: 1rem; }
  .btn { padding: 0.4rem 1.2rem; background: #0f3460; color: #f0c040; border: 1px solid #f0c040; border-radius: 4px; cursor: pointer; font-size: 0.85rem; }
  .btn:hover { background: #1a5276; }
  .btn.primary { background: #f0c040; color: #1a1a2e; font-weight: 600; }
  .btn.primary:hover { background: #f4d060; }
  .btn.danger { background: #e94560; color: #fff; border-color: #e94560; }
  .btn.danger:hover { background: #ff6b81; }
  .btn:disabled { opacity: 0.5; cursor: default; }
  .table-scroll { max-height: 300px; overflow-y: auto; }
  .table-scroll td { padding: 0.25rem 0.5rem; }
  .section-title { margin: 1.5rem 0 0.8rem; font-size: 1.1rem; color: #f0c040; }
  .whea-card { grid-column: 1 / -1; }
  .whea-list { margin-top: 0.8rem; max-height: 260px; overflow-y: auto; display: flex; flex-direction: column; gap: 0.4rem; }
  .whea-event { background: rgba(233, 69, 96, 0.08); border: 1px solid rgba(233, 69, 96, 0.25); border-radius: 4px; padding: 0.5rem 0.7rem; }
  .whea-meta { display: flex; gap: 0.6rem; align-items: center; margin-bottom: 0.25rem; font-size: 0.75rem; }
  .whea-idx { color: #e94560; font-weight: 700; }
  .whea-id { background: #0f3460; color: #f0c040; padding: 0.1rem 0.4rem; border-radius: 3px; font-weight: 600; }
  .whea-time { color: #888; }
  .whea-desc { font-size: 0.82rem; color: #e0e0e0; line-height: 1.4; }
  .whea-ok { margin: 0.5rem 0 0; font-size: 0.85rem; color: #4caf50; }
  .profile-grid { display: grid; grid-template-columns: 1fr 1fr 1fr; gap: 1rem; }
  .profile-card { background: #16213e; border-radius: 8px; padding: 1.2rem; border: 1px solid #0f3460; }
  .profile-header { margin-bottom: 0.5rem; }
  .profile-name { font-size: 1rem; font-weight: 600; color: #f0c040; }
  .profile-desc { font-size: 0.8rem; color: #888; margin: 0 0 0.8rem; line-height: 1.4; }
  @media (max-width: 700px) { .profile-grid { grid-template-columns: 1fr; } }
  @media (max-width: 600px) { .grid { grid-template-columns: 1fr; } }
</style>
