<script>
  import { serviceCall, formatDriverStatus, driverStatusHint, serviceUnavailableHint } from "../service.js";

  let { step = $bindable(1), onComplete } = $props();
  let loading = $state(false);
  let error = $state(null);
  let driverStatus = $state(null);

  async function probe() {
    loading = true;
    error = null;
    try {
      await serviceCall("DetectHardware");
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
    step = 3;
  }

  function chooseGoal(goal) {
    onComplete?.(goal);
  }
</script>

<div class="onboarding">
  <header>
    <p class="eyebrow">Step {step} of 3</p>
    <h1>Welcome to the forge</h1>
    <p class="lead">
      Nidavellir detects what your hardware can actually do — and never promises what it cannot deliver.
    </p>
  </header>

  {#if step === 1}
    <section class="card">
      <h2>Detect hardware</h2>
      <p>We scan CPU, GPU, RAM, and motherboard, then probe capabilities.</p>
      {#if error}
        <p class="error">{error}</p>
        <p class="hint">{serviceUnavailableHint()}</p>
      {/if}
      <button onclick={probe} disabled={loading}>
        {loading ? "Scanning…" : "Run detection"}
      </button>
    </section>
  {:else if step === 2}
    <section class="card">
      <h2>Understand the risks</h2>
      <p>
        Hardware tuning can cause instability or crashes. Nidavellir v0.2 will add the Safe Loop parachute;
        until then, proceed only if you accept responsibility for changes to your system.
      </p>
      {#if driverStatus}
        <p class="meta driver-ok">{formatDriverStatus(driverStatus)}</p>
        {#if driverStatusHint(driverStatus)}
          <p class="hint">{driverStatusHint(driverStatus)}</p>
        {/if}
      {/if}
      <button onclick={acceptRisk}>I understand — continue</button>
    </section>
  {:else}
    <section class="card">
      <h2>Choose your first goal</h2>
      <div class="goals">
        <button class="goal" onclick={() => chooseGoal("gpu")}>GPU first</button>
        <button class="goal" onclick={() => chooseGoal("all")}>Explore everything</button>
        <button class="goal" onclick={() => chooseGoal("explore")}>Just look around</button>
      </div>
    </section>
  {/if}
</div>

<style>
  .onboarding { max-width: 640px; margin: 0 auto; }
  .eyebrow { text-transform: uppercase; letter-spacing: 0.12em; font-size: 0.75rem; color: var(--nord-frost); }
  .lead { color: var(--nord-mist); line-height: 1.6; }
  .card {
    background: rgba(19, 31, 46, 0.82);
    border: 1px solid var(--nord-border-card);
    border-radius: 12px;
    padding: 1.5rem;
    margin-top: 1.5rem;
  }
  button {
    margin-top: 1rem;
    background: linear-gradient(135deg, var(--nord-frost) 0%, var(--nord-frost-bright) 100%);
    color: #08101b;
    border: none;
    border-radius: 8px;
    padding: 0.75rem 1.25rem;
    font-weight: 600;
    cursor: pointer;
  }
  button:disabled { opacity: 0.6; cursor: wait; }
  .goals { display: flex; gap: 0.75rem; flex-wrap: wrap; }
  .goal { background: rgba(10, 16, 28, 0.75); color: var(--nord-silver); border: 1px solid var(--nord-border); }
  .error { color: var(--nord-danger); }
  .hint { color: var(--nord-frost); font-size: 0.9rem; }
  .meta { color: var(--nord-mist); font-size: 0.9rem; }
  .driver-ok { color: var(--nord-mist); }
</style>

