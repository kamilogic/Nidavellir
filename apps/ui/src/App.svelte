<script>
  import Onboarding from "./lib/views/Onboarding.svelte";
  import CapabilityReport from "./lib/views/CapabilityReport.svelte";
  import Dashboard from "./lib/views/Dashboard.svelte";
  import SafeLoop from "./lib/views/SafeLoop.svelte";
  import Forge from "./lib/views/Forge.svelte";
  import { t, locale, locales } from "./lib/i18n.js";

  let onboarded = $state(false);
  let onboardingStep = $state(1);
  let activeTab = $state("forge");

  function finishOnboarding(_goal) {
    onboarded = true;
    activeTab = "forge";
  }
</script>

<main>
  <header class="top">
    <div>
      <h1>Nidavellir</h1>
      <p class="tagline">{$t("app.tagline")}</p>
    </div>
    <div class="top-right">
      {#if onboarded}
        <nav>
          <button class:active={activeTab === "forge"} onclick={() => (activeTab = "forge")}>
            {$t("nav.forge")}
          </button>
          <button class:active={activeTab === "capability"} onclick={() => (activeTab = "capability")}>
            {$t("nav.capabilities")}
          </button>
          <button class:active={activeTab === "dashboard"} onclick={() => (activeTab = "dashboard")}>
            {$t("nav.sensors")}
          </button>
          <button class:active={activeTab === "safety"} onclick={() => (activeTab = "safety")}>
            {$t("nav.safety")}
          </button>
        </nav>
      {/if}
      <select class="lang" bind:value={$locale} aria-label="Language">
        {#each locales as l}
          <option value={l.id}>{l.label}</option>
        {/each}
      </select>
    </div>
  </header>

  {#if !onboarded}
    <Onboarding bind:step={onboardingStep} onComplete={finishOnboarding} />
  {:else if activeTab === "capability"}
    <CapabilityReport />
  {:else if activeTab === "forge"}
    <Forge />
  {:else if activeTab === "dashboard"}
    <Dashboard />
  {:else}
    <SafeLoop />
  {/if}
</main>

<style>
  :global(:root) {
    --forge-void: #05070b;
    --forge-night: #090d13;
    --forge-iron: #10161e;
    --forge-panel: rgba(18, 24, 32, 0.94);
    --forge-panel-raised: rgba(26, 34, 45, 0.94);
    --forge-graphite: #222c38;
    --forge-line: rgba(189, 166, 126, 0.18);
    --forge-line-strong: rgba(214, 168, 93, 0.34);
    --forge-text: #e8edf4;
    --forge-muted: #9aa7b7;
    --forge-dim: #657386;
    --forge-steel: #9caabd;
    --forge-blue: #7eadbe;
    --forge-green: #9dbf91;
    --forge-gold: #d6a85d;
    --forge-copper: #b9754b;
    --forge-red: #c56f73;
    --forge-panel-bg: linear-gradient(180deg, rgba(24, 31, 40, 0.96), rgba(13, 18, 25, 0.94));
    --forge-panel-edge: inset 0 1px 0 rgba(255, 255, 255, 0.045);
    --forge-shadow-panel: 0 18px 45px rgba(0, 0, 0, 0.34), var(--forge-panel-edge);
    --forge-shadow-active: 0 18px 42px rgba(0, 0, 0, 0.3), 0 0 0 1px rgba(214, 168, 93, 0.12);

    --nord-void: var(--forge-void);
    --nord-night: var(--forge-night);
    --nord-deep: var(--forge-iron);
    --nord-surface: var(--forge-panel);
    --nord-surface-2: var(--forge-panel-raised);
    --nord-raised: var(--forge-graphite);
    --nord-border: var(--forge-line);
    --nord-border-card: var(--forge-line);
    --nord-frost: var(--forge-blue);
    --nord-frost-bright: var(--forge-blue);
    --nord-frost-dim: var(--forge-dim);
    --nord-silver: var(--forge-text);
    --nord-mist: var(--forge-muted);
    --nord-dim: var(--forge-dim);
    --nord-aurora: var(--forge-green);
    --nord-aurora-glow: rgba(157, 191, 145, 0.28);
    --nord-ember: var(--forge-copper);
    --nord-ember-bright: var(--forge-gold);
    --nord-twilight: rgba(185, 117, 75, 0.08);
    --nord-danger: var(--forge-red);
  }

  :global(body) {
    margin: 0;
    min-height: 100vh;
    font-family: Inter, "Segoe UI", system-ui, sans-serif;
    color: var(--nord-silver);
    background-color: var(--nord-void);
    background-image:
      radial-gradient(ellipse 120% 70% at 50% -30%, rgba(214, 168, 93, 0.09) 0%, rgba(214, 168, 93, 0.035) 34%, transparent 68%),
      linear-gradient(180deg, rgba(255, 255, 255, 0.018) 0, transparent 14rem),
      linear-gradient(145deg, var(--forge-void) 0%, var(--forge-night) 48%, #07090d 100%);
    background-attachment: fixed;
  }

  main {
    width: min(calc(100vw - 3rem), 1200px);
    margin: 0 auto;
    padding: 1.4rem 0 2.25rem;
  }

  .top {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    gap: 1rem;
    margin-bottom: 1.1rem;
    border-bottom: 1px solid var(--nord-border);
    padding-bottom: 0.9rem;
  }

  h1 {
    margin: 0;
    font-size: 1.7rem;
    font-weight: 800;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--nord-silver);
    text-shadow: 0 1px 0 rgba(0, 0, 0, 0.65);
  }

  .tagline {
    margin: 0.5rem 0 0;
    color: var(--nord-frost-dim);
    font-size: 0.92rem;
    font-style: italic;
    letter-spacing: 0.01em;
  }

  .top-right {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    flex-wrap: wrap;
    justify-content: flex-end;
  }
  .lang {
    background: rgba(10, 16, 28, 0.75);
    color: var(--nord-mist);
    border: 1px solid var(--nord-border);
    border-radius: 8px;
    padding: 0.45rem 0.6rem;
    font-size: 0.8rem;
    font-weight: 600;
    cursor: pointer;
  }
  nav {
    display: flex;
    gap: 0.35rem;
    padding: 0.3rem;
    border-radius: 10px;
    background: rgba(8, 11, 16, 0.76);
    border: 1px solid var(--nord-border);
    backdrop-filter: blur(10px);
  }

  nav button {
    background: transparent;
    color: var(--nord-mist);
    border: none;
    border-radius: 8px;
    padding: 0.5rem 1rem;
    cursor: pointer;
    font-weight: 600;
    font-size: 0.85rem;
    letter-spacing: 0.02em;
  }

  nav button:hover {
    color: var(--nord-silver);
    background: rgba(214, 168, 93, 0.07);
  }

  nav button.active {
    color: var(--forge-gold);
    background: rgba(214, 168, 93, 0.1);
    box-shadow:
      inset 0 0 0 1px rgba(214, 168, 93, 0.22),
      0 0 24px rgba(214, 168, 93, 0.08);
  }
  @media (max-width: 760px) {
    main {
      width: min(calc(100vw - 1.5rem), 1200px);
      padding-top: 1rem;
    }
    .top {
      flex-direction: column;
    }
    .top-right {
      justify-content: flex-start;
    }
  }
</style>
