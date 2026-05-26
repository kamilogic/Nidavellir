<script>
  import { invoke } from "@tauri-apps/api/core";

  let hwData = $state(null);
  let sensorData = $state(null);
  let hwLoading = $state(true);
  let sensorLoading = $state(true);
  let error = $state(null);
  let activeTab = $state("hardware");
  let autoRefresh = $state(false);
  let refreshInterval = $state(null);

  async function loadHw() {
    hwLoading = true;
    try { hwData = await invoke("detect_hardware"); }
    catch (e) { error = String(e); }
    finally { hwLoading = false; }
  }

  async function loadSensors() {
    sensorLoading = true;
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

  $effect(() => {
    loadHw();
    loadSensors();
    return () => { if (refreshInterval) clearInterval(refreshInterval); };
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
  {:else}
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
          </tbody></table>
        </div>
        <div class="card">
          <h2>Memory</h2>
          <table><tbody>
            <tr><td>Used</td><td>{sensorData.memory.used_mb} MB / {sensorData.memory.total_mb} MB</td></tr>
            <tr><td>Usage</td><td>{sensorData.memory.used_pct.toFixed(1)}%</td></tr>
          </tbody></table>
        </div>
        <div class="card">
          <h2>WHEA</h2>
          <table><tbody>
            <tr><td>Errors</td><td>{sensorData.whea.error_count}</td></tr>
          </tbody></table>
        </div>
        <div class="card">
          <h2>Boot Status</h2>
          <table><tbody>
            <tr><td>Previous crash</td><td>{sensorData.boot_status.previous_boot_crashed ? "Yes ⚠️" : "No ✅"}</td></tr>
          </tbody></table>
        </div>
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
  .sensor-bar { display: flex; align-items: center; gap: 0.8rem; margin-bottom: 1rem; font-size: 0.9rem; }
  .sensor-label { color: #888; }
  .toggle { padding: 0.3rem 0.8rem; border-radius: 4px; border: 1px solid #0f3460; background: #16213e; color: #888; cursor: pointer; font-weight: 600; }
  .toggle.on { background: #0f3460; color: #f0c040; border-color: #f0c040; }
  .btn { padding: 0.4rem 1.2rem; background: #0f3460; color: #f0c040; border: 1px solid #f0c040; border-radius: 4px; cursor: pointer; font-size: 0.85rem; }
  .btn:hover { background: #1a5276; }
  @media (max-width: 600px) { .grid { grid-template-columns: 1fr; } }
</style>
