<script>
  import { serviceCall } from "../service.js";

  let status = $state(null);
  let error = $state(null);
  let timer = $state(null);

  const STATE_LABEL = {
    idle: "Ocioso",
    probing: "Sondando",
    applying: "Aplicando",
    dwell: "Em observação (dwell)",
    validated: "Validado",
    unstable: "Instável",
    safe_mode: "Modo Seguro",
  };

  const CRASH_LABEL = {
    oc_instability: "Instabilidade de OC",
    unrelated: "Não relacionado",
    unknown: "Desconhecido",
  };

  function stateLabel(s) {
    return STATE_LABEL[s] ?? s ?? "—";
  }

  function pointText(p) {
    if (!p || !p.axes || Object.keys(p.axes).length === 0) return "stock (sem offsets)";
    return Object.entries(p.axes)
      .map(([k, v]) => `${k}: ${v > 0 ? "+" : ""}${v}`)
      .join(" · ");
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
    <h2>Central de Segurança</h2>
    <p class="lead">
      O Safe Loop é o paraquedas do Nidavellir: antes de cada ajuste ele grava uma
      boot-flag em disco e só a limpa após a validação. Se a máquina travar, no
      próximo boot o serviço detecta a flag armada, isola a região instável e
      recua para o último perfil estável.
    </p>
  </header>

  {#if error}
    <p class="err">{error}</p>
  {:else if status}
    {#if status.safe_mode}
      <div class="alert">
        <strong>Modo Seguro ativo.</strong> Após {status.crash_threshold} travamentos
        seguidos, o Nidavellir aplicou o perfil stock e parou de mexer no hardware.
      </div>
    {:else if status.boot_flag_armed}
      <div class="alert alert--warn">
        <strong>Boot-flag armada.</strong> Um ajuste está em validação — se houver
        travamento, a recuperação age no próximo boot.
      </div>
    {/if}

    <div class="grid">
      <article class="tile">
        <span class="lab">Estado</span>
        <p class="val">{stateLabel(status.state)}</p>
      </article>
      <article class="tile">
        <span class="lab">Travamentos seguidos</span>
        <p class="val" class:danger={status.consecutive_crashes > 0}>
          {status.consecutive_crashes} / {status.crash_threshold}
        </p>
        <p class="sub">limite para Modo Seguro</p>
      </article>
      <article class="tile">
        <span class="lab">Boot-flag</span>
        <p class="val">{status.boot_flag_armed ? "Armada" : "Limpa"}</p>
      </article>
      <article class="tile">
        <span class="lab">Último perfil validado</span>
        <p class="val small">{pointText(status.last_validated)}</p>
      </article>
    </div>

    <div class="section">
      <h3 class="section-head">Regiões em blacklist ({status.blacklist.length})</h3>
      {#if status.blacklist.length}
        <ul class="list">
          {#each status.blacklist as region}
            <li>
              <span class="mono">{pointText(region.center)}</span>
              <span class="dim">raio ±{region.radius}</span>
            </li>
          {/each}
        </ul>
      {:else}
        <p class="empty">Nenhuma região instável registrada.</p>
      {/if}
    </div>

    {#if status.recent_crashes?.length}
      <div class="section">
        <h3 class="section-head">Travamentos recentes</h3>
        <ul class="list">
          {#each status.recent_crashes as c}
            <li><span class="mono">{CRASH_LABEL[c] ?? c}</span></li>
          {/each}
        </ul>
      </div>
    {/if}
  {:else}
    <p class="wait">Aguardando o serviço…</p>
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
    gap: 1.25rem;
  }
  .safe-head h2 {
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
  .alert {
    border-radius: 12px;
    padding: 0.85rem 1.1rem;
    font-size: 0.88rem;
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
    font-size: 1.05rem;
  }
  .val.small {
    font-size: 0.82rem;
    font-weight: 500;
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
    font-weight: 700;
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
  }
  .dim {
    color: var(--nord-dim);
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
