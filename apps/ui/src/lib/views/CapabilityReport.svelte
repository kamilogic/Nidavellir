<script>
  import { serviceCall } from "../service.js";

  let report = $state(null);
  let hw = $state(null);
  let loading = $state(true);
  let error = $state(null);

  async function load() {
    loading = true;
    error = null;
    try {
      const [capResp, hwResp] = await Promise.all([
        serviceCall("GetCapabilityReport"),
        serviceCall("DetectHardware"),
      ]);
      report = capResp?.data?.type === "Capability" ? capResp.data : null;
      hw = hwResp?.data?.type === "Hardware" ? hwResp.data : null;
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    load();
  });

  function ramGb(totalMb) {
    if (totalMb == null) return "—";
    return (Number(totalMb) / 1024).toFixed(0);
  }

  function gpuPrimary(gpus) {
    if (!gpus?.length) return null;
    return gpus[0];
  }

  function xmpLabel(enabled) {
    return enabled ? "XMP ligado" : "XMP desligado";
  }

  function ramRated(ram) {
    const rated = ram?.rated_speed_mts;
    return rated ? `${rated} MT/s` : "—";
  }

  function formatGain(text) {
    if (!text) return null;
    const t = String(text).trim();
    if (t.startsWith("↑") || t.startsWith("↓")) return t;
    return `↑ ${t}`;
  }

  function guideUrl(title) {
    const q = encodeURIComponent(`${title} BIOS como ativar`);
    return `https://duckduckgo.com/?q=${q}`;
  }

  function iconKind(id) {
    if (!id) return "chip";
    if (id.includes("gpu")) return "gpu";
    if (id === "enable_xmp" || id.includes("timing")) return "ram";
    if (id.includes("cpu_power")) return "bolt";
    if (id.includes("turbo") || id.includes("cstate")) return "chip";
    if (id.includes("undervolt")) return "cpu";
    return "chip";
  }

  function versionBadge(id) {
    if (id === "ram_timings_runtime") return "v0.8+";
    return null;
  }
</script>

<section class="forge">
  {#if loading}
    <p class="state">Analisando seu hardware…</p>
  {:else if error}
    <p class="state state--err">{error}</p>
  {:else if report && hw}
    <div class="hw-row">
      <article class="hw-card">
        <span class="hw-label">CPU</span>
        <p class="hw-title">{hw.cpu.model}</p>
        <p class="hw-meta">{hw.cpu.cores}C / {hw.cpu.threads}T · base {hw.cpu.base_freq_mhz} MHz</p>
        <p class="hw-meta">máx teórico do silício: {hw.cpu.max_freq_mhz} MHz</p>
      </article>
      <article class="hw-card">
        <span class="hw-label">GPU</span>
        {#if gpuPrimary(hw.gpu)}
          <p class="hw-title">{gpuPrimary(hw.gpu).model}</p>
          <p class="hw-meta">{Math.round(gpuPrimary(hw.gpu).vram_mb / 1024)} GB VRAM</p>
          <p class="hw-meta">
            core máx {gpuPrimary(hw.gpu).max_core_clock_mhz ?? "—"} MHz · mem máx
            {gpuPrimary(hw.gpu).max_memory_clock_mhz ?? "—"} MHz
          </p>
        {:else}
          <p class="hw-title">—</p>
          <p class="hw-meta">Não detectada</p>
        {/if}
      </article>
      <article class="hw-card">
        <span class="hw-label">RAM</span>
        <p class="hw-title">{ramGb(hw.ram.total_mb)} GB</p>
        <p class="hw-meta">configurada {hw.ram.configured_speed_mts} MT/s</p>
        <p class="hw-meta">máx teórico do módulo: {ramRated(hw.ram)}</p>
        <p class="hw-meta">{xmpLabel(hw.ram.xmp_enabled)}</p>
      </article>
      <article class="hw-card">
        <span class="hw-label">Motherboard</span>
        <p class="hw-title">{hw.motherboard.vendor} {hw.motherboard.model}</p>
        <p class="hw-meta">BIOS {hw.motherboard.bios_version}</p>
      </article>
    </div>

    <div class="summary-row">
      <div class="summary-stat summary-stat--ok">
        <span class="summary-num">{report.automatic.length}</span>
        <span class="summary-label">automático</span>
      </div>
      <div class="summary-stat summary-stat--warn">
        <span class="summary-num">{report.needs_action.length}</span>
        <span class="summary-label">precisa da sua ação</span>
      </div>
      <div class="summary-stat summary-stat--muted">
        <span class="summary-num">{report.blocked.length}</span>
        <span class="summary-label">bloqueado por hardware</span>
      </div>
    </div>

    <div class="section section--auto">
      <h2 class="section-head">
        <span class="dot dot--ok"></span>
        Automático — o Nidavellir cuida sozinho
      </h2>
      {#each report.automatic as item (item.id)}
        <article class="feat feat--auto">
          <div class="feat-icon" aria-hidden="true">
            {#if iconKind(item.id) === "gpu"}
              <svg viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="currentColor" stroke-width="1.6">
                <rect x="2" y="5" width="20" height="14" rx="2" />
                <path d="M8 19h8M12 5v-2" />
              </svg>
            {:else if iconKind(item.id) === "cpu"}
              <svg viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="currentColor" stroke-width="1.6">
                <rect x="7" y="7" width="10" height="10" rx="1" />
                <path d="M7 3v4M17 3v4M7 17v4M17 17v4M3 7h4M3 17h4M17 7h4M17 17h4" />
              </svg>
            {:else if iconKind(item.id) === "bolt"}
              <svg viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="currentColor" stroke-width="1.6">
                <path d="M13 2L3 14h9l-1 8 10-12h-9l1-8z" />
              </svg>
            {:else if iconKind(item.id) === "ram"}
              <svg viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="currentColor" stroke-width="1.6">
                <path d="M4 8h16v8H4zM7 8v8M10 8v8M13 8v8M16 8v8" />
              </svg>
            {:else}
              <svg viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="currentColor" stroke-width="1.6">
                <path d="M9 3h6v4H9zM5 9h14v12H5zM9 9V7M15 9V7" />
              </svg>
            {/if}
          </div>
          <div class="feat-body">
            <h3 class="feat-title">{item.title}</h3>
            <p class="feat-desc">{item.description}</p>
            {#if item.estimated_gain}
              <p class="feat-gain feat-gain--ok">{formatGain(item.estimated_gain)}</p>
            {/if}
          </div>
          <span class="pill pill--ok">disponível</span>
        </article>
      {:else}
        <p class="empty">Nada listado nesta categoria.</p>
      {/each}
    </div>

    <div class="section section--action">
      <h2 class="section-head">
        <span class="dot dot--warn"></span>
        Precisa da sua ação — uma mudança na BIOS libera isso
      </h2>
      {#each report.needs_action as item (item.id)}
        <article class="feat feat--action">
          <div class="feat-icon feat-icon--warn" aria-hidden="true">
            <svg viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="currentColor" stroke-width="1.6">
              <circle cx="12" cy="12" r="9" />
              <path d="M12 8v5M12 16h.01" />
            </svg>
          </div>
          <div class="feat-body">
            <h3 class="feat-title">{item.title}</h3>
            <p class="feat-desc">{item.description}</p>
            {#if item.estimated_gain}
              <p class="feat-gain feat-gain--warn">{formatGain(item.estimated_gain)}</p>
            {/if}
            <a class="guide" href={guideUrl(item.title)} target="_blank" rel="noopener noreferrer">como fazer ↗</a>
          </div>
          <span class="pill pill--warn">ação simples</span>
        </article>
      {:else}
        <p class="empty">Sem passos manuais obrigatórios.</p>
      {/each}
    </div>

    <div class="section section--blocked">
      <h2 class="section-head">
        <span class="dot dot--muted"></span>
        Bloqueado por hardware — o que é possível dentro desses limites
      </h2>
      {#each report.blocked as item (item.id)}
        <article class="feat feat--blocked">
          <div class="feat-icon feat-icon--lock" aria-hidden="true">
            <svg viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="currentColor" stroke-width="1.6">
              <rect x="5" y="11" width="14" height="10" rx="2" />
              <path d="M8 11V8a4 4 0 0 1 8 0v3" />
            </svg>
          </div>
          <div class="feat-body">
            <h3 class="feat-title">{item.title}</h3>
            <p class="feat-desc">{item.description}</p>
          </div>
          <div class="feat-right">
            {#if versionBadge(item.id)}
              <span class="ver">{versionBadge(item.id)}</span>
            {/if}
            <span class="pill pill--blocked">bloqueado</span>
          </div>
        </article>
      {:else}
        <p class="empty">Sem bloqueios reportados.</p>
      {/each}
    </div>

    {#if report.fingerprint}
      <footer class="foot">
        <span>ID da máquina</span>
        <code>{report.fingerprint.hash.slice(0, 20)}…</code>
        <span class="foot-sep">·</span>
        <span class="foot-probe">Passo 2 (driver): {report.probe_pass2_status}</span>
      </footer>
    {/if}

    <div class="scroll-hint" aria-hidden="true">
      <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
        <path d="M6 9l6 6 6-6" />
      </svg>
    </div>
  {/if}
</section>

<style>
  .forge {
    --bg-deep: var(--nord-night);
    --surface: rgba(19, 31, 46, 0.82);
    --surface-2: rgba(26, 40, 57, 0.82);
    --border: var(--nord-border-card);
    --text: var(--nord-silver);
    --muted: var(--nord-mist);
    --dim: var(--nord-dim);
    --forge-green: var(--nord-aurora);
    --forge-green-dim: #5a8f00;
    --forge-orange: var(--nord-ember);
    --forge-orange-bg: rgba(232, 162, 58, 0.12);
    --forge-green-bg: rgba(118, 185, 0, 0.1);
    display: flex;
    flex-direction: column;
    gap: 1.75rem;
  }

  .state {
    color: var(--muted);
    padding: 2rem 0;
    text-align: center;
  }
  .state--err {
    color: #fca5a5;
  }

  .hw-row {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 0.85rem;
  }
  @media (max-width: 1020px) {
    .hw-row {
      grid-template-columns: repeat(2, 1fr);
    }
  }
  @media (max-width: 520px) {
    .hw-row {
      grid-template-columns: 1fr;
    }
  }

  .hw-card {
    background: var(--surface);
    border: 1px solid #3f3f3f;
    border-radius: 12px;
    padding: 1rem 1.1rem;
    min-height: 5.5rem;
  }
  .hw-label {
    display: block;
    font-size: 0.7rem;
    font-weight: 700;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: var(--dim);
    margin-bottom: 0.35rem;
  }
  .hw-title {
    margin: 0;
    font-size: 0.95rem;
    font-weight: 600;
    color: var(--text);
    line-height: 1.35;
  }
  .hw-meta {
    margin: 0.35rem 0 0;
    font-size: 0.8rem;
    color: var(--muted);
    line-height: 1.35;
  }

  .summary-row {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 0.85rem;
    padding: 0;
    margin: 0;
    background: none;
    border: none;
    box-shadow: none;
    outline: none;
  }
  @media (max-width: 640px) {
    .summary-row {
      grid-template-columns: 1fr;
    }
  }

  .summary-stat {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    text-align: center;
    gap: 0.4rem;
    min-width: 0;
    min-height: 5.5rem;
    padding: 1.25rem 0.75rem;
    background: var(--surface);
    border: none;
    border-radius: 12px;
    box-shadow: none;
    outline: none;
  }

  .summary-num {
    font-size: 2.5rem;
    font-weight: 800;
    line-height: 1;
    font-variant-numeric: tabular-nums;
  }
  .summary-stat--ok .summary-num {
    color: #4ade80;
  }
  .summary-stat--warn .summary-num {
    color: #fbbf24;
  }
  .summary-stat--muted .summary-num {
    color: var(--text);
  }
  .summary-label {
    font-size: 0.875rem;
    color: #9ca3af;
    max-width: 13rem;
    line-height: 1.35;
    font-weight: 500;
  }

  .section {
    display: flex;
    flex-direction: column;
    gap: 0.65rem;
  }
  .section-head {
    margin: 0 0 0.35rem;
    font-size: 0.72rem;
    font-weight: 700;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--muted);
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
  }
  .dot--ok {
    background: var(--forge-green);
    box-shadow: 0 0 10px rgba(118, 185, 0, 0.45);
  }
  .dot--warn {
    background: var(--forge-orange);
  }
  .dot--muted {
    background: #555;
  }

  .feat {
    display: grid;
    grid-template-columns: 48px 1fr auto;
    align-items: center;
    gap: 1rem;
    padding: 1rem 1.15rem;
    border-radius: 12px;
    border: 1px solid var(--border);
    background: var(--surface);
  }
  .feat--auto {
    background: linear-gradient(135deg, var(--surface) 0%, #1a2214 100%);
    border-color: #2d3f1f;
  }
  .feat--action {
    background: linear-gradient(135deg, var(--surface) 0%, #241c12 100%);
    border-color: #4a3820;
  }
  .feat--blocked {
    opacity: 0.88;
    background: var(--surface-2);
    border-color: #3a3a3a;
  }

  .feat-icon {
    width: 48px;
    height: 48px;
    border-radius: 10px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: #2a2a2a;
    color: var(--forge-green);
  }
  .feat-icon--warn {
    color: var(--forge-orange);
    background: var(--forge-orange-bg);
  }
  .feat-icon--lock {
    color: #888;
    background: #2a2a2a;
  }

  .feat-body {
    min-width: 0;
  }
  .feat-title {
    margin: 0;
    font-size: 1rem;
    font-weight: 700;
    color: var(--text);
  }
  .feat-desc {
    margin: 0.25rem 0 0;
    font-size: 0.85rem;
    color: var(--muted);
    line-height: 1.45;
  }
  .feat-gain {
    margin: 0.45rem 0 0;
    font-size: 0.8rem;
    font-weight: 600;
  }
  .feat-gain--ok {
    color: var(--forge-green);
  }
  .feat-gain--warn {
    color: #fbbf24;
  }

  .guide {
    display: inline-block;
    margin-top: 0.55rem;
    font-size: 0.8rem;
    font-weight: 600;
    color: var(--forge-orange);
    text-decoration: none;
    border-bottom: 1px solid rgba(232, 162, 58, 0.35);
  }
  .guide:hover {
    color: #f5c56a;
    border-bottom-color: var(--forge-orange);
  }

  .feat-right {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 0.35rem;
  }

  .pill {
    font-size: 0.68rem;
    font-weight: 700;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    padding: 0.35rem 0.65rem;
    border-radius: 999px;
    white-space: nowrap;
  }
  .pill--ok {
    background: rgba(118, 185, 0, 0.2);
    color: var(--forge-green);
    border: 1px solid rgba(118, 185, 0, 0.45);
  }
  .pill--warn {
    background: var(--forge-orange-bg);
    color: #fcd34d;
    border: 1px solid rgba(232, 162, 58, 0.35);
  }
  .pill--blocked {
    background: #333;
    color: #9ca3af;
    border: 1px solid #444;
  }

  .ver {
    font-size: 0.65rem;
    font-weight: 700;
    color: var(--dim);
    letter-spacing: 0.04em;
  }

  .empty {
    color: var(--dim);
    font-size: 0.9rem;
    margin: 0.25rem 0 0.5rem;
  }

  .foot {
    font-size: 0.75rem;
    color: var(--dim);
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.35rem;
    padding-top: 0.5rem;
    border-top: 1px solid var(--border);
  }
  .foot code {
    background: #0f0f0f;
    padding: 0.15rem 0.4rem;
    border-radius: 4px;
    color: var(--muted);
  }
  .foot-sep {
    opacity: 0.5;
  }
  .foot-probe {
    color: #555;
  }

  .scroll-hint {
    display: flex;
    justify-content: center;
    padding: 0.25rem 0 0.5rem;
    color: #444;
  }
  @media (max-width: 640px) {
    .feat {
      grid-template-columns: 44px 1fr;
      grid-template-rows: auto auto;
    }
    .feat-icon {
      grid-row: 1;
    }
    .feat-body {
      grid-column: 2;
    }
    .pill,
    .feat-right {
      grid-column: 1 / -1;
      justify-self: end;
    }
  }
</style>

