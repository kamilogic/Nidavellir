<script>
  import Onboarding from "./lib/views/Onboarding.svelte";
  import Dashboard from "./lib/views/Dashboard.svelte";
  import SafeLoop from "./lib/views/SafeLoop.svelte";
  import Forge from "./lib/views/Forge.svelte";
  import { t, locale, locales } from "./lib/i18n.js";

  function initialOnboarded() {
    try {
      return localStorage.getItem("nidavellir-gpu-onboarded") === "true";
    } catch {
      return false;
    }
  }

  let onboarded = $state(initialOnboarded());
  let onboardingStep = $state(1);
  let activeTab = $state("forge");

  function finishOnboarding(_goal) {
    onboarded = true;
    activeTab = "forge";
    try {
      localStorage.setItem("nidavellir-gpu-onboarded", "true");
    } catch {}
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
          <button class:active={activeTab === "dashboard"} onclick={() => (activeTab = "dashboard")}>
            {$t("nav.sensors")}
          </button>
          <button class:active={activeTab === "safety"} onclick={() => (activeTab = "safety")}>
            {$t("nav.safety")}
          </button>
        </nav>
      {/if}
      {#if locales.length > 1}
        <select class="lang" bind:value={$locale} aria-label="Language">
          {#each locales as l}
            <option value={l.id}>{l.label}</option>
          {/each}
        </select>
      {/if}
    </div>
  </header>

  {#if !onboarded}
    <Onboarding bind:step={onboardingStep} onComplete={finishOnboarding} />
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
    --forge-void: #0b0f14;
    --forge-night: #0b0f14;
    --forge-iron: #10161e;
    --forge-panel: #141b24;
    --forge-panel-raised: #1b232e;
    --forge-graphite: #222c38;
    --forge-line: rgba(189, 166, 126, 0.18);
    --forge-line-strong: rgba(214, 168, 93, 0.34);
    --forge-border-neutral: #2a3441;
    --forge-text: #e8ecf1;
    --forge-muted: #8a96a3;
    --forge-dim: #5c6774;
    --forge-steel: #9caabd;
    --forge-blue: #7eadbe;
    --forge-green: #33c481;
    --forge-green-bg: #1f4436;
    --forge-gold: #e3a83b;
    --forge-copper: #b9754b;
    --forge-red: #e2545a;
    --forge-red-bg: #3a2226;
    --forge-teal: #3fd0c9;
    --forge-ink: #0b0f14;
    --forge-panel-bg: var(--forge-panel);
    --forge-panel-edge: inset 0 0 0 0 transparent;
    --forge-shadow-panel: none;
    --forge-shadow-active: 0 0 0 1px rgba(227, 168, 59, 0.3);

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
    --nord-aurora-glow: rgba(51, 196, 129, 0.22);
    --nord-ember: var(--forge-copper);
    --nord-ember-bright: var(--forge-gold);
    --nord-twilight: rgba(185, 117, 75, 0.08);
    --nord-danger: var(--forge-red);
  }

  :global(body) {
    margin: 0;
    min-height: 100vh;
    font-family: "Source Sans Pro", Inter, "Segoe UI", system-ui, sans-serif;
    color: var(--nord-silver);
    background-color: var(--nord-void);
  }

  :global(html) {
    -webkit-font-smoothing: antialiased;
    -moz-osx-font-smoothing: grayscale;
  }

  :global(h1, h2, h3, h4) {
    text-wrap: balance;
  }

  :global(p, li, small) {
    text-wrap: pretty;
  }

  :global(button) {
    min-height: 2.5rem;
    transition-property: background-color, border-color, color, box-shadow, scale;
    transition-duration: 150ms;
    transition-timing-function: ease-out;
  }

  :global(button:active:not(:disabled)) {
    scale: 0.96;
  }

  :global(button:focus-visible),
  :global(a:focus-visible),
  :global(summary:focus-visible),
  :global(select:focus-visible),
  :global(input:focus-visible) {
    outline: 2px solid rgba(214, 168, 93, 0.78);
    outline-offset: 3px;
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
    background: var(--forge-panel-raised);
    color: var(--nord-mist);
    border: none;
    border-radius: 999px;
    padding: 0.45rem 0.6rem;
    font-size: 0.8rem;
    font-weight: 600;
    cursor: pointer;
  }
  nav {
    display: flex;
    gap: 0.4rem;
  }

  nav button {
    background: var(--forge-panel-raised);
    color: var(--nord-mist);
    border: none;
    border-radius: 999px;
    padding: 0.5rem 1.05rem;
    cursor: pointer;
    font-weight: 600;
    font-size: 0.8rem;
    letter-spacing: 0.02em;
    text-transform: uppercase;
  }

  nav button:hover {
    color: var(--nord-silver);
  }

  nav button.active {
    color: var(--forge-ink);
    background: var(--forge-gold);
    font-weight: 700;
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
