<script>
  import Onboarding from "./lib/views/Onboarding.svelte";
  import CapabilityReport from "./lib/views/CapabilityReport.svelte";
  import Dashboard from "./lib/views/Dashboard.svelte";
  import SafeLoop from "./lib/views/SafeLoop.svelte";

  let onboarded = $state(false);
  let onboardingStep = $state(1);
  let activeTab = $state("capability");

  function finishOnboarding(_goal) {
    onboarded = true;
    activeTab = "capability";
  }
</script>

<main>
  <header class="top">
    <div>
      <h1>Nidavellir</h1>
      <p class="tagline">Where silicon is forged to its prime.</p>
    </div>
    {#if onboarded}
      <nav>
        <button class:active={activeTab === "capability"} onclick={() => (activeTab = "capability")}>
          Capacidades
        </button>
        <button class:active={activeTab === "dashboard"} onclick={() => (activeTab = "dashboard")}>
          Sensores
        </button>
        <button class:active={activeTab === "safety"} onclick={() => (activeTab = "safety")}>
          Segurança
        </button>
      </nav>
    {/if}
  </header>

  {#if !onboarded}
    <Onboarding bind:step={onboardingStep} onComplete={finishOnboarding} />
  {:else if activeTab === "capability"}
    <CapabilityReport />
  {:else if activeTab === "dashboard"}
    <Dashboard />
  {:else}
    <SafeLoop />
  {/if}
</main>

<style>
  :global(:root) {
    --nord-void: #060910;
    --nord-night: #0a101c;
    --nord-deep: #0e1726;
    --nord-surface: #131f2e;
    --nord-surface-2: #1a2839;
    --nord-raised: #223044;
    --nord-border: rgba(136, 192, 208, 0.14);
    --nord-border-card: rgba(136, 192, 208, 0.22);
    --nord-frost: #8fbcbb;
    --nord-frost-bright: #88c0d0;
    --nord-frost-dim: #5e8094;
    --nord-silver: #d8dee9;
    --nord-mist: #92a4bd;
    --nord-dim: #5c6b7e;
    --nord-aurora: #a3be8c;
    --nord-aurora-glow: rgba(163, 190, 140, 0.35);
    --nord-ember: #d08770;
    --nord-ember-bright: #ebcb8b;
    --nord-twilight: rgba(180, 142, 173, 0.12);
    --nord-danger: #bf616a;
  }

  :global(body) {
    margin: 0;
    min-height: 100vh;
    font-family: Inter, "Segoe UI", system-ui, sans-serif;
    color: var(--nord-silver);
    background-color: var(--nord-void);
    background-image:
      radial-gradient(ellipse 110% 65% at 50% -28%, rgba(94, 129, 172, 0.28) 0%, transparent 52%),
      radial-gradient(ellipse 55% 45% at 105% 5%, var(--nord-twilight) 0%, transparent 48%),
      radial-gradient(ellipse 40% 35% at -5% 25%, rgba(143, 188, 187, 0.06) 0%, transparent 45%),
      linear-gradient(168deg, var(--nord-void) 0%, var(--nord-night) 38%, #070a10 100%);
    background-attachment: fixed;
  }

  main {
    max-width: 1120px;
    margin: 0 auto;
    padding: 2rem 1.5rem 3rem;
  }

  .top {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    gap: 1rem;
    margin-bottom: 1.75rem;
    border-bottom: 1px solid var(--nord-border);
    padding-bottom: 1.25rem;
  }

  h1 {
    margin: 0;
    font-family: Cinzel, "Palatino Linotype", serif;
    font-size: 1.85rem;
    font-weight: 600;
    letter-spacing: 0.18em;
    text-transform: uppercase;
    color: var(--nord-silver);
    text-shadow:
      0 0 42px rgba(136, 192, 208, 0.18),
      0 1px 0 rgba(0, 0, 0, 0.5);
  }

  .tagline {
    margin: 0.5rem 0 0;
    color: var(--nord-frost-dim);
    font-size: 0.92rem;
    font-style: italic;
    letter-spacing: 0.02em;
  }

  nav {
    display: flex;
    gap: 0.35rem;
    padding: 0.3rem;
    border-radius: 12px;
    background: rgba(10, 16, 28, 0.75);
    border: 1px solid var(--nord-border);
    backdrop-filter: blur(10px);
  }

  nav button {
    background: transparent;
    color: var(--nord-mist);
    border: none;
    border-radius: 9px;
    padding: 0.5rem 1rem;
    cursor: pointer;
    font-weight: 600;
    font-size: 0.85rem;
    letter-spacing: 0.03em;
  }

  nav button:hover {
    color: var(--nord-silver);
    background: rgba(136, 192, 208, 0.06);
  }

  nav button.active {
    color: var(--nord-frost-bright);
    background: rgba(136, 192, 208, 0.1);
    box-shadow:
      inset 0 0 0 1px rgba(136, 192, 208, 0.28),
      0 0 24px rgba(136, 192, 208, 0.08);
  }
</style>
