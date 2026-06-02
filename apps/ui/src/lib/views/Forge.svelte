<script>
  import { serviceCall } from "../service.js";
  import { t } from "../i18n.js";
  import VfChart from "../components/VfChart.svelte";

  let error = $state(null);
  let timer = $state(null);
  let realCurve = $state(null);
  let validation = $state(null);
  let advanced = $state(false);
  let expanded = $state(false);
  let realSweep = $state(null);
  let preflight = $state(false);
  let memSweep = $state(null);
  let memPreflight = $state(false);
  let applied = $state(null);
  let forge = $state(null);
  let forgePreflight = $state(false);
  let benchmark = $state(null);
  let powerSweep = $state(null);
  const forgeRunning = $derived(forge?.running);
  const powerRunning = $derived(powerSweep?.running);

  // Keep a terminal pinned to its newest line (tail -f). The `dep` param makes
  // the action re-run on every appended line so the latest is always in view.
  function autoscroll(node, _dep) {
    const toBottom = () => {
      node.scrollTop = node.scrollHeight;
    };
    toBottom();
    return { update: toBottom };
  }

  const SWEEPING = ["baseline", "vram_diagnostic", "voltage_bisection", "synthesis"];
  const realRunning = $derived(realSweep && SWEEPING.includes(realSweep.phase));
  const memRunning = $derived(memSweep?.running);
  const benchRunning = $derived(benchmark?.running);

  // The point the chart should flatten the curve at. When a profile is applied
  // the GPU is hard-capped there (clock lock), so the *effective* curve is flat
  // from that voltage on — show THAT, not the silicon curve's natural plateau
  // (which is the uncapped top, e.g. 2175 MHz @ 1075 mV, and is misleading once
  // a lower undervolt limit like 1920 @ 900 is locked in).
  const appliedLimit = $derived(
    applied?.core ? { voltage_mv: applied.core.voltage_mv, freq_mhz: applied.core.freq_mhz } : null,
  );
  const chartLimit = $derived(appliedLimit ?? realCurve?.plateau ?? null);

  async function refresh() {
    try {
      const v = await serviceCall("GetGpuValidation");
      validation = v?.data?.type === "GpuValidation" ? v.data : validation;
      const rs = await serviceCall("GetRealSweepProgress");
      realSweep = rs?.data?.type === "GpuSweep" ? rs.data : realSweep;
      const ms = await serviceCall("GetMemSweepProgress");
      memSweep = ms?.data?.type === "MemSweep" ? ms.data : memSweep;
      const ap = await serviceCall("GetAppliedProfile");
      applied = ap?.data?.type === "GpuApply" ? ap.data : applied;
      const fa = await serviceCall("GetForgeAllProgress");
      forge = fa?.data?.type === "ForgeAll" ? fa.data : forge;
      const bm = await serviceCall("GetBenchmarkProgress");
      benchmark = bm?.data?.type === "Benchmark" ? bm.data : benchmark;
      const ps = await serviceCall("GetPowerSweepProgress");
      powerSweep = ps?.data?.type === "PowerSweep" ? ps.data : powerSweep;
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
  const setMem = (r) => (memSweep = r?.data?.type === "MemSweep" ? r.data : memSweep);
  const startMem = () => {
    memPreflight = false;
    call("StartMemSweep", setMem);
  };
  const stopMem = () => call("StopMemSweep", setMem);
  const setApplied = (r) => (applied = r?.data?.type === "GpuApply" ? r.data : applied);
  const CORE_APPLY = ["ApplyGodforge", "ApplyBrokkrs", "ApplyDeepCalm"];
  const applyCore = (i) => call(CORE_APPLY[i], setApplied);
  const applyMem = () => call("ApplyMemPeak", setApplied);
  const resetTuning = () => call("ResetGpuTuning", setApplied);
  const setForge = (r) => (forge = r?.data?.type === "ForgeAll" ? r.data : forge);
  const startForge = () => {
    forgePreflight = false;
    call("StartForgeAll", setForge);
  };
  const stopForge = () => call("StopForgeAll", setForge);
  const setBench = (r) => (benchmark = r?.data?.type === "Benchmark" ? r.data : benchmark);
  const startBench = () => call("StartBenchmark", setBench);
  const stopBench = () => call("StopBenchmark", setBench);
  const setPower = (r) => (powerSweep = r?.data?.type === "PowerSweep" ? r.data : powerSweep);
  const startPower = () => call("StartPowerSweep", setPower);
  const stopPower = () => call("StopPowerSweep", setPower);
  const POWER_APPLY = {
    godforge: "ApplyPowerGodforge",
    brokkrs: "ApplyPowerBrokkrs",
    deep_calm: "ApplyPowerDeepCalm",
  };
  const applyPower = (which) => call(POWER_APPLY[which], setApplied);
  const pct = (a, b) => (a > 0 ? ((b - a) / a) * 100 : 0);
  const sgn = (x, d = 0) => (x >= 0 ? "+" : "") + x.toFixed(d);

  // Before/after rows for the benchmark table (computed here so the markup
  // stays clean — {@const} can't live directly inside a <tr>).
  const benchRows = $derived.by(() => {
    const s = benchmark?.stock,
      u = benchmark?.tuned;
    if (!s || !u) return [];
    const rows = [
      { key: "forge.benchFps", s: s.fps.toFixed(0), u: u.fps.toFixed(0), d: sgn(pct(s.fps, u.fps)) + "%", good: u.fps >= s.fps },
      { key: "forge.benchClock", s: String(s.avg_clock_mhz), u: String(u.avg_clock_mhz), d: sgn(u.avg_clock_mhz - s.avg_clock_mhz) + " MHz", good: u.avg_clock_mhz >= s.avg_clock_mhz },
      { key: "forge.benchPower", s: s.avg_power_w.toFixed(0) + " W", u: u.avg_power_w.toFixed(0) + " W", d: sgn(pct(s.avg_power_w, u.avg_power_w)) + "%", good: u.avg_power_w <= s.avg_power_w },
      { key: "forge.benchPerfWatt", s: s.perf_per_watt.toFixed(2), u: u.perf_per_watt.toFixed(2), d: sgn(pct(s.perf_per_watt, u.perf_per_watt)) + "%", good: u.perf_per_watt >= s.perf_per_watt },
      { key: "forge.benchBandwidth", s: s.bandwidth_gbps.toFixed(0), u: u.bandwidth_gbps.toFixed(0), d: sgn(pct(s.bandwidth_gbps, u.bandwidth_gbps)) + "%", good: u.bandwidth_gbps >= s.bandwidth_gbps },
      { key: "forge.benchTemp", s: s.max_temp_c.toFixed(0) + "°C", u: u.max_temp_c.toFixed(0) + "°C", d: sgn(u.max_temp_c - s.max_temp_c) + "°C", good: u.max_temp_c <= s.max_temp_c },
    ];
    if (s.power_capped_frac > 0.05 || u.power_capped_frac > 0.05) {
      rows.push({ key: "forge.benchPowerCap", s: (s.power_capped_frac * 100).toFixed(0) + "%", u: (u.power_capped_frac * 100).toFixed(0) + "%", d: sgn((u.power_capped_frac - s.power_capped_frac) * 100) + "%", good: u.power_capped_frac <= s.power_capped_frac });
    }
    return rows;
  });

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
  </header>

  {#if error}<p class="err">{error}</p>{/if}

  <div class="applied-bar">
    <div>
      <span class="lab">{applied?.label ? $t("forge.appliedNow", { label: applied.label }) : $t("forge.appliedNone")}</span>
      {#if applied?.core}
        <span class="applied-detail">{applied.core.freq_mhz} MHz @ {applied.core.voltage_mv} mV</span>
      {/if}
      {#if applied?.mem_offset_mhz}
        <span class="applied-detail">mem +{applied.mem_offset_mhz} MHz</span>
      {/if}
      {#if applied?.message}<span class="applied-msg">{applied.message}</span>{/if}
    </div>
    <button class="btn small" onclick={resetTuning}>{$t("forge.reset")}</button>
  </div>
  <p class="sub apply-hint">{$t("forge.applyHint")}</p>

  <div class="bench">
    <div class="real-head">
      <h3 class="section-head">📊 {$t("forge.benchTitle")}</h3>
      {#if benchRunning}
        <button class="btn stop" onclick={stopBench}>{$t("forge.benchStop")}</button>
      {:else}
        <button class="btn go" onclick={startBench} disabled={!applied?.core && !applied?.mem_offset_mhz}>
          {$t("forge.benchRun")}
        </button>
      {/if}
    </div>
    <p class="sub">{$t("forge.benchDesc")}</p>
    {#if benchmark && benchmark.phase !== "idle"}
      {#if benchmark.log?.length}
        <div class="terminal">
          <div class="term-head">
            <span class="dots"><i></i><i></i><i></i></span>
            <span class="term-title">nidavellir · benchmark</span>
            <span class="term-status" class:live={benchRunning}>{benchRunning ? benchmark.phase : "done"}</span>
          </div>
          <div class="term-body" use:autoscroll={benchmark.log.length}>
            {#each benchmark.log as line, i}
              <div class="tline"><span class="gutter">{(i + 1).toString().padStart(2, "0")}</span><span class="tlead">{line}</span></div>
            {/each}
          </div>
        </div>
      {/if}
      {#if benchRows.length}
        <table class="bench-table">
          <thead>
            <tr><th>{$t("forge.benchMetric")}</th><th>Stock</th><th>Tuned</th><th>Δ</th></tr>
          </thead>
          <tbody>
            {#each benchRows as row}
              <tr>
                <td>{$t(row.key)}</td>
                <td>{row.s}</td>
                <td>{row.u}</td>
                <td class:accent={row.good} class:danger={!row.good}>{row.d}</td>
              </tr>
            {/each}
          </tbody>
        </table>
        {#if benchmark.power_limit_w > 0}
          <p class="sub">{$t("forge.benchLimit", { w: benchmark.power_limit_w.toFixed(0) })}</p>
        {/if}
      {/if}
      {#if benchmark.note}<p class="point" class:accent={!benchRunning}>{benchmark.note}</p>{/if}
    {/if}
  </div>

  <div class="forge-all">
    <div class="real-head">
      <h3 class="section-head">⚒ {$t("forge.forgeAll")}</h3>
      {#if forgeRunning}
        <button class="btn stop" onclick={stopForge}>{$t("forge.stopForge")}</button>
      {:else}
        <button class="btn go" onclick={() => (forgePreflight = true)}>{$t("forge.runForge")}</button>
      {/if}
    </div>
    <p class="sub">{$t("forge.forgeAllDesc")}</p>
    {#if forge && forge.phase !== "idle" && (forge.log?.length || forge.running)}
      <div class="terminal">
        <div class="term-head">
          <span class="dots"><i></i><i></i><i></i></span>
          <span class="term-title">nidavellir · forge</span>
          <span class="term-status" class:live={forge.running}>{forge.running ? forge.phase : "done"}</span>
        </div>
        <div class="term-body" use:autoscroll={(forge.log?.length ?? 0) + (forge.running ? 1 : 0)}>
          {#each forge.log as line, i}
            <div class="tline"><span class="gutter">{(i + 1).toString().padStart(2, "0")}</span><span class="tlead">{line}</span></div>
          {/each}
          {#if forge.running}
            <div class="tline running"><span class="gutter">»</span><span class="cursor"></span><span class="tlead">{forge.phase}…</span></div>
          {/if}
        </div>
      </div>
      {#if forge.note}<p class="point" class:accent={!forge.running}>{forge.note}</p>{/if}
    {/if}
  </div>

  <p class="sub apply-hint">{$t("forge.orderHint")}</p>

  <div class="power">
    <div class="real-head">
      <h3 class="section-head">⚡ {$t("forge.powerTitle")}</h3>
      {#if powerRunning}
        <button class="btn stop" onclick={stopPower}>{$t("forge.benchStop")}</button>
      {:else}
        <button class="btn go" onclick={startPower}>{$t("forge.powerRun")}</button>
      {/if}
    </div>
    <p class="sub">{$t("forge.powerDesc")}</p>
    {#if powerSweep && powerSweep.phase !== "idle"}
      {#if powerSweep.power_limit_w > 0}
        <p class="sub">{$t("forge.powerCap", { w: powerSweep.power_limit_w.toFixed(0) })}</p>
      {/if}
      {#if powerSweep.log?.length}
        <div class="terminal">
          <div class="term-head">
            <span class="dots"><i></i><i></i><i></i></span>
            <span class="term-title">nidavellir · power sweep</span>
            <span class="term-status" class:live={powerRunning}>{powerRunning ? "running" : "done"}</span>
          </div>
          <div class="term-body" use:autoscroll={powerSweep.log.length}>
            {#each powerSweep.log as line, i}
              <div class="tline"><span class="gutter">{(i + 1).toString().padStart(2, "0")}</span><span class="tlead">{line}</span></div>
            {/each}
          </div>
        </div>
      {/if}
      {#if powerSweep.stock_clock_mhz > 0}
        <p class="sub">{$t("forge.powerStock", { c: powerSweep.stock_clock_mhz })}</p>
      {/if}
      {#if powerSweep.points?.length}
        <table class="bench-table">
          <thead>
            <tr><th>mV</th><th>MHz</th><th>W (máx)</th><th>cap%</th><th>MHz/W</th></tr>
          </thead>
          <tbody>
            {#each powerSweep.points as p}
              <tr>
                <td>{p.voltage_mv}</td>
                <td>{p.clock_mhz}</td>
                <td>{p.power_w.toFixed(0)} ({p.max_power_w.toFixed(0)})</td>
                <td class:danger={p.power_capped_frac > 0.05}>{(p.power_capped_frac * 100).toFixed(0)}%</td>
                <td>{p.perf_per_watt.toFixed(1)}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      {/if}
      {#if !powerRunning && (powerSweep.godforge || powerSweep.brokkrs || powerSweep.deep_calm)}
        <div class="profiles">
          {#each [["godforge", powerSweep.godforge], ["brokkrs", powerSweep.brokkrs], ["deep_calm", powerSweep.deep_calm]] as [key, p]}
            <div class="profile">
              <div class="prof-name">{$t("forge.prof_" + key)}</div>
              {#if p}
                <div class="prof-val">{p.clock_mhz} MHz @ {p.voltage_mv} mV</div>
                <div class="prof-sub">{p.power_w.toFixed(0)} W · {p.perf_per_watt.toFixed(1)} MHz/W</div>
                <button class="btn go small" onclick={() => applyPower(key)}>{$t("forge.apply")}</button>
              {:else}
                <div class="prof-sub">—</div>
              {/if}
            </div>
          {/each}
        </div>
      {/if}
      {#if powerSweep.note}<p class="point" class:accent={!powerRunning}>{powerSweep.note}</p>{/if}
    {/if}
  </div>

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
        {#if appliedLimit}
          <p class="point accent">
            ⚑ {$t("forge.plateau", { f: appliedLimit.freq_mhz, v: appliedLimit.voltage_mv })}
          </p>
        {:else if realCurve.plateau}
          <p class="point">
            {$t("forge.plateau", { f: realCurve.plateau.freq_mhz, v: realCurve.plateau.voltage_mv })}
          </p>
        {/if}
        <VfChart points={realCurve.points} plateau={chartLimit} height={300} />
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
            {#each [realSweep.profiles.godforge, realSweep.profiles.brokkrs_best, realSweep.profiles.deep_calm] as prof, i}
              <article class="profile">
                <h4>{prof.name}</h4>
                <p class="desc">{prof.description}</p>
                <p class="point">{prof.point.freq_mhz} MHz @ {prof.point.voltage_mv} mV</p>
                <button class="btn go small" onclick={() => applyCore(i)}>{$t("forge.apply")}</button>
              </article>
            {/each}
          </div>
        {/if}
        {#if realSweep.validation_note}
          <p class="note">{realSweep.validation_note}</p>
        {/if}
      {/if}
    </div>

    <div class="realsweep">
      <div class="real-head">
        <h4 class="section-head">{$t("forge.memSweep")}</h4>
        {#if memRunning}
          <button class="btn stop" onclick={stopMem}>{$t("forge.stopMem")}</button>
        {:else}
          <button class="btn go" onclick={() => (memPreflight = true)}>{$t("forge.runMem")}</button>
        {/if}
      </div>
      {#if memSweep && memSweep.phase !== "idle"}
        <div class="terminal">
          <div class="term-head">
            <span class="dots"><i></i><i></i><i></i></span>
            <span class="term-title">nidavellir · memory sweep</span>
            <span class="term-status" class:live={memRunning}>{memRunning ? "running" : "done"}</span>
          </div>
          <div class="term-body" use:autoscroll={(memSweep.points?.length ?? 0) + (memRunning ? 1 : 0)}>
            <div class="tline base"><span class="gutter">··</span><span class="tlead">base · {memSweep.baseline_gbps.toFixed(0)} GB/s</span></div>
            {#each memSweep.points as p, i}
              <div class="tline">
                <span class="gutter">{(i + 1).toString().padStart(2, "0")}</span>
                <span class="tlead">+{p.offset_mhz} MHz · {p.mem_mhz} MHz</span>
                <span class="tval" class:accent={p.stable} class:danger={!p.stable}>{p.bandwidth_gbps.toFixed(0)} GB/s</span>
                {#if p.min_gbps > 0}<span class="tmin">min {p.min_gbps.toFixed(0)}</span>{/if}
                <span class="tstatus" class:danger={!p.stable}>{p.stable ? "ok" : "✗ queda"}</span>
              </div>
            {/each}
            {#if memRunning}
              <div class="tline running">
                <span class="gutter">»</span>
                <span class="cursor"></span>
                <span class="tlead">{memSweep.validation_note ?? "…"}</span>
              </div>
            {/if}
          </div>
        </div>
        {#if memSweep.peak_gbps > 0}
          <p class="point accent">
            {$t("forge.peakResult", { o: memSweep.peak_offset_mhz, g: memSweep.peak_gbps.toFixed(0) })}
          </p>
          {#if !memRunning && memSweep.validation_note}<p class="sub">{memSweep.validation_note}</p>{/if}
          {#if !memRunning}<button class="btn go small" onclick={applyMem}>{$t("forge.applyMem")}</button>{/if}
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

{#if memPreflight}
  <div class="overlay" onclick={() => (memPreflight = false)} role="presentation">
    <div class="modal" onclick={(e) => e.stopPropagation()} role="presentation">
      <div class="modal-head"><strong>⚠ {$t("forge.preTitle")}</strong></div>
      <p class="pre-body">{$t("forge.memPreBody")}</p>
      <div class="pre-actions">
        <button class="btn ghost" onclick={() => (memPreflight = false)}>{$t("forge.preCancel")}</button>
        <button class="btn go" onclick={startMem}>{$t("forge.runMem")}</button>
      </div>
    </div>
  </div>
{/if}

{#if forgePreflight}
  <div class="overlay" onclick={() => (forgePreflight = false)} role="presentation">
    <div class="modal" onclick={(e) => e.stopPropagation()} role="presentation">
      <div class="modal-head"><strong>⚒ {$t("forge.preTitle")}</strong></div>
      <p class="pre-body">{$t("forge.forgePreBody")}</p>
      <div class="pre-actions">
        <button class="btn ghost" onclick={() => (forgePreflight = false)}>{$t("forge.preCancel")}</button>
        <button class="btn go" onclick={startForge}>{$t("forge.runForge")}</button>
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
      <VfChart points={realCurve.points} plateau={chartLimit} height={560} />
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
  .btn.small {
    padding: 0.35rem 0.8rem;
    font-size: 0.78rem;
    margin-top: 0.5rem;
  }
  .applied-bar {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 1rem;
    flex-wrap: wrap;
    background: rgba(163, 190, 140, 0.08);
    border: 1px solid rgba(163, 190, 140, 0.3);
    border-radius: 10px;
    padding: 0.6rem 0.9rem;
  }
  .applied-bar .lab {
    margin: 0;
    color: var(--accent);
    letter-spacing: normal;
    text-transform: none;
    font-size: 0.9rem;
  }
  .applied-detail {
    font-variant-numeric: tabular-nums;
    color: var(--text);
    font-size: 0.85rem;
    margin-left: 0.6rem;
  }
  .applied-msg {
    display: block;
    color: var(--muted);
    font-size: 0.78rem;
    margin-top: 0.2rem;
  }
  .apply-hint {
    margin: 0.1rem 0 0.4rem;
    font-size: 0.75rem;
    color: var(--nord-dim);
  }
  .forge-all {
    background: rgba(163, 190, 140, 0.06);
    border: 1px solid rgba(163, 190, 140, 0.28);
    border-radius: 12px;
    padding: 0.85rem 1rem;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  .bench,
  .power {
    margin-top: 1rem;
    padding-top: 1rem;
    border-top: 1px solid var(--border);
  }
  .bench-table tr.rec td {
    background: rgba(163, 190, 140, 0.12);
    color: var(--nord-aurora);
    font-weight: 700;
  }
  .profiles {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 0.6rem;
    margin-top: 0.7rem;
  }
  .profile {
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 0.6rem 0.7rem;
    background: rgba(10, 16, 28, 0.5);
    text-align: center;
  }
  .prof-name {
    font-weight: 700;
    color: var(--nord-frost-bright);
    font-size: 0.85rem;
    letter-spacing: 0.03em;
  }
  .prof-val {
    margin-top: 0.3rem;
    color: var(--text);
    font-variant-numeric: tabular-nums;
  }
  .prof-sub {
    color: var(--muted);
    font-size: 0.78rem;
    margin: 0.15rem 0 0.5rem;
    font-variant-numeric: tabular-nums;
  }
  .bench-table {
    width: 100%;
    border-collapse: collapse;
    margin-top: 0.6rem;
    font-size: 0.85rem;
    font-variant-numeric: tabular-nums;
  }
  .bench-table th,
  .bench-table td {
    text-align: right;
    padding: 0.32rem 0.6rem;
    border-bottom: 1px solid var(--border);
  }
  .bench-table th:first-child,
  .bench-table td:first-child {
    text-align: left;
    color: var(--muted);
  }
  .bench-table th {
    color: var(--nord-mist);
    font-size: 0.72rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .bench-table td.accent {
    color: var(--nord-aurora);
    font-weight: 700;
  }
  .bench-table td.danger {
    color: var(--nord-danger);
    font-weight: 700;
  }
  .terminal {
    font-family: "Cascadia Code", "Consolas", ui-monospace, monospace;
    font-size: 0.8rem;
    background: rgba(6, 9, 16, 0.92);
    border: 1px solid var(--border);
    border-radius: 10px;
    overflow: hidden;
    box-shadow: inset 0 0 0 1px rgba(136, 192, 208, 0.04), 0 8px 24px rgba(0, 0, 0, 0.35);
  }
  .term-head {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    padding: 0.4rem 0.7rem;
    background: rgba(136, 192, 208, 0.05);
    border-bottom: 1px solid var(--border);
  }
  .dots {
    display: inline-flex;
    gap: 0.32rem;
  }
  .dots i {
    width: 0.62rem;
    height: 0.62rem;
    border-radius: 50%;
    background: var(--nord-dim);
    opacity: 0.6;
  }
  .dots i:nth-child(1) {
    background: var(--nord-danger);
  }
  .dots i:nth-child(2) {
    background: var(--nord-ember-bright);
  }
  .dots i:nth-child(3) {
    background: var(--nord-aurora);
  }
  .term-title {
    color: var(--nord-mist);
    font-size: 0.74rem;
    letter-spacing: 0.04em;
  }
  .term-status {
    margin-left: auto;
    font-size: 0.68rem;
    text-transform: lowercase;
    color: var(--nord-dim);
    padding: 0.08rem 0.5rem;
    border-radius: 999px;
    border: 1px solid var(--border);
  }
  .term-status.live {
    color: var(--nord-ember-bright);
    border-color: rgba(235, 203, 139, 0.4);
    background: rgba(235, 203, 139, 0.08);
  }
  .term-body {
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
    padding: 0.55rem 0.7rem;
    max-height: 340px;
    overflow-y: auto;
    scroll-behavior: smooth;
  }
  .term-body::-webkit-scrollbar {
    width: 8px;
  }
  .term-body::-webkit-scrollbar-thumb {
    background: rgba(136, 192, 208, 0.18);
    border-radius: 8px;
  }
  .tline {
    display: flex;
    align-items: baseline;
    gap: 0.75rem;
    padding: 0.12rem 0;
    color: var(--muted);
    font-variant-numeric: tabular-nums;
    border-radius: 4px;
  }
  .gutter {
    color: var(--nord-dim);
    opacity: 0.55;
    min-width: 1.4rem;
    text-align: right;
    user-select: none;
    flex-shrink: 0;
  }
  .tline.base {
    color: var(--nord-dim);
    border-bottom: 1px solid var(--border);
    padding-bottom: 0.3rem;
    margin-bottom: 0.2rem;
  }
  .tlead {
    min-width: 16rem;
    color: var(--text);
  }
  .cursor {
    display: inline-block;
    width: 0.5rem;
    height: 0.85rem;
    background: var(--nord-ember-bright);
    align-self: center;
    animation: blink 1s steps(2, start) infinite;
    flex-shrink: 0;
  }
  @keyframes blink {
    50% {
      opacity: 0;
    }
  }
  .tval {
    min-width: 5rem;
    text-align: right;
  }
  .tval.accent {
    color: var(--accent);
  }
  .tval.danger,
  .tstatus.danger {
    color: var(--nord-danger);
  }
  .tmin {
    min-width: 5rem;
    text-align: right;
    color: var(--nord-dim);
    font-size: 0.72rem;
  }
  .tstatus {
    color: var(--nord-aurora);
    font-size: 0.72rem;
    opacity: 0.8;
  }
  .tline.running {
    color: var(--nord-ember-bright);
  }
</style>
