<script>
  import { t } from "../../i18n.js";
  import LogTerminal from "./LogTerminal.svelte";

  let { benchmark = null, benchRunning = false, applied = null, onStartBench, onStopBench } = $props();

  const pct = (a, b) => (a > 0 ? ((b - a) / a) * 100 : 0);
  const sgn = (x, d = 0) => (x >= 0 ? "+" : "") + x.toFixed(d);

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
      { key: "forge.benchTemp", s: s.max_temp_c.toFixed(0) + " C", u: u.max_temp_c.toFixed(0) + " C", d: sgn(u.max_temp_c - s.max_temp_c) + " C", good: u.max_temp_c <= s.max_temp_c },
    ];
    if (s.power_capped_frac > 0.05 || u.power_capped_frac > 0.05) {
      rows.push({ key: "forge.benchPowerCap", s: (s.power_capped_frac * 100).toFixed(0) + "%", u: (u.power_capped_frac * 100).toFixed(0) + "%", d: sgn((u.power_capped_frac - s.power_capped_frac) * 100) + "%", good: u.power_capped_frac <= s.power_capped_frac });
    }
    return rows;
  });
</script>

<div class="bench">
  <div class="real-head">
    <h3 class="section-head">{$t("forge.benchTitle")}</h3>
    {#if benchRunning}
      <button class="btn stop" onclick={onStopBench}>{$t("forge.benchStop")}</button>
    {:else}
      <button class="btn go" onclick={onStartBench} disabled={!applied?.core && !applied?.mem_offset_mhz}>
        {$t("forge.benchRun")}
      </button>
    {/if}
  </div>
  <p class="sub">{$t("forge.benchDesc")}</p>
  {#if benchmark && benchmark.phase !== "idle"}
    {#if benchmark.log?.length}
      <LogTerminal
        title="nidavellir / benchmark"
        status={benchRunning ? benchmark.phase : "done"}
        live={benchRunning}
        lines={benchmark.log}
      />
    {/if}
    {#if benchRows.length}
      <table class="bench-table">
        <thead>
          <tr><th>{$t("forge.benchMetric")}</th><th>Stock</th><th>Tuned</th><th>Change</th></tr>
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
    {#if benchmark.note}
      <p class="point" class:accent={!benchRunning}>{benchmark.note}</p>
    {/if}
  {/if}
</div>

<style>
  .bench {
    margin-top: 1rem;
    padding-top: 1rem;
    border-top: 1px solid var(--forge-line);
  }
  .real-head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 1rem;
  }
  .section-head {
    margin: 0 0 0.5rem;
    font-size: 0.72rem;
    font-weight: 700;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--muted);
  }
  .sub {
    margin: 0.25rem 0;
    font-size: 0.82rem;
    color: var(--muted);
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
  .btn {
    border: 1px solid var(--border);
    border-radius: 9px;
    padding: 0.55rem 1.1rem;
    font-weight: 600;
    font-size: 0.85rem;
    cursor: pointer;
    background: rgba(8, 11, 16, 0.66);
    color: var(--text);
  }
  .btn.go {
    background: rgba(214, 168, 93, 0.13);
    color: var(--forge-gold);
    border-color: rgba(214, 168, 93, 0.42);
  }
  .btn.stop {
    background: rgba(191, 97, 106, 0.16);
    color: #f3b9bd;
    border-color: rgba(191, 97, 106, 0.45);
  }
  .btn:disabled {
    opacity: 0.55;
    cursor: default;
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
    border-bottom: 1px solid var(--forge-line);
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
</style>
