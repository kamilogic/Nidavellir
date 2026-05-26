<script>
  import { invoke } from "@tauri-apps/api/core";

  let loading = $state(true);
  let error = $state(null);
  let data = $state(null);

  async function load() {
    loading = true;
    error = null;
    try {
      data = await invoke("detect_hardware");
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => { load(); });
</script>

<div class="container">
  <header>
    <h1>Nidavellir ⚒️</h1>
    <p>Forging your silicon to its maximum potential</p>
  </header>

  {#if loading}
    <div class="loading">Detecting hardware...</div>
  {:else if error}
    <div class="card error">
      <strong>Error:</strong> {error}
      <button onclick={load}>Retry</button>
    </div>
  {:else if data}
    <div class="grid">
      <div class="card">
        <h2>CPU</h2>
        <table>
          <tbody>
            <tr><td>Vendor</td><td>{data.cpu.vendor}</td></tr>
            <tr><td>Model</td><td>{data.cpu.model}</td></tr>
            <tr><td>Cores</td><td>{data.cpu.cores}</td></tr>
            <tr><td>Threads</td><td>{data.cpu.threads}</td></tr>
            <tr><td>Base Freq</td><td>{data.cpu.base_freq_mhz} MHz</td></tr>
            <tr><td>Max Freq</td><td>{data.cpu.max_freq_mhz} MHz</td></tr>
          </tbody>
        </table>
      </div>

      <div class="card">
        <h2>GPU</h2>
        {#each data.gpu as gpu, i}
          <table>
            <tbody>
              <tr><td>Vendor</td><td>{gpu.vendor}</td></tr>
              <tr><td>Model</td><td>{gpu.model}</td></tr>
              <tr><td>VRAM</td><td>{gpu.vram_mb} MB</td></tr>
            </tbody>
          </table>
          {#if i < data.gpu.length - 1}<hr>{/if}
        {/each}
      </div>

      <div class="card">
        <h2>RAM</h2>
        <table>
          <tbody>
            <tr><td>Total</td><td>{(data.ram.total_mb / 1024).toFixed(0)} GB</td></tr>
          </tbody>
        </table>
        {#if data.ram.modules.length > 0}
          <hr>
          <h3>Modules</h3>
          {#each data.ram.modules as mod, i}
            <table>
              <tbody>
                <tr><td>Size</td><td>{mod.size_mb} MB</td></tr>
                <tr><td>Speed</td><td>{mod.speed_mts} MT/s</td></tr>
              </tbody>
            </table>
            {#if i < data.ram.modules.length - 1}<hr>{/if}
          {/each}
        {/if}
      </div>

      <div class="card">
        <h2>Motherboard</h2>
        <table>
          <tbody>
            <tr><td>Vendor</td><td>{data.motherboard.vendor}</td></tr>
            <tr><td>Model</td><td>{data.motherboard.model}</td></tr>
            <tr><td>BIOS</td><td>{data.motherboard.bios_version}</td></tr>
          </tbody>
        </table>
      </div>
    </div>

    <button class="refresh" onclick={load}>Refresh</button>
  {/if}
</div>

<style>
  .container { max-width: 900px; margin: 0 auto; padding: 2rem; font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; color: #e0e0e0; background: #1a1a2e; min-height: 100vh; }
  header { text-align: center; margin-bottom: 2rem; }
  header h1 { margin: 0; font-size: 2rem; color: #f0c040; }
  header p { margin: 0.3rem 0 0; color: #888; font-size: 0.9rem; }
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
  .error { text-align: center; padding: 2rem; }
  .error button { margin-top: 1rem; padding: 0.4rem 1.2rem; background: #e94560; color: #fff; border: none; border-radius: 4px; cursor: pointer; }
  .refresh { display: block; margin: 1.5rem auto 0; padding: 0.5rem 2rem; background: #0f3460; color: #f0c040; border: 1px solid #f0c040; border-radius: 4px; cursor: pointer; font-size: 0.9rem; }
  .refresh:hover { background: #1a5276; }
  @media (max-width: 600px) { .grid { grid-template-columns: 1fr; } }
</style>
