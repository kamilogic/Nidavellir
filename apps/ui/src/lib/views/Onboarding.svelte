<script>
  import { serviceCall, formatDriverStatus, driverStatusHint, serviceUnavailableHint } from "../service.js";

  let { step = $bindable(1), onComplete } = $props();
  let loading = $state(false);
  let error = $state(null);
  let driverStatus = $state(null);
  let hardware = $state(null);

  const primaryGpu = $derived(hardware?.gpu?.[0] ?? null);

  async function probe() {
    loading = true;
    error = null;
    try {
      const hw = await serviceCall("DetectHardware");
      hardware = hw?.data?.type === "Hardware" ? hw.data : null;
      const drv = await serviceCall("GetDriverStatus");
      driverStatus = drv?.data?.type === "DriverStatus" ? drv.data : null;
      step = 2;
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }

  function acceptRisk() {
    onComplete?.("gpu");
  }
</script>

<div class="onboarding">
  <header>
    <p class="eyebrow">Step {step} of 2</p>
    <h1>Prepare the GPU Forge</h1>
    <p class="lead">
      Nidavellir starts with your NVIDIA GPU, checks that the service can see it, and keeps risky tuning behind Safe Loop protection.
    </p>
  </header>

  {#if step === 1}
    <section class="card">
      <span class="card-kicker">GPU readiness</span>
      <h2>Detect the GPU</h2>
      <p>We check the local GPU service and confirm the current NVIDIA target before the Forge screen opens.</p>
      {#if error}
        <p class="error">{error}</p>
        <p class="hint">{serviceUnavailableHint()}</p>
      {/if}
      <button onclick={probe} disabled={loading}>
        {loading ? "Checking GPU..." : "Check GPU readiness"}
      </button>
    </section>
  {:else if step === 2}
    <section class="card">
      <span class="card-kicker">Safety gate</span>
      <h2>Safety acknowledgement</h2>
      <p>
        GPU tuning can cause instability, display driver resets, or a reboot near the edge of stability.
        Nidavellir uses Safe Loop recovery, records risky steps before they run, and returns to stock when safety requires it.
      </p>
      {#if primaryGpu}
        <p class="meta gpu-ok">GPU target: {primaryGpu.model}</p>
      {/if}
      {#if driverStatus}
        <p class="meta driver-ok">{formatDriverStatus(driverStatus)}</p>
        {#if driverStatusHint(driverStatus)}
          <p class="hint">{driverStatusHint(driverStatus)}</p>
        {/if}
      {/if}
      <button class="go" onclick={acceptRisk}>Open GPU Forge</button>
    </section>
  {/if}
</div>

<style>
  .onboarding {
    max-width: 640px;
    margin: 0 auto;
  }
  .eyebrow,
  .card-kicker {
    display: block;
    text-transform: uppercase;
    letter-spacing: 0.1em;
    font-size: 0.7rem;
    font-weight: 800;
    color: var(--nord-dim);
    margin-bottom: 0.35rem;
  }
  h1,
  h2 {
    color: var(--forge-text);
  }
  h2 {
    margin: 0;
    font-size: 1.15rem;
  }
  .lead {
    color: var(--nord-mist);
    line-height: 1.6;
    max-width: 54ch;
  }
  .card {
    background: var(--forge-panel-bg);
    border: none;
    border-radius: 12px;
    padding: 1.35rem 1.45rem;
    margin-top: 1.5rem;
  }
  .card p {
    color: var(--nord-mist);
    line-height: 1.55;
    margin: 0.6rem 0 0;
  }
  button {
    margin-top: 1rem;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-height: 2.5rem;
    background: var(--forge-gold);
    color: var(--forge-ink);
    border: none;
    border-radius: 8px;
    padding: 0.56rem 1.1rem;
    font-weight: 700;
    cursor: pointer;
  }
  button:hover {
    opacity: 0.92;
  }
  button:disabled {
    opacity: 0.6;
    cursor: wait;
  }
  .error { color: var(--nord-danger); }
  .hint {
    color: var(--forge-blue);
    font-size: 0.9rem;
  }
  .meta {
    border: 1px solid rgba(255, 255, 255, 0.055);
    border-radius: 8px;
    background: rgba(5, 7, 11, 0.22);
    color: var(--nord-mist);
    font-size: 0.86rem;
    padding: 0.52rem 0.65rem;
  }
  .gpu-ok { color: var(--forge-gold); }
  .driver-ok { color: var(--nord-mist); }
</style>
