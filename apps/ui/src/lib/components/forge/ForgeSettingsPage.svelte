<script>
  import { getVersion } from "@tauri-apps/api/app";
  import { onMount } from "svelte";
  import { Check, Languages, Palette, RefreshCw, Settings } from "@lucide/svelte";
  import { locale } from "../../i18n.js";

  let { theme = "command", onThemeChange } = $props();

  let appVersion = $state(null);
  let versionUnavailable = $state(false);

  const themes = [
    { id: "command", name: "Command Deck", description: "Wide control surface" },
    { id: "instrument", name: "Instrument Panel", description: "Industrial side rail" },
    { id: "workshop", name: "Quiet Workshop", description: "Focused daily view" },
  ];

  function selectTheme(nextTheme) {
    onThemeChange?.(nextTheme);
    requestAnimationFrame(() => {
      document.querySelector(`[data-theme-option="${nextTheme}"]`)?.focus();
    });
  }

  onMount(() => {
    let active = true;

    getVersion()
      .then((version) => {
        if (active) appVersion = version;
      })
      .catch(() => {
        if (active) versionUnavailable = true;
      });

    return () => {
      active = false;
    };
  });
</script>

<section class={`forge-settings ${theme}`} aria-labelledby="forge-settings-title">
  <header class="settings-heading">
    <span class="settings-kicker">INTERFACE</span>
    <div class="settings-title">
      <Settings size={34} strokeWidth={1.55} />
      <div>
        <h1 id="forge-settings-title">Settings</h1>
        <p>Personalize the interface without changing how Nidavellir tunes your GPU.</p>
      </div>
    </div>
  </header>

  <div class="settings-list">
    <section class="settings-row" aria-labelledby="theme-setting-title">
      <div class="setting-label">
        <Palette size={23} strokeWidth={1.6} />
        <div>
          <h2 id="theme-setting-title">Interface theme</h2>
          <p>Choose the layout that best fits how you use Nidavellir.</p>
        </div>
      </div>

      <div class="theme-options" role="group" aria-label="Interface theme">
        {#each themes as option, index}
          <button
            type="button"
            class:active={theme === option.id}
            aria-pressed={theme === option.id}
            data-theme-option={option.id}
            onclick={() => selectTheme(option.id)}
          >
            <span class="choice-index">0{index + 1}</span>
            <span class="choice-copy"><strong>{option.name}</strong><small>{option.description}</small></span>
            <span class="choice-check" aria-hidden="true"><Check size={17} strokeWidth={2} /></span>
          </button>
        {/each}
      </div>
    </section>

    <section class="settings-row" aria-labelledby="language-setting-title">
      <div class="setting-label">
        <Languages size={23} strokeWidth={1.6} />
        <div>
          <h2 id="language-setting-title">Interface language</h2>
          <p>Language used by navigation, actions and status messages.</p>
        </div>
      </div>

      <div class="setting-control">
        <div class="language-options" role="group" aria-label="Interface language">
          <button
            type="button"
            class:active={$locale === "en"}
            aria-pressed={$locale === "en"}
            onclick={() => locale.set("en")}
          >
            <span><strong>English</strong><small>Available</small></span>
            <span class="choice-check" aria-hidden="true"><Check size={17} strokeWidth={2} /></span>
          </button>
          <button type="button" aria-pressed="false" disabled>
            <span><strong>Português</strong><small>Em breve</small></span>
          </button>
        </div>
        <p class="setting-note">Português will be enabled when every visible screen has a complete translation.</p>
      </div>
    </section>

    <section class="settings-row" aria-labelledby="updates-setting-title">
      <div class="setting-label">
        <RefreshCw size={23} strokeWidth={1.6} />
        <div>
          <h2 id="updates-setting-title">Updates</h2>
          <p>Desktop build information and update availability.</p>
        </div>
      </div>

      <div class="update-status">
        <div>
          <span>Installed version</span>
          <strong>{appVersion ? `v${appVersion}` : versionUnavailable ? "Desktop app" : "Reading…"}</strong>
        </div>
        <span class="update-mode">MANUAL</span>
        <p>Automatic update checks are not configured for this build.</p>
      </div>
    </section>
  </div>
</section>

<style>
  .forge-settings,
  .forge-settings * {
    box-sizing: border-box;
  }

  .forge-settings {
    --settings-accent: #d0a15f;
    --settings-accent-soft: rgba(208, 161, 95, 0.1);
    --settings-line: rgba(172, 181, 187, 0.28);
    --settings-text: #e2e3e2;
    --settings-muted: #969da2;
    width: min(1120px, 100%);
    margin: 0 auto;
    color: var(--settings-text);
  }

  .forge-settings.instrument {
    --settings-accent: #d3b478;
    --settings-accent-soft: rgba(211, 180, 120, 0.09);
    --settings-line: rgba(119, 125, 122, 0.42);
    --settings-text: #d9dad8;
    --settings-muted: #929795;
    font-family: Bahnschrift, "Arial Narrow", "Segoe UI", sans-serif;
  }

  .forge-settings.workshop {
    --settings-accent: #d29063;
    --settings-accent-soft: rgba(210, 144, 99, 0.08);
    --settings-line: rgba(100, 105, 104, 0.42);
    --settings-text: #eeece8;
    --settings-muted: #929795;
  }

  .settings-heading {
    border-bottom: 1px solid var(--settings-line);
    padding: 12px 0 34px;
  }

  .settings-kicker {
    color: var(--settings-accent);
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.18em;
  }

  .settings-title {
    display: flex;
    align-items: flex-start;
    gap: 17px;
    margin-top: 13px;
  }

  .settings-title > :global(svg) {
    flex: 0 0 auto;
    margin-top: 4px;
    color: var(--settings-accent);
  }

  h1,
  h2,
  p {
    text-wrap: pretty;
  }

  h1 {
    margin: 0;
    color: var(--settings-text);
    font-size: clamp(30px, 4vw, 43px);
    font-weight: 540;
    letter-spacing: -0.025em;
    line-height: 1.05;
    text-wrap: balance;
  }

  .instrument h1 {
    font-family: "Bahnschrift Condensed", "Arial Narrow", Bahnschrift, sans-serif;
    font-size: clamp(36px, 5vw, 50px);
    font-weight: 700;
    letter-spacing: 0.02em;
    text-transform: uppercase;
  }

  .settings-title p {
    max-width: 650px;
    margin: 9px 0 0;
    color: var(--settings-muted);
    font-size: 15px;
    line-height: 1.6;
  }

  .settings-list {
    border-bottom: 1px solid var(--settings-line);
  }

  .settings-row {
    display: grid;
    grid-template-columns: minmax(0, 0.72fr) minmax(0, 1.55fr);
    gap: clamp(32px, 6vw, 84px);
    border-top: 1px solid var(--settings-line);
    padding: 34px 0;
  }

  .settings-row:first-child {
    border-top: 0;
  }

  .setting-label {
    display: grid;
    grid-template-columns: 28px 1fr;
    align-content: start;
    gap: 13px;
  }

  .setting-label > :global(svg) {
    margin-top: 1px;
    color: var(--settings-accent);
  }

  .setting-label h2 {
    margin: 0;
    color: var(--settings-text);
    font-size: 16px;
    font-weight: 620;
    line-height: 1.3;
  }

  .instrument .setting-label h2 {
    font-family: "Bahnschrift Condensed", "Arial Narrow", Bahnschrift, sans-serif;
    font-size: 19px;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }

  .setting-label p,
  .setting-note,
  .update-status p {
    margin: 7px 0 0;
    color: var(--settings-muted);
    font-size: 13px;
    line-height: 1.55;
  }

  .theme-options,
  .language-options {
    display: grid;
    overflow: hidden;
    border: 1px solid var(--settings-line);
  }

  .theme-options {
    grid-template-columns: repeat(3, minmax(0, 1fr));
  }

  .language-options {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }

  .theme-options button,
  .language-options button {
    display: grid;
    min-width: 0;
    min-height: 78px;
    align-items: center;
    gap: 12px;
    border: 0;
    border-left: 1px solid var(--settings-line);
    padding: 14px 16px;
    background: rgba(5, 8, 9, 0.12);
    color: var(--settings-muted);
    text-align: left;
    cursor: pointer;
    transition:
      background-color 150ms ease,
      color 150ms ease,
      box-shadow 150ms ease,
      transform 100ms ease;
  }

  .theme-options button {
    grid-template-columns: auto minmax(0, 1fr) auto;
    min-height: 108px;
  }

  .language-options button {
    grid-template-columns: minmax(0, 1fr) auto;
  }

  .theme-options button:first-child,
  .language-options button:first-child {
    border-left: 0;
  }

  .theme-options button:hover:not(:disabled),
  .language-options button:hover:not(:disabled) {
    background: var(--settings-accent-soft);
    color: var(--settings-text);
  }

  .theme-options button.active,
  .language-options button.active {
    background: var(--settings-accent-soft);
    color: var(--settings-text);
    box-shadow: inset 0 -2px 0 var(--settings-accent);
  }

  .theme-options button:focus-visible,
  .language-options button:focus-visible {
    position: relative;
    z-index: 1;
    outline: 2px solid var(--settings-accent);
    outline-offset: -3px;
  }

  .theme-options button:active:not(:disabled),
  .language-options button:active:not(:disabled) {
    transform: scale(0.96);
  }

  .theme-options button:disabled,
  .language-options button:disabled {
    cursor: not-allowed;
    opacity: 0.5;
  }

  .choice-index {
    color: var(--settings-accent);
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.1em;
  }

  .choice-copy,
  .language-options button > span:first-child {
    display: flex;
    min-width: 0;
    flex-direction: column;
    gap: 5px;
  }

  .choice-copy strong,
  .language-options strong {
    color: inherit;
    font-size: 14px;
    font-weight: 620;
    line-height: 1.25;
  }

  .choice-copy small,
  .language-options small {
    color: var(--settings-muted);
    font-size: 11px;
    line-height: 1.4;
  }

  .choice-check {
    display: grid;
    width: 25px;
    height: 25px;
    place-items: center;
    border: 1px solid var(--settings-line);
    border-radius: 50%;
    color: transparent;
  }

  button.active .choice-check {
    border-color: var(--settings-accent);
    color: var(--settings-accent);
  }

  .update-status {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    align-items: center;
    gap: 8px 20px;
    min-height: 78px;
    border-block: 1px solid var(--settings-line);
    padding: 13px 16px;
  }

  .update-status > div {
    display: flex;
    min-width: 0;
    align-items: baseline;
    justify-content: space-between;
    gap: 18px;
  }

  .update-status > div span {
    color: var(--settings-muted);
    font-size: 12px;
  }

  .update-status > div strong {
    color: var(--settings-text);
    font-size: 14px;
    font-weight: 620;
  }

  .update-mode {
    border: 1px solid var(--settings-line);
    padding: 5px 8px;
    color: var(--settings-accent);
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.12em;
  }

  .update-status p {
    grid-column: 1 / -1;
    margin-top: 0;
  }

  @media (max-width: 900px) {
    .settings-row {
      grid-template-columns: 1fr;
      gap: 22px;
    }
  }

  @media (max-width: 680px) {
    .settings-heading {
      padding-top: 0;
    }

    .settings-title {
      gap: 12px;
    }

    .theme-options,
    .language-options {
      grid-template-columns: 1fr;
    }

    .theme-options button,
    .language-options button {
      border-top: 1px solid var(--settings-line);
      border-left: 0;
    }

    .theme-options button:first-child,
    .language-options button:first-child {
      border-top: 0;
    }

    .update-status > div {
      align-items: flex-start;
      flex-direction: column;
      gap: 4px;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .theme-options button,
    .language-options button {
      transition: none;
    }
  }
</style>
