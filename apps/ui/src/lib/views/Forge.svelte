<script>
  import { serviceCall } from "../service.js";
  import { t } from "../i18n.js";
  import VfChart from "../components/VfChart.svelte";

  let progress = $state(null);
  let error = $state(null);
  let timer = $state(null);
  let realCurve = $state(null);
  let validation = $state(null);
  let advanced = $state(false);
  let expanded = $state(false);
  let realSweep = $state(null);
  let preflight = $state(false);

  const realRunning = $derived(
    realSweep &&
      ["baseline", "vram_diagnostic", "voltage_bisection", "synthesis"].includes(realSweep.phase),
  );

  const running = $derived(
    progress &&
      ["baseline", "vram_diagnostic", "voltage_bisection", "synthesis"].includes(progress.phase),
  );

  function captureProgress(r) {
    progress = r?.data?.type === "GpuSweep" ? r.data : progress;
  }

  async function refresh() {
    try {
      captureProgress(await serviceCall("GetGpuSweepProgress"));
      const v = await serviceCall("GetGpuValidation");
      validation = v?.data?.type === "GpuValidation" ? v.data : validation;
      const rs = await serviceCall("GetRealSweepProgress");
      realSweep = rs?.data?.type === "GpuSweep" ? rs.data : realSweep;
      error = null;
    } catch (e) {
      error = String(e);
    }
  }

  async function call(method, set) {
    try {
      const r = await serviceCall(method);
      set(r);
      error = null;
    } catch (e) {
      error = String(e);
    }
  }

  const start = () => call("StartGpuSweep", captureProgress);
  const stop = () => call("StopGpuSweep", captureProgress);
  const readRealCurve = () =>
    call("GetGpuCurve", (r) => (realCurve = r?.data?.type === "GpuCurve" ? r.data : realCurve));
  const startValidation = () =>
    call("StartGpuValidation", (r) => (validation = r?.data?.type === "GpuValidation" ? r.data : validation));
  const setReal = (r) => (realSweep = r?.data?.type === "GpuSweep" ? r.data : realSweep);
  const startReal = (method) => {
    preflight = false;
    call(method, setReal);
  };
  const stopReal = () => call("StopRealSweep", setReal);

  $effect(() => {
    refresh();
    timer = setInterval(refresh, 500);
    return () => clearInterval(timer);
  });
</script>

<section class="forge">
  <header class="forge-head">
    <div>
      <h2>{$t("forge.title")}</h2>
      <p class="lead">{$t("forge.lead")}</p>
    </div>
    <div class="actions">
      {#if progress?.simulated}
        <span class="badge sim">{$t("forge.simulated")}</span>
      {/if}
      {#if running}
        <button class="btn stop" onclick={stop}>{$t("forge.stop")}</button>
      {:else}
        <button class="btn go" onclick={start}>{$t("forge.start")}</button>
      {/if}
    </div>
  </header>

  {#if progress?.simulated}
    <p class="note">{$t("forge.simNote")}</p>
  {/if}

  {#if error}<p class="err">{error}</p>{/if}

  {#if progress}
    <div class="grid">
      <article class="tile">
        <span class="lab">{$t("forge.phase")}</span>
        <p class="val">{$t("phase." + progress.phase)}</p>
      </article>
      <article class="tile">
        <span class="lab">{$t("forge.frequency")}</span>
        <p class="val">{progress.freq_index} / {progress.total_freqs}</p>
      </article>
      <article class="tile">
        <span class="lab">{$t("forge.testingNow")}</span>
        <p class="val">
          {#if progress.current}{progress.current.freq_mhz} MHz @ {progress.current.voltage_mv} mV{:else}—{/if}
        </p>
      </article>
    </div>

    {#if progress.tradeoffs?.length}
      <div class="section">
        <h3 class="section-head">{$t("forge.tradeoffs")}</h3>
        <ul class="list">
          {#each progress.tradeoffs as tp}
            <li><span class="mono">{tp.freq_mhz} MHz</span><span class="mono accent">{tp.vmin_mv} mV</span></li>
          {/each}
        </ul>
      </div>
    {/if}

    {#if progress.profiles}
      <div class="section">
        <h3 class="section-head">{$t("forge.profiles")}</h3>
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
  {/if}

  <div class="section real">
    <div class="real-head">
      <h3 class="section-head">{$t("forge.realTitle")}</h3>
      <label class="adv-toggle">
        <input type="checkbox" bind:checked={advanced} /> {$t("forge.advanced")}
      </label>
    </div>
    <div class="real-actions">
      <button class="btn" onclick={readRealCurve}>{$t("forge.readCurve")}</button>
      <button class="btn go" onclick={startValidation} disabled={validation?.running}>
        {validation?.running ? $t("forge.validating") : $t("forge.validate")}
      </button>
      {#if realCurve?.real}
        <button class="btn ghost" onclick={() => (expanded = true)}>{$t("forge.expand")}</button>
      {/if}
    </div>

    {#if realCurve}
      {#if realCurve.real}
        {#if realCurve.plateau}
          <p class="point accent">
            {$t("forge.plateau", { f: realCurve.plateau.freq_mhz, v: realCurve.plateau.voltage_mv })}
          </p>
        {/if}
        <VfChart points={realCurve.points} plateau={realCurve.plateau} height={300} />
        {#if advanced}
          <p class="sub">{$t("forge.curvePoints", { name: realCurve.name, n: realCurve.points.length })}</p>
          <ul class="list">
            {#each realCurve.points.filter((_, i) => i % 4 === 0) as p}
              <li><span class="mono">{p.voltage_mv} mV</span><span class="mono accent">{p.freq_mhz} MHz</span></li>
            {/each}
          </ul>
        {/if}
      {:else}
        <p class="err">{realCurve.name}</p>
      {/if}
    {/if}

    {#if validation}
      <div class="val-box">
        {#if validation.error}<p class="err">{validation.error}</p>{/if}
        {#if validation.total_stages}
          <div class="stages">
            {#each Array(validation.total_stages) as _, i}
              {@const done = validation.stages[i]}
              {@const active = validation.running && i === validation.stage_index}
              <div class="stage" class:active class:done>
                <span class="stage-ic">
                  {#if done}{done.result === "stable" ? "✓" : "✗"}{:else if active}<span class="spin">◴</span>{:else}·{/if}
                </span>
                <span class="stage-name">{done?.name ?? (active ? validation.current_stage : $t("forge.stageN", { n: i + 1 }))}</span>
                {#if done}
                  <span class="stage-meta" class:danger={done.result !== "stable"}>
                    {$t("stage." + done.result)} · {done.mismatches} mm · {done.elapsed_ms} ms
                  </span>
                {/if}
              </div>
            {/each}
          </div>
        {/if}
        {#if validation.running}
          <p class="sub">{$t("forge.running")}</p>
        {:else if validation.result}
          <p class="point" class:danger={validation.result !== "stable"} class:accent={validation.result === "stable"}>
            {$t("forge.result", { r: $t("val." + validation.result) })}
          </p>
          {#if validation.adapter}<p class="sub">{validation.adapter}</p>{/if}
        {/if}
      </div>
    {/if}

    <div class="realsweep">
      <div class="real-head">
        <h4 class="section-head">{$t("forge.realSweep")}</h4>
        {#if realRunning}
          <button class="btn stop" onclick={stopReal}>{$t("forge.stopReal")}</button>
        {:else}
          <button class="btn go" onclick={() => (preflight = true)}>{$t("forge.runReal")}</button>
        {/if}
      </div>

      {#if realSweep && realSweep.phase !== "idle"}
        <div class="grid">
          <article class="tile">
            <span class="lab">{$t("forge.phase")}</span>
            <p class="val">{$t("phase." + realSweep.phase)}</p>
          </article>
          <article class="tile">
            <span class="lab">{$t("forge.voltageIdx")}</span>
            <p class="val">{realSweep.freq_index} / {realSweep.total_freqs}</p>
          </article>
          <article class="tile">
            <span class="lab">{$t("forge.testingNow")}</span>
            {#if realSweep.current}
              <p class="val" class:accent={realSweep.last_result === "stable"} class:danger={realSweep.last_result && realSweep.last_result !== "stable"}>
                {realSweep.current.freq_mhz} MHz @ {realSweep.current.voltage_mv} mV
              </p>
              <p class="sub">
                {#if realSweep.gpu_temp_c != null}{$t("forge.tempC", { t: realSweep.gpu_temp_c.toFixed(0) })}{/if}
                {#if realSweep.last_result} · {$t("stage." + realSweep.last_result)}{/if}
              </p>
            {:else}
              <p class="val">—</p>
            {/if}
          </article>
        </div>

        {#if realSweep.tradeoffs?.length}
          <h5 class="section-head">{$t("forge.realResult")}</h5>
          <ul class="list">
            {#each realSweep.tradeoffs as tp}
              <li><span class="mono">{tp.freq_mhz} MHz</span><span class="mono accent">{tp.vmin_mv} mV</span></li>
            {/each}
          </ul>
        {/if}

        {#if realSweep.profiles}
          <h5 class="section-head">{$t("forge.profiles")}</h5>
          <div class="profiles">
            {#each [realSweep.profiles.godforge, realSweep.profiles.brokkrs_best, realSweep.profiles.deep_calm] as prof}
              <article class="profile">
                <h4>{prof.name}</h4>
                <p class="desc">{prof.description}</p>
                <p class="point">{prof.point.freq_mhz} MHz @ {prof.point.voltage_mv} mV</p>
              </article>
            {/each}
          </div>
        {/if}
      {/if}
    </div>
  </div>
</section>

{#if preflight}
  <div class="overlay" onclick={() => (preflight = false)} role="presentation">
    <div class="modal" onclick={(e) => e.stopPropagation()} role="presentation">
      <div class="modal-head">
        <strong>⚠ {$t("forge.preTitle")}</strong>
      </div>
      <p class="pre-body">{$t("forge.preBody")}</p>
      <div class="pre-actions">
        <button class="btn ghost" onclick={() => (preflight = false)}>{$t("forge.preCancel")}</button>
        <button class="btn" onclick={() => startReal("StartRealSweepFast")}>{$t("forge.preFast")}</button>
        <button class="btn go" onclick={() => startReal("StartRealSweep")}>{$t("forge.preThorough")}</button>
      </div>
    </div>
  </div>
{/if}

{#if expanded && realCurve?.real}
  <div class="overlay" onclick={() => (expanded = false)} role="presentation">
    <div class="modal" onclick={(e) => e.stopPropagation()} role="presentation">
      <div class="modal-head">
        <strong>{realCurve.name}</strong>
        <button class="btn ghost" onclick={() => (expanded = false)}>{$t("forge.close")}</button>
      </div>
      <VfChart points={realCurve.points} plateau={realCurve.plateau} height={560} />
    </div>
  </div>
{/if}

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
    max-width: 64ch;
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
    background: rgba(10, 16, 28, 0.6);
    color: var(--text);
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
  .btn.ghost {
    background: transparent;
    color: var(--muted);
  }
  .btn:disabled {
    opacity: 0.55;
    cursor: default;
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
    margin: 0.3rem 0;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    color: var(--text);
  }
  .point.accent {
    color: var(--accent);
  }
  .point.danger {
    color: var(--nord-danger);
  }
  .sub {
    margin: 0.25rem 0;
    font-size: 0.82rem;
    color: var(--muted);
  }
  .err {
    color: var(--nord-danger);
    font-size: 0.9rem;
  }
  .real {
    border-top: 1px solid var(--border);
    padding-top: 1rem;
  }
  .real-head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 1rem;
  }
  .adv-toggle {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    font-size: 0.75rem;
    color: var(--muted);
    cursor: pointer;
    user-select: none;
  }
  .real-actions {
    display: flex;
    gap: 0.6rem;
    margin-bottom: 0.6rem;
    flex-wrap: wrap;
  }
  .val-box {
    margin-top: 0.6rem;
  }
  .stages {
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
    margin: 0.5rem 0;
  }
  .stage {
    display: grid;
    grid-template-columns: 1.4rem 1fr auto;
    align-items: center;
    gap: 0.5rem;
    padding: 0.45rem 0.7rem;
    border-radius: 9px;
    border: 1px solid var(--border);
    background: var(--surface);
    font-size: 0.82rem;
    opacity: 0.7;
  }
  .stage.active,
  .stage.done {
    opacity: 1;
  }
  .stage.active {
    border-color: rgba(163, 190, 140, 0.45);
  }
  .stage-ic {
    text-align: center;
    color: var(--accent);
    font-weight: 700;
  }
  .stage-name {
    color: var(--text);
  }
  .stage-meta {
    font-variant-numeric: tabular-nums;
    color: var(--muted);
    font-size: 0.75rem;
  }
  .stage-meta.danger {
    color: var(--nord-danger);
  }
  .spin {
    display: inline-block;
    animation: spin 1s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .overlay {
    position: fixed;
    inset: 0;
    background: rgba(4, 6, 12, 0.78);
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 2rem;
    z-index: 50;
  }
  .modal {
    background: var(--nord-deep, #0e1726);
    border: 1px solid var(--border);
    border-radius: 14px;
    padding: 1.1rem;
    width: min(1100px, 95vw);
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.5);
  }
  .modal-head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 0.6rem;
    color: var(--text);
  }
  .realsweep {
    margin-top: 1rem;
    padding-top: 0.85rem;
    border-top: 1px dashed var(--border);
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
  }
  .pre-body {
    color: var(--muted);
    font-size: 0.9rem;
    line-height: 1.55;
    margin: 0 0 1rem;
  }
  .pre-actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.6rem;
  }
</style>
