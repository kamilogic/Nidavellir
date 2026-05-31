<script>
  import { serviceCall } from "../service.js";

  let progress = $state(null);
  let error = $state(null);
  let timer = $state(null);
  let realCurve = $state(null);
  let validation = $state(null);

  const VAL_LABEL = {
    stable: "Estável (0 erros)",
    silent_error: "ERRO SILENCIOSO detectado",
    crash: "Crash / driver perdido",
  };

  const PHASE_LABEL = {
    idle: "Ocioso",
    baseline: "Baseline (equilíbrio térmico)",
    vram_diagnostic: "Diagnóstico de VRAM",
    voltage_bisection: "Bisseção de voltagem",
    synthesis: "Síntese dos perfis",
    done: "Concluído",
    aborted: "Abortado",
  };

  const running = $derived(
    progress &&
      ["baseline", "vram_diagnostic", "voltage_bisection", "synthesis"].includes(progress.phase),
  );

  function phaseLabel(p) {
    return PHASE_LABEL[p] ?? p ?? "—";
  }

  function captureProgress(r) {
    progress = r?.data?.type === "GpuSweep" ? r.data : progress;
  }

  async function refresh() {
    try {
      captureProgress(await serviceCall("GetGpuSweepProgress"));
      const v = await serviceCall("GetGpuValidation");
      validation = v?.data?.type === "GpuValidation" ? v.data : validation;
      error = null;
    } catch (e) {
      error = String(e);
    }
  }

  async function readRealCurve() {
    try {
      const r = await serviceCall("GetGpuCurve");
      realCurve = r?.data?.type === "GpuCurve" ? r.data : realCurve;
      error = null;
    } catch (e) {
      error = String(e);
    }
  }

  async function startValidation() {
    try {
      const r = await serviceCall("StartGpuValidation");
      validation = r?.data?.type === "GpuValidation" ? r.data : validation;
    } catch (e) {
      error = String(e);
    }
  }

  async function start() {
    try {
      captureProgress(await serviceCall("StartGpuSweep"));
      error = null;
    } catch (e) {
      error = String(e);
    }
  }

  async function stop() {
    try {
      captureProgress(await serviceCall("StopGpuSweep"));
    } catch (e) {
      error = String(e);
    }
  }

  $effect(() => {
    refresh();
    timer = setInterval(refresh, 500);
    return () => clearInterval(timer);
  });
</script>

<section class="forge">
  <header class="forge-head">
    <div>
      <h2>Forja — Sweep de GPU</h2>
      <p class="lead">
        Mapeia a voltagem mínima estável por frequência via bisseção da fronteira,
        detectando erros computacionais silenciosos (não só crashes) e sintetiza os
        três perfis. Cada passo passa pelo Safe Loop.
      </p>
    </div>
    <div class="actions">
      {#if progress?.simulated}
        <span class="badge sim">simulado</span>
      {/if}
      {#if running}
        <button class="btn stop" onclick={stop}>Parar</button>
      {:else}
        <button class="btn go" onclick={start}>Iniciar sweep</button>
      {/if}
    </div>
  </header>

  {#if progress?.simulated}
    <p class="note">
      Backend simulado: a engine roda de ponta a ponta sem escrever na GPU. A escrita
      real de curva V/F (NVAPI) entra num incremento futuro — nenhum offset é aplicado
      ao hardware agora.
    </p>
  {/if}

  {#if error}
    <p class="err">{error}</p>
  {/if}

  {#if progress}
    <div class="grid">
      <article class="tile">
        <span class="lab">Fase</span>
        <p class="val">{phaseLabel(progress.phase)}</p>
      </article>
      <article class="tile">
        <span class="lab">Frequência</span>
        <p class="val">{progress.freq_index} / {progress.total_freqs}</p>
      </article>
      <article class="tile">
        <span class="lab">Testando agora</span>
        <p class="val">
          {#if progress.current}
            {progress.current.freq_mhz} MHz @ {progress.current.voltage_mv} mV
          {:else}
            —
          {/if}
        </p>
      </article>
    </div>

    {#if progress.tradeoffs?.length}
      <div class="section">
        <h3 class="section-head">Mapa freq × voltagem mínima</h3>
        <ul class="list">
          {#each progress.tradeoffs as t}
            <li>
              <span class="mono">{t.freq_mhz} MHz</span>
              <span class="mono accent">{t.vmin_mv} mV</span>
            </li>
          {/each}
        </ul>
      </div>
    {/if}

    {#if progress.profiles}
      <div class="section">
        <h3 class="section-head">Perfis sintetizados</h3>
        <div class="profiles">
          {#each [progress.profiles.godforge, progress.profiles.brokkrs_best, progress.profiles.deep_calm] as prof}
            <article class="profile">
              <h4>{prof.name}</h4>
              <p class="desc">{prof.description}</p>
              <p class="point">{prof.point.freq_mhz} MHz @ {prof.point.voltage_mv} mV</p>
            </article>
          {/each}
        </div>
      </div>
    {/if}
  {:else}
    <p class="wait">Aguardando o serviço…</p>
  {/if}

  <div class="section real">
    <h3 class="section-head">Comparação real (NVAPI) — sua GPU de verdade</h3>
    <div class="real-actions">
      <button class="btn" onclick={readRealCurve}>Ler curva real</button>
      <button class="btn go" onclick={startValidation} disabled={validation?.running}>
        {validation?.running ? "Validando…" : "Validar estabilidade (real)"}
      </button>
    </div>

    {#if realCurve}
      {#if realCurve.real}
        <p class="sub">{realCurve.name} · {realCurve.points.length} pontos na curva</p>
        {#if realCurve.plateau}
          <p class="point accent">
            Plateau (UV travado): {realCurve.plateau.freq_mhz} MHz @ {realCurve.plateau.voltage_mv} mV
          </p>
        {/if}
        <ul class="list">
          {#each realCurve.points.filter((_, i) => i % 8 === 0) as p}
            <li><span class="mono">{p.voltage_mv} mV</span><span class="mono accent">{p.freq_mhz} MHz</span></li>
          {/each}
        </ul>
      {:else}
        <p class="err">{realCurve.name}</p>
      {/if}
    {/if}

    {#if validation}
      <div class="val">
        {#if validation.running}
          <p class="sub">Rodando known-answer test na GPU… (detecta erro silencioso sem crash)</p>
        {:else if validation.result}
          <p class="point" class:danger={validation.result !== "stable"} class:accent={validation.result === "stable"}>
            {VAL_LABEL[validation.result] ?? validation.result}
          </p>
          <p class="sub">
            mismatches: {validation.mismatches} · {validation.elapsed_ms} ms
            {#if validation.adapter}· {validation.adapter}{/if}
          </p>
        {:else if validation.adapter}
          <p class="err">{validation.adapter}</p>
        {/if}
      </div>
    {/if}
  </div>
</section>

<style>
  .forge {
    --surface: rgba(19, 31, 46, 0.82);
    --border: var(--nord-border-card);
    --muted: var(--nord-mist);
    --text: var(--nord-silver);
    --accent: var(--nord-aurora);
    display: flex;
    flex-direction: column;
    gap: 1.25rem;
  }
  .forge-head {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    gap: 1rem;
  }
  .forge-head h2 {
    margin: 0 0 0.5rem;
    font-size: 0.85rem;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--muted);
  }
  .lead {
    margin: 0;
    font-size: 0.88rem;
    line-height: 1.5;
    color: var(--muted);
    max-width: 60ch;
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    flex-shrink: 0;
  }
  .btn {
    border: 1px solid var(--border);
    border-radius: 9px;
    padding: 0.55rem 1.1rem;
    font-weight: 600;
    font-size: 0.85rem;
    cursor: pointer;
  }
  .btn.go {
    background: rgba(163, 190, 140, 0.16);
    color: var(--accent);
    border-color: rgba(163, 190, 140, 0.45);
  }
  .btn.stop {
    background: rgba(191, 97, 106, 0.16);
    color: #f3b9bd;
    border-color: rgba(191, 97, 106, 0.45);
  }
  .badge.sim {
    font-size: 0.68rem;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    padding: 0.3rem 0.6rem;
    border-radius: 999px;
    background: rgba(232, 162, 58, 0.14);
    color: var(--nord-ember-bright);
    border: 1px solid rgba(232, 162, 58, 0.4);
  }
  .note {
    margin: 0;
    font-size: 0.8rem;
    line-height: 1.5;
    color: var(--nord-ember-bright);
    background: rgba(232, 162, 58, 0.08);
    border: 1px solid rgba(232, 162, 58, 0.25);
    border-radius: 10px;
    padding: 0.7rem 0.9rem;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
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
  .section-head {
    margin: 0 0 0.5rem;
    font-size: 0.72rem;
    font-weight: 700;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--muted);
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(150px, 1fr));
    gap: 0.4rem;
  }
  .list li {
    display: flex;
    justify-content: space-between;
    gap: 0.75rem;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 9px;
    padding: 0.5rem 0.75rem;
    font-size: 0.82rem;
  }
  .mono {
    font-variant-numeric: tabular-nums;
    color: var(--text);
  }
  .mono.accent {
    color: var(--accent);
  }
  .profiles {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
    gap: 0.85rem;
  }
  .profile {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 12px;
    padding: 1rem 1.1rem;
  }
  .profile h4 {
    margin: 0 0 0.35rem;
    font-family: Cinzel, "Palatino Linotype", serif;
    letter-spacing: 0.06em;
    color: var(--accent);
  }
  .desc {
    margin: 0 0 0.6rem;
    font-size: 0.8rem;
    color: var(--muted);
    line-height: 1.45;
  }
  .point {
    margin: 0;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    color: var(--text);
  }
  .wait,
  .err {
    color: var(--nord-dim);
    font-size: 0.9rem;
  }
  .err {
    color: var(--nord-danger);
  }
  .real {
    border-top: 1px solid var(--border);
    padding-top: 1rem;
  }
  .real-actions {
    display: flex;
    gap: 0.6rem;
    margin-bottom: 0.75rem;
    flex-wrap: wrap;
  }
  .btn:disabled {
    opacity: 0.55;
    cursor: default;
  }
  .point.accent {
    color: var(--accent);
  }
  .point.danger {
    color: var(--nord-danger);
  }
  .val {
    margin-top: 0.6rem;
  }
</style>
