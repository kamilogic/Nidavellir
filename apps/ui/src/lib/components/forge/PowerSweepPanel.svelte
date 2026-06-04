<script>
  import { t } from "../../i18n.js";
  import LogTerminal from "./LogTerminal.svelte";
  import ProfileCards from "./ProfileCards.svelte";

  let { powerSweep = null, powerRunning = false, applied = null, onStartPower, onStopPower, onApplyPower } = $props();
</script>

<div class="power">
  <div class="real-head">
    <h3 class="section-head">{$t("forge.powerTitle")}</h3>
    {#if powerRunning}
      <button class="btn stop" onclick={onStopPower}>{$t("forge.benchStop")}</button>
    {:else}
      <button class="btn go" onclick={onStartPower}>{$t("forge.powerRun")}</button>
    {/if}
  </div>
  <p class="sub">{$t("forge.powerDesc")}</p>
  {#if powerSweep && powerSweep.phase !== "idle"}
    {#if powerSweep.power_limit_w > 0}
      <p class="sub">{$t("forge.powerCap", { w: powerSweep.power_limit_w.toFixed(0) })}</p>
    {/if}
    {#if powerSweep.log?.length}
      <LogTerminal
        title="nidavellir / power sweep"
        status={powerRunning ? "running" : "done"}
        live={powerRunning}
        lines={powerSweep.log}
      />
    {/if}
    {#if powerSweep.stock_clock_mhz > 0}
      <p class="sub">{$t("forge.powerStock", { c: powerSweep.stock_clock_mhz })}</p>
    {/if}
    {#if powerSweep.points?.length}
      <table class="bench-table">
        <thead>
          <tr><th>mV</th><th>MHz</th><th>W (max)</th><th>cap%</th><th>MHz/W</th></tr>
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
    <ProfileCards mode="power" {powerSweep} {applied} onApplyPower={onApplyPower} />
    {#if powerSweep.note}
      <p class="point" class:accent={!powerRunning}>{powerSweep.note}</p>
    {/if}
  {/if}
</div>

<style>
  .power {
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
  .bench-table td.danger {
    color: var(--nord-danger);
    font-weight: 700;
  }
</style>
