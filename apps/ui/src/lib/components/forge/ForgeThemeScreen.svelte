<script>
  import {
    Activity,
    Anvil,
    ChevronDown,
    ChevronRight,
    CircleGauge,
    Cpu,
    Fan,
    Feather,
    Gauge,
    Hammer,
    Settings,
    ShieldCheck,
    Star,
    Thermometer,
    Trash2,
    TriangleAlert,
    X,
    Zap,
  } from "@lucide/svelte";
  import ForgeSettingsPage from "./ForgeSettingsPage.svelte";
  import ForgeProgress from "./ForgeProgress.svelte";
  import TelemetrySpark from "./TelemetrySpark.svelte";
  import commandGpu from "../../assets/themes/command-gpu.png";
  import commandMark from "../../assets/themes/nidavellir-mark.png";
  import instrumentGauge from "../../assets/themes/instrument-gauge.png";
  import instrumentLockup from "../../assets/themes/instrument-lockup.png";
  import copperPlate from "../../assets/themes/copper-plate.png";
  import forgeTexture from "../../assets/themes/forge-texture.png";

  let {
    children,
    theme = "command",
    activeView = "forge",
    hardware = null,
    gpu = null,
    sparks = null,
    powerSweep = null,
    safeLoop = null,
    applied = null,
    forgeMode = "standard",
    powerRunning = false,
    fullResetBusy = false,
    fullResetFeedback = null,
    onThemeChange,
    onForgeModeChange,
    onStartPower,
    onRecoverContinue,
    onStopPower,
    onResumePower,
    onApplyPower,
    onReportProfileUnstable,
    onFullReset,
    onDismissFullResetFeedback,
    onViewChange,
  } = $props();

  let resetConfirmOpen = $state(false);
  let resetDialog = $state(null);
  let resetTrigger = $state(null);
  let resetCancelButton = $state(null);
  let expandedProfile = $state(null);

  const profileMeta = [
    {
      key: "godforge",
      name: "Godforge",
      line: "Maximum sustainable performance",
      summary: "Forged at the edge of sustainable performance for the highest clock this GPU proved it can hold.",
    },
    {
      key: "brokkrs",
      name: "Brokkr’s Best",
      line: "Balanced daily performance",
      summary: "Tempered for daily use, balancing strong performance with lower power, heat and noise.",
    },
    {
      key: "deep_calm",
      name: "Deep Calm",
      line: "Maximum efficiency",
      summary: "Refined for cool, quiet efficiency and the most useful performance from every watt.",
    },
  ];

  const primaryGpu = $derived(hardware?.gpu?.[0] ?? null);
  const gpuName = $derived(primaryGpu?.model ?? "NVIDIA GPU");
  const gpuConnectionLabel = $derived(primaryGpu ? "GPU Connected" : "Waiting for GPU");
  const hasCompleteProfileSet = $derived(
    Boolean(powerSweep?.godforge && powerSweep?.brokkrs && powerSweep?.deep_calm),
  );
  const isUndervolt = $derived(Boolean(powerSweep?.is_undervolt));
  const profilesReady = $derived(
    Boolean(
      hasCompleteProfileSet &&
        !powerRunning &&
        (powerSweep?.frontier_complete || !powerSweep?.is_undervolt),
    ),
  );
  const profilesQualified = $derived(
    Boolean(profilesReady && (!isUndervolt || powerSweep?.profiles_qualified)),
  );
  const hasForgeRun = $derived(Boolean(powerSweep && powerSweep.phase !== "idle"));
  const forgePaused = $derived(powerSweep?.phase === "paused");
  const resumeAvailable = $derived(Boolean(forgePaused && powerSweep?.resume_available));
  const state = $derived.by(() => {
    if (powerRunning) return "FORGING";
    if (profilesReady && profileMeta.some((profile) => appliedMatches(profile))) return "FORGED";
    if (profilesReady) return "REFINED";
    return "RAW";
  });
  const activeKey = $derived.by(() => {
    return profileMeta.find((profile) => appliedMatches(profile))?.key ?? null;
  });
  const activeName = $derived(profileMeta.find((item) => item.key === activeKey)?.name ?? "Stock");
  const safeLoopKnown = $derived(Boolean(safeLoop));
  const recoveryPending = $derived(Boolean(safeLoop?.recovery_pending_ack));
  const protectedState = $derived(
    Boolean(safeLoopKnown && !(safeLoop?.safe_mode || safeLoop?.state === "unstable")),
  );
  const protectionLabel = $derived(
    !safeLoopKnown ? "Awaiting" : recoveryPending ? "Needs Attention" : protectedState ? "Protected" : "Review",
  );
  const protectionMessage = $derived(
    !safeLoopKnown
      ? "Waiting for Safe Loop status."
      : recoveryPending
        ? (safeLoop?.pending_forge_incident?.message ?? "Interrupted Forge requires acknowledgement before continuing.")
      : protectedState
        ? "Your GPU is monitored and ready."
        : "Safe Loop needs your attention.",
  );

  function finite(value) {
    if (value == null || value === "") return null;
    const number = Number(value);
    if (Number.isFinite(number)) return number;
    return null;
  }

  const temperature = $derived(finite(gpu?.temperature_c));
  const power = $derived(finite(gpu?.power_w));
  const clock = $derived(finite(gpu?.core_clock_mhz));
  const memory = $derived(finite(gpu?.memory_clock_mhz));
  const vramTotal = $derived(finite(gpu?.vram_total_mb));
  const voltage = $derived(finite(gpu?.voltage_mv));
  const fan = $derived(finite(gpu?.fan_speed_pct));
  const usage = $derived(finite(gpu?.utilization_pct));

  function values(key) {
    const live = sparks?.[key] ?? [];
    return live.length > 1 ? live : [];
  }

  function display(value, digits = 0) {
    return value == null ? "—" : Number(value).toFixed(digits);
  }

  function vramCapacity(value) {
    if (value == null) return "Capacity unavailable";
    const gigabytes = value / 1024;
    return `${gigabytes >= 10 ? gigabytes.toFixed(0) : gigabytes.toFixed(1)} GB total`;
  }

  function pointFor(key) {
    return powerSweep?.[key] ?? null;
  }

  function sameNumber(a, b) {
    if (a == null || b == null) return false;
    return Number(a) === Number(b);
  }

  function normalize(value) {
    return String(value ?? "").toLowerCase().replace(/[^a-z0-9]/g, "");
  }

  function appliedMatches(profile) {
    const point = pointFor(profile.key);
    if (!profilesReady || !point || !applied?.core) return false;
    if (normalize(applied.label) !== normalize(profile.name)) return false;
    const targetClock = point.target_clock_mhz ?? point.clock_mhz;
    if (!sameNumber(applied.core.freq_mhz, targetClock)) return false;
    const applyVoltage = point.vf_table_voltage_mv ?? point.voltage_mv;
    return applyVoltage == null || sameNumber(applied.core.voltage_mv, applyVoltage);
  }

  function profilePower(point) {
    const sustainedP99 = finite(point?.power_p99_w);
    if (sustainedP99 != null && sustainedP99 > 0) return sustainedP99;
    const peak = finite(point?.max_power_w);
    if (peak != null && peak > 0) return peak;
    const average = finite(point?.power_w);
    return average != null && average > 0 ? average : null;
  }

  function profileTarget(point) {
    const target = finite(point?.target_clock_mhz ?? point?.clock_mhz);
    return target == null ? "—" : `${target.toFixed(0)} MHz`;
  }

  function profilePeakPowerText(point) {
    const peak = finite(point?.max_power_w);
    if (peak != null && peak > 0) return `${peak.toFixed(0)} W`;
    const measuredPower = profilePower(point);
    return measuredPower == null ? "—" : `${measuredPower.toFixed(0)} W`;
  }

  function profileVoltage(point) {
    const voltage = finite(point?.vf_table_voltage_mv ?? point?.voltage_mv);
    return voltage == null ? "—" : `${voltage.toFixed(0)} mV`;
  }

  function profileEfficiency(point) {
    const efficiency = finite(point?.perf_per_watt);
    return efficiency == null ? "—" : `${efficiency.toFixed(1)} MHz/W`;
  }

  function profileEfficiencyVsStock(point) {
    const stockClock = finite(powerSweep?.stock_clock_mhz);
    const stockPower = finite(powerSweep?.stock_power_p99_w);
    const efficiency = finite(point?.perf_per_watt);
    if (stockClock == null || stockClock <= 0 || stockPower == null || stockPower <= 0 || efficiency == null) {
      return "—";
    }
    const delta = ((efficiency / (stockClock / stockPower)) - 1) * 100;
    return `${delta >= 0 ? "+" : ""}${delta.toFixed(0)}%`;
  }

  function profileExpanded(key) {
    return profilesReady && expandedProfile === key;
  }

  function toggleProfile(key) {
    if (!profilesReady) return;
    expandedProfile = expandedProfile === key ? null : key;
  }

  function profileStatus(key) {
    if (!profilesReady) return "Available after Forge";
    if (profileActive(key)) return "Applied";
    if (canApply(key)) return "Ready";
    return profilesQualified ? "Measured" : "Qualification pending";
  }

  function profileActive(key) {
    return activeKey === key;
  }

  function canApply(key) {
    const point = pointFor(key);
    if (!profilesReady || !point || profileActive(key)) return false;
    if (!isUndervolt) return true;
    const sustainedP99 = finite(point.power_p99_w);
    return Boolean(
      profilesQualified &&
        point.apply_qualified &&
        sustainedP99 != null &&
        sustainedP99 > 0,
    );
  }

  function profileAction(key) {
    if (!canApply(key)) return;
    onApplyPower?.(key);
  }

  function runForge() {
    if (powerRunning) return;
    if (forgePaused) {
      if (resumeAvailable) onResumePower?.();
      return;
    }
    if (recoveryPending) {
      onRecoverContinue?.(forgeMode);
      return;
    }
    onStartPower?.(forgeMode);
  }

  function reportProfile(profile) {
    if (powerRunning || !pointFor(profile.key)) return;
    onReportProfileUnstable?.(profile.key);
  }

  function selectMode(event) {
    onForgeModeChange?.(event.currentTarget.value);
  }

  function chooseTheme(next) {
    onThemeChange?.(next);
  }

  function navigate(target) {
    onViewChange?.(["forge", "advanced", "settings"].includes(target) ? target : "forge");
  }

  function openResetConfirmation(event) {
    if (fullResetBusy) return;
    resetTrigger = event.currentTarget;
    resetConfirmOpen = true;
    requestAnimationFrame(() => {
      resetDialog?.showModal();
      resetCancelButton?.focus();
    });
  }

  function closeResetConfirmation() {
    if (fullResetBusy) return;
    resetDialog?.close();
    resetConfirmOpen = false;
    requestAnimationFrame(() => resetTrigger?.focus());
  }

  async function confirmFullReset() {
    if (fullResetBusy) return;
    await onFullReset?.();
    resetDialog?.close();
    resetConfirmOpen = false;
    requestAnimationFrame(() => resetTrigger?.focus());
  }

  function handleResetDialogCancel(event) {
    event.preventDefault();
    closeResetConfirmation();
  }

  function handleResetDialogKeydown(event) {
    if (event.key !== "Escape") return;
    event.preventDefault();
    closeResetConfirmation();
  }

  function elapsed() {
    const milliseconds = finite(powerSweep?.elapsed_ms);
    if (milliseconds == null) return "—";
    const total = Math.floor(milliseconds / 1000);
    const hours = Math.floor(total / 3600).toString().padStart(2, "0");
    const minutes = Math.floor((total % 3600) / 60).toString().padStart(2, "0");
    const seconds = (total % 60).toString().padStart(2, "0");
    return `${hours}:${minutes}:${seconds}`;
  }

  const testsCompleted = $derived(powerSweep ? (powerSweep.points?.length ?? 0) : null);

  const backgroundStyle = `--forge-texture: url('${forgeTexture}')`;
</script>

{#snippet profileDisclosure(profile, variant)}
  {@const point = pointFor(profile.key)}
  <article
    class={`forge-profile-card ${variant}`}
    class:active={profileActive(profile.key)}
    class:expanded={profileExpanded(profile.key)}
    class:preview={!profilesReady}
  >
    <button
      class="profile-disclosure"
      type="button"
      onclick={() => toggleProfile(profile.key)}
      disabled={!profilesReady}
      aria-expanded={profileExpanded(profile.key)}
      aria-controls={`profile-details-${variant}-${profile.key}`}
    >
      <span class="profile-card-icon">
        {#if profile.key === "godforge"}<Hammer size={27} />
        {:else if profile.key === "brokkrs"}<Star size={29} />
        {:else}<Feather size={27} />{/if}
      </span>
      <span class="profile-card-copy">
        <strong>{profile.name}</strong>
        <small>{profile.summary}</small>
      </span>
      <span class="profile-card-state">
        <small>{profileStatus(profile.key)}</small>
        {#if profilesReady}<ChevronDown size={21} strokeWidth={1.7} />{/if}
      </span>
    </button>

    {#if profileExpanded(profile.key)}
      <div class="profile-card-details" id={`profile-details-${variant}-${profile.key}`}>
        <div class="profile-card-metrics">
          <span><small>Target clock</small><strong>{profileTarget(point)}</strong></span>
          <span><small>Target voltage</small><strong>{profileVoltage(point)}</strong></span>
          <span><small>Maximum power</small><strong>{profilePeakPowerText(point)}</strong></span>
          <span><small>Efficiency vs stock</small><strong>{profileEfficiencyVsStock(point)}</strong></span>
          <span><small>Efficiency</small><strong>{profileEfficiency(point)}</strong></span>
        </div>
        <div class="profile-card-actions">
          <button class="profile-apply" type="button" onclick={() => profileAction(profile.key)} disabled={!canApply(profile.key)}>
            {#if profileActive(profile.key)}<ShieldCheck size={17} />Applied{:else}Apply {profile.name}{/if}
          </button>
          <button class="field-failure" type="button" onclick={() => reportProfile(profile)} disabled={powerRunning || !point}>Mark unstable</button>
        </div>
      </div>
    {/if}
  </article>
{/snippet}

{#snippet fullResetControl()}
  {#if !powerRunning}
    <section class="full-reset-strip" aria-label="Reset total">
      <div class="full-reset-copy">
        <span>RECUPERAÇÃO DESTRUTIVA</span>
        <strong>Reset Total</strong>
        <p>Volta a GPU para stock, limpa a forja ativa e prepara a próxima execução como Clean Run.</p>
      </div>
      <button class="full-reset-action" type="button" onclick={openResetConfirmation} disabled={fullResetBusy}>
        <Trash2 size={18} strokeWidth={1.7} />
        <span>Reset Total</span>
      </button>
    </section>
  {/if}
{/snippet}

<section class={`forge-theme-screen ${theme}`} style={backgroundStyle}>
  {#if theme === "command"}
    <header class="command-header">
      <button class="brand command-brand" onclick={() => navigate("forge")} aria-label="Nidavellir Forge home">
        <img src={commandMark} alt="" />
        <span>NIDAVELLIR</span>
      </button>
      <nav class="command-nav" aria-label="Primary navigation">
        <button class:active={activeView === "forge"} onclick={() => navigate("forge")}>Forge</button>
        <button class:active={activeView === "settings"} onclick={() => navigate("settings")}>Settings</button>
      </nav>
      <div class="connected" class:pending={!primaryGpu}>
        <i></i>
        <span><strong>{gpuConnectionLabel}</strong><small>{gpuName}</small></span>
      </div>
    </header>

    {#if activeView === "settings"}
      <div class="command-page"><ForgeSettingsPage {theme} onThemeChange={chooseTheme} /></div>
    {:else if activeView === "advanced"}
      <div class="command-page">{@render children?.()}</div>
    {:else}
    <div class="command-body">
      <section class="command-hero">
        <div class="command-gpu-wrap"><img class="command-gpu" src={commandGpu} alt="NVIDIA graphics card" /></div>
        <div class="command-identity">
          <span class="eyebrow">GPU</span>
          <h1>{gpuName.replace(/^NVIDIA\s+/i, "")}</h1>
          <div class="state-status">
            <div><span>STATE</span><strong>{state}</strong></div>
            <div><span>STATUS</span><strong class="protected" class:pending={!safeLoopKnown}><ShieldCheck size={38} />{protectionLabel}</strong></div>
          </div>
          <p>{state === "FORGED" ? "Ready for daily use." : state === "FORGING" ? "Building safe profiles now." : "Ready to begin a supervised forge."}</p>
        </div>
        <div class="command-cta">
          <button class="plate-button" onclick={runForge} disabled={powerRunning || (forgePaused && !resumeAvailable)}>
            <img src={copperPlate} alt="" />
            <span>{powerRunning ? "Forging…" : resumeAvailable ? "Resume Forge" : forgePaused ? "Resume unavailable" : recoveryPending ? "Review & Continue" : "Forge GPU"}</span>
          </button>
          <div class="command-run-mode">
            <label for="command-run-mode">
              <span>RUN</span>
              <select id="command-run-mode" value={forgeMode} onchange={selectMode} disabled={powerRunning || forgePaused}>
                <option value="clean">Clean Run · ignores prior boundaries</option>
                <option value="standard">Standard · compact proof</option>
                <option value="long">Long · exhaustive proof</option>
              </select>
              <ChevronDown size={18} />
            </label>
            <small>{forgeMode === "long" ? "Explicit long run · exhaustive evidence" : forgeMode === "clean" ? "Organic learning · compact Standard proof" : "Recommended · compact proof to completion"}</small>
          </div>
          {#if profilesReady}
            <span class="refine">Profiles forged from measured hardware data <ShieldCheck size={23} /></span>
          {/if}
        </div>
      </section>

      {#if hasForgeRun}
        <ForgeProgress
          {powerSweep}
          {powerRunning}
          {safeLoop}
          {forgeMode}
          {onStopPower}
          {onStartPower}
          {onRecoverContinue}
          {onResumePower}
        />
      {/if}

      <section class="command-telemetry" aria-label="Live GPU telemetry">
        {#each [
          { key: "temp", label: "Temperature", value: temperature, unit: "°C" },
          { key: "power", label: "Power", value: power, unit: "W" },
          { key: "core", label: "Clock", value: clock, unit: "MHz" },
          { key: "mem", label: "VRAM", value: memory, unit: "MHz", hint: vramCapacity(vramTotal) },
          { key: "voltage", label: "Voltage", value: voltage, unit: "mV", hint: voltage == null ? "Sensor not exposed" : "Live core voltage" },
          { key: "fan", label: "Fan", value: fan, unit: "%", hint: fan == null ? "Sensor not exposed" : "Average duty" },
          { key: "usage", label: "Utilization", value: usage, unit: "%" },
        ] as metric}
          <article class="command-metric">
            <div class="metric-title">
              {#if metric.key === "temp"}<Thermometer size={29} />
              {:else if metric.key === "power"}<Zap size={29} />
              {:else if metric.key === "core"}<Gauge size={29} />
              {:else if metric.key === "fan"}<Fan size={29} />
              {:else if metric.key === "voltage"}<Activity size={29} />
              {:else}<Cpu size={29} />{/if}
              <span>{metric.label}</span>
            </div>
            <div class="metric-reading"><strong>{display(metric.value)}</strong><span>{metric.unit}</span></div>
            {#if metric.hint}<small>{metric.hint}</small>{/if}
            <TelemetrySpark values={values(metric.key)} color="#80bd31" fill="rgba(128, 189, 49, 0.08)" height={48} />
          </article>
        {/each}
      </section>

      <section class="command-profiles" class:profile-overview={!profilesReady}>
        <div class="section-label"><span>{profilesReady ? "FORGED PROFILES" : "PROFILE OVERVIEW"}</span></div>
        {#each profileMeta as profile}
          {@render profileDisclosure(profile, "command")}
        {/each}
      </section>

      {@render fullResetControl()}

      <button class="command-advanced" onclick={() => navigate("advanced")}
        ><span>Advanced diagnostics <ChevronRight size={22} /></span><small>Live log, Sentinel, Game Trace and manual point</small><ChevronDown size={24} /></button
      >
    </div>
    {/if}
  {:else if theme === "instrument"}
    <div class="instrument-frame">
      <aside class="instrument-rail">
        <button class="instrument-lockup" onclick={() => navigate("forge")} aria-label="Nidavellir Forge home"><img src={instrumentLockup} alt="Nidavellir Forge" /></button>
        <nav aria-label="Primary navigation">
          <button class:active={activeView === "forge"} onclick={() => navigate("forge")}><Anvil size={31} /><span>Forge</span></button>
          <button class:active={activeView === "settings"} onclick={() => navigate("settings")}><Settings size={31} /><span>Settings</span></button>
        </nav>
        <div class="rail-gpu"><span class="nvidia-mark">NVIDIA</span><strong>{gpuName.replace(/^NVIDIA\s+/i, "")}</strong><small>{primaryGpu?.driver ?? "Waiting for driver"}</small></div>
      </aside>

      <main class="instrument-content" class:diagnostics-view={activeView !== "forge"}>
        {#if activeView === "settings"}
          <div class="instrument-page"><ForgeSettingsPage {theme} onThemeChange={chooseTheme} /></div>
        {:else if activeView === "advanced"}
          <div class="instrument-page">{@render children?.()}</div>
        {:else}
        <div class="instrument-main-column">
          <section class="instrument-intro">
            <span class="instrument-kicker"><i></i> ACTIVE GPU</span>
            <h1>{gpuName}</h1>
            <p><ShieldCheck size={34} /> {protectionMessage}</p>
          </section>

          <section class="gauge-layout">
            <div class="gauge-side left">
              <div><span>Temperature</span><strong>{display(temperature)}</strong><small>°C</small><TelemetrySpark values={values("temp")} color="#627d90" fill="rgba(98, 125, 144, 0.04)" height={30} /></div>
              <div><span>Power</span><strong>{display(power)}</strong><small>W</small><TelemetrySpark values={values("power")} color="#627d90" fill="rgba(98, 125, 144, 0.04)" height={30} /></div>
              <div><span>Fan</span><strong>{display(fan)}</strong><small>{fan == null ? "Not exposed" : "%"}</small><TelemetrySpark values={values("fan")} color="#7a9748" fill="rgba(122, 151, 72, 0.04)" height={30} /></div>
            </div>
            <div class="gauge-bezel">
              <img src={instrumentGauge} alt="Thermal and power gauge" />
              <div class="gauge-value"><span>THERMAL / POWER</span><strong>{display(temperature)}</strong><small>°C</small></div>
            </div>
            <div class="gauge-side right">
              <div><span>Clock</span><strong>{display(clock)}</strong><small>MHz</small><TelemetrySpark values={values("core")} color="#627d90" fill="rgba(98, 125, 144, 0.04)" height={30} /></div>
              <div><span>VRAM</span><strong>{display(memory)}</strong><small>MHz · {vramCapacity(vramTotal)}</small><TelemetrySpark values={values("mem")} color="#627d90" fill="rgba(98, 125, 144, 0.04)" height={30} /></div>
              <div><span>Voltage</span><strong>{display(voltage)}</strong><small>{voltage == null ? "Not exposed" : "mV"}</small><TelemetrySpark values={values("voltage")} color="#627d90" fill="rgba(98, 125, 144, 0.04)" height={30} /></div>
              <div><span>Utilization</span><strong>{display(usage)}</strong><small>%</small><TelemetrySpark values={values("usage")} color="#7a9748" fill="rgba(122, 151, 72, 0.04)" height={30} /></div>
            </div>
          </section>

          {#if hasForgeRun}
            <ForgeProgress
              {powerSweep}
              {powerRunning}
              {safeLoop}
              {forgeMode}
              {onStopPower}
              {onStartPower}
              {onRecoverContinue}
              {onResumePower}
            />
          {/if}

          <section class="recommended-panel">
            <span class="instrument-kicker">{profilesReady ? "FORGED PROFILES" : "PROFILE OVERVIEW"}</span>
            <div class="instrument-profile-grid" class:ready={profilesReady}>
              {#each profileMeta as profile}
                {@render profileDisclosure(profile, "instrument")}
              {/each}
            </div>
          </section>
          {@render fullResetControl()}
        </div>

        <aside class="instrument-action-panel">
          <span class="panel-kicker">PRIMARY ACTION</span>
          <button class="instrument-forge" onclick={runForge} disabled={powerRunning || (forgePaused && !resumeAvailable)}><Anvil size={42} /><strong>{powerRunning ? "FORGING…" : resumeAvailable ? "RESUME FORGE" : forgePaused ? "RESUME UNAVAILABLE" : recoveryPending ? "REVIEW & CONTINUE" : "FORGE GPU"}</strong></button>
          <div class="mode-block">
            <label for="instrument-mode">MODE</label>
            <select id="instrument-mode" value={forgeMode} onchange={selectMode} disabled={powerRunning || forgePaused}>
              <option value="clean">Clean Run — ignores prior boundaries</option>
              <option value="standard">Standard — compact proof</option>
              <option value="long">Long — exhaustive proof</option>
            </select>
            <p>Builds safe profiles while you use your PC.</p>
            <small>Learns your GPU, tests limits safely and creates personalized profiles.</small>
          </div>
          <div class="safe-loop-block">
            <span>SAFE LOOP</span>
            <strong class:pending={!safeLoopKnown}><ShieldCheck size={64} /> {protectionLabel.toUpperCase()}</strong>
            <p>Continuous monitoring. Automatic recovery if anything leaves safe limits.</p>
          </div>
          <div class="instrument-runtime">
            <div><CircleGauge size={35} /><span>ACTIVE TIME<strong>{elapsed()}</strong></span></div>
            <div><span>TESTS COMPLETED<strong>{testsCompleted ?? "—"}</strong></span></div>
          </div>
        </aside>

        <button class="instrument-advanced" onclick={() => navigate("advanced")}><Activity size={40} /><span><strong>ADVANCED DETAILS</strong><small>Live terminal, Sentinel, Game Trace and manual point</small></span><small>Open workspace</small><ChevronRight size={22} /></button>
        {/if}
      </main>
    </div>
  {:else}
    <header class="workshop-header">
      <button class="brand workshop-brand" onclick={() => navigate("forge")}><Anvil size={28} /><span>NIDAVELLIR</span></button>
      <nav>
        <button class:active={activeView === "forge"} onclick={() => navigate("forge")}>Forge</button>
      </nav>
      <button class="workshop-settings" class:active={activeView === "settings"} onclick={() => navigate("settings")} aria-label="Settings"><Settings size={27} /></button>
    </header>

    <main class="workshop-content" class:diagnostics-view={activeView !== "forge"}>
      {#if activeView === "settings"}
        <div class="workshop-page"><ForgeSettingsPage {theme} onThemeChange={chooseTheme} /></div>
      {:else if activeView === "advanced"}
        <div class="workshop-page">{@render children?.()}</div>
      {:else}
      <section class="workshop-hero">
        <h1>{primaryGpu ? "Your GPU is ready" : "Waiting for GPU"}</h1>
        <h2>{gpuName}</h2>
        <p class:pending={!safeLoopKnown} class:review={safeLoopKnown && !protectedState}><i></i> {safeLoopKnown ? (protectedState ? "Protected by Safe Loop" : "Safe Loop needs review") : "Safe Loop status unavailable"}</p>
        <div class="workshop-actions">
          <button class="workshop-forge" onclick={runForge} disabled={powerRunning || (forgePaused && !resumeAvailable)}><Anvil size={25} />{powerRunning ? "Forging…" : resumeAvailable ? "Resume Forge" : forgePaused ? "Resume unavailable" : recoveryPending ? "Review & Continue" : "Forge GPU"}</button>
          <label><select value={forgeMode} onchange={selectMode} disabled={powerRunning || forgePaused}><option value="clean">Clean Run · Ignores prior boundaries</option><option value="standard">Standard · Compact proof</option><option value="long">Long · Exhaustive proof</option></select><ChevronDown size={20} /></label>
        </div>
      </section>

      {#if hasForgeRun}
        <div class="workshop-progress-wrap">
          <ForgeProgress
            {powerSweep}
            {powerRunning}
            {safeLoop}
            {forgeMode}
            {onStopPower}
            {onStartPower}
            {onRecoverContinue}
            {onResumePower}
          />
        </div>
      {/if}

      <section class="workshop-profile" class:ready={profilesReady}>
        <div class="workshop-current"><span>Current profile</span><div><span class="workshop-profile-icon"><Hammer size={33} /></span><strong>{activeName}</strong></div><small><i></i>{activeKey ? "Applied" : "Stock"}</small></div>
        {#each profileMeta as profile}
          {@render profileDisclosure(profile, "workshop")}
        {/each}
      </section>

      {@render fullResetControl()}

      <section class="workshop-telemetry" aria-label="Live telemetry">
        {#each [
          { key: "temp", label: "Temperature", value: temperature, unit: "°C" },
          { key: "power", label: "Power", value: power, unit: "W" },
          { key: "core", label: "Clock", value: clock, unit: "MHz" },
          { key: "mem", label: "VRAM", value: memory, unit: "MHz", hint: vramCapacity(vramTotal) },
          { key: "voltage", label: "Voltage", value: voltage, unit: "mV", hint: voltage == null ? "Not exposed" : null },
          { key: "fan", label: "Fans", value: fan, unit: "%", hint: fan == null ? "Not exposed" : null },
          { key: "usage", label: "Utilization", value: usage, unit: "%" },
        ] as metric}
          <article>
            <div>{#if metric.key === "temp"}<Thermometer size={24} />{:else if metric.key === "power"}<Zap size={24} />{:else if metric.key === "core"}<Gauge size={24} />{:else if metric.key === "mem"}<Cpu size={24} />{:else if metric.key === "voltage"}<Activity size={24} />{:else if metric.key === "fan"}<Fan size={24} />{:else}<CircleGauge size={24} />{/if}<span>{metric.label}<strong>{display(metric.value)} <small>{metric.unit}</small></strong>{#if metric.hint}<em>{metric.hint}</em>{/if}</span></div>
            <TelemetrySpark values={values(metric.key)} color="#87aada" fill="rgba(135, 170, 218, 0.04)" height={43} />
          </article>
        {/each}
        <button class="workshop-advanced" onclick={() => navigate("advanced")}><ChevronRight size={23} /><span><strong>Advanced</strong><small>Logs, Sentinel, Game Trace and manual point</small></span></button>
      </section>

      <footer class="workshop-footer" class:pending={!safeLoopKnown}>
        <span><ShieldCheck size={23} /> {safeLoopKnown ? (protectedState ? "Safe Loop active · Adjustments within monitored limits" : "Safe Loop needs review") : "Safe Loop status unavailable"} <CircleGauge size={20} /></span>
        <span>{profilesReady ? "Profiles generated from measured hardware data" : "No forged profiles yet"}</span>
      </footer>
      {/if}
    </main>
  {/if}

  {#if fullResetFeedback?.message}
    <aside
      class={`reset-feedback ${fullResetFeedback.tone ?? "success"}`}
      role={fullResetFeedback.tone === "error" ? "alert" : "status"}
      aria-live={fullResetFeedback.tone === "error" ? "assertive" : "polite"}
    >
      {#if fullResetFeedback.tone === "success"}
        <ShieldCheck size={22} strokeWidth={1.8} />
      {:else}
        <TriangleAlert size={22} strokeWidth={1.8} />
      {/if}
      <div>
        <strong>{fullResetFeedback.tone === "success" ? "Reset concluído" : fullResetFeedback.tone === "warning" ? "Reset concluído com aviso" : "Falha no reset"}</strong>
        <p>{fullResetFeedback.message}</p>
      </div>
      <button type="button" onclick={onDismissFullResetFeedback} aria-label="Fechar mensagem do reset">
        <X size={18} />
      </button>
    </aside>
  {/if}

  {#if resetConfirmOpen}
    <dialog
      bind:this={resetDialog}
      class="reset-dialog"
      aria-labelledby="reset-dialog-title"
      aria-describedby="reset-dialog-description"
      oncancel={handleResetDialogCancel}
      onkeydown={handleResetDialogKeydown}
    >
      <div class="reset-dialog-heading">
        <span class="reset-dialog-icon"><TriangleAlert size={25} strokeWidth={1.7} /></span>
        <div>
          <span>CONFIRMAÇÃO OBRIGATÓRIA</span>
          <h2 id="reset-dialog-title">Reset Total</h2>
        </div>
      </div>
      <p id="reset-dialog-description">Isto apaga os perfis forjados, as observações da forja e o histórico do Sentinela. A GPU volta a stock e a próxima execução é preparada como Clean Run.</p>
      <p class="reset-dialog-note">Falhas reais já confirmadas continuam guardadas no ledger durável, mas não influenciam essa próxima busca orgânica.</p>
      <div class="reset-dialog-actions">
        <button bind:this={resetCancelButton} class="reset-cancel" type="button" onclick={closeResetConfirmation} disabled={fullResetBusy}>Cancelar</button>
        <button class="reset-confirm" type="button" onclick={confirmFullReset} disabled={fullResetBusy}>
          <Trash2 size={18} strokeWidth={1.8} />
          <span>{fullResetBusy ? "Apagando…" : "Apagar tudo e recomeçar"}</span>
        </button>
      </div>
    </dialog>
  {/if}
</section>

<style>
  :global(body) {
    overflow-x: hidden;
    background: #080b0c;
  }

  .forge-theme-screen {
    position: relative;
    min-height: 100vh;
    overflow-x: hidden;
    color: #d8dbde;
    background-color: #0a0d0e;
    background-image: var(--forge-texture);
    background-blend-mode: normal;
    background-size: 1536px 1024px;
    background-repeat: repeat;
    font-family: "Segoe UI", system-ui, sans-serif;
    -webkit-font-smoothing: antialiased;
  }

  .forge-theme-screen::before {
    content: "";
    position: absolute;
    z-index: 0;
    inset: 0;
    background: #0a0c0d;
    opacity: 0.55;
    pointer-events: none;
  }

  .forge-theme-screen > * {
    position: relative;
    z-index: 1;
  }

  .instrument::before {
    background: #141718;
    opacity: 0.6;
  }

  .workshop::before {
    background: #101314;
    opacity: 0.58;
  }

  .forge-theme-screen,
  .forge-theme-screen * {
    box-sizing: border-box;
  }

  button,
  select {
    font: inherit;
  }

  button {
    color: inherit;
  }

  button:active:not(:disabled) {
    scale: 0.96;
  }

  .full-reset-strip {
    display: flex;
    min-width: 0;
    min-height: 68px;
    align-items: center;
    justify-content: space-between;
    gap: 24px;
    border: 1px solid rgba(174, 91, 72, 0.38);
    padding: 12px 18px;
    background: rgba(40, 15, 12, 0.12);
  }

  .full-reset-copy {
    display: grid;
    min-width: 0;
    grid-template-columns: auto 1fr;
    align-items: baseline;
    gap: 3px 14px;
  }

  .full-reset-copy > span {
    grid-column: 1 / -1;
    color: #936f66;
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.12em;
  }

  .full-reset-copy > strong {
    color: #d3b3aa;
    font-size: 15px;
    font-weight: 560;
    white-space: nowrap;
  }

  .full-reset-copy > p {
    margin: 0;
    color: #878c8e;
    font-size: 12px;
    line-height: 1.45;
    text-wrap: pretty;
  }

  .full-reset-action,
  .reset-dialog-actions button {
    display: inline-flex;
    min-height: 44px;
    align-items: center;
    justify-content: center;
    gap: 9px;
    border: 1px solid rgba(193, 96, 76, 0.65);
    padding: 0 16px;
    background: transparent;
    color: #d8a398;
    cursor: pointer;
    transition:
      color 150ms ease,
      background-color 150ms ease,
      border-color 150ms ease,
      opacity 150ms ease;
  }

  .full-reset-action:hover:not(:disabled),
  .reset-confirm:hover:not(:disabled) {
    border-color: #d27361;
    background: rgba(145, 50, 35, 0.14);
    color: #f0b1a4;
  }

  .full-reset-action:focus-visible,
  .reset-dialog-actions button:focus-visible,
  .reset-feedback button:focus-visible {
    outline: 2px solid #cf8f79;
    outline-offset: 3px;
  }

  .full-reset-action:disabled,
  .reset-dialog-actions button:disabled {
    cursor: not-allowed;
    opacity: 0.48;
  }

  .instrument .full-reset-strip {
    margin-top: 18px;
    border-color: #62534b;
    background: rgba(20, 14, 12, 0.28);
    box-shadow: inset 0 0 0 3px rgba(0, 0, 0, 0.2);
  }

  .workshop .full-reset-strip {
    min-height: 74px;
    margin: 0 40px;
    border: 0;
    border-bottom: 1px solid #373a3a;
    padding-inline: 0;
    background: transparent;
  }

  .reset-dialog::backdrop {
    background: rgba(3, 5, 6, 0.82);
    backdrop-filter: blur(4px);
  }

  .reset-dialog {
    box-sizing: border-box;
    width: min(570px, 100%);
    max-height: calc(100vh - 48px);
    margin: auto;
    border: 1px solid #765249;
    padding: 26px;
    background-color: #111516;
    background-image: var(--forge-texture);
    background-size: 1024px 683px;
    color: #dedbd7;
    overflow-y: auto;
    box-shadow:
      inset 0 0 0 3px rgba(0, 0, 0, 0.32),
      0 28px 80px rgba(0, 0, 0, 0.68);
  }

  .instrument .reset-dialog {
    border-color: #81715c;
    box-shadow:
      inset 0 0 0 4px #171a19,
      inset 0 0 0 5px #75634e,
      0 28px 80px rgba(0, 0, 0, 0.68);
  }

  .workshop .reset-dialog {
    border-color: #6e4b43;
    background-color: #121516;
  }

  .reset-dialog-heading {
    display: flex;
    align-items: center;
    gap: 14px;
    padding-bottom: 18px;
    border-bottom: 1px solid rgba(204, 116, 96, 0.28);
  }

  .reset-dialog-icon {
    display: grid;
    width: 46px;
    height: 46px;
    flex: 0 0 auto;
    place-items: center;
    border: 1px solid #8f5b50;
    color: #dd8d7b;
    background: rgba(117, 41, 29, 0.16);
  }

  .reset-dialog-heading > div {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }

  .reset-dialog-heading > div > span {
    color: #9b746b;
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.12em;
  }

  .reset-dialog h2 {
    margin: 0;
    color: #eee9e5;
    font-size: 25px;
    font-weight: 560;
    letter-spacing: -0.02em;
  }

  .reset-dialog > p {
    margin: 20px 0 0;
    color: #d4ccc7;
    font-size: 15px;
    line-height: 1.6;
    text-wrap: pretty;
  }

  .reset-dialog > .reset-dialog-note {
    margin-top: 10px;
    color: #9c8e89;
    font-size: 12px;
  }

  .reset-dialog-actions {
    display: flex;
    justify-content: flex-end;
    gap: 10px;
    margin-top: 24px;
  }

  .reset-dialog-actions .reset-cancel {
    border-color: #4d5457;
    color: #c7c9c8;
  }

  .reset-dialog-actions .reset-cancel:hover:not(:disabled) {
    border-color: #727a7d;
    background: rgba(255, 255, 255, 0.04);
    color: #f0f0ee;
  }

  .reset-confirm {
    min-width: 226px;
  }

  .reset-feedback {
    position: fixed;
    z-index: 260;
    right: 24px;
    bottom: 24px;
    display: grid;
    width: min(500px, calc(100% - 48px));
    grid-template-columns: auto 1fr auto;
    align-items: start;
    gap: 12px;
    border: 1px solid #4f585b;
    border-left: 3px solid #6ca25b;
    padding: 14px 12px 14px 15px;
    background: #111617;
    color: #7fbb6d;
    box-shadow: 0 18px 52px rgba(0, 0, 0, 0.55);
  }

  .reset-feedback.warning {
    border-left-color: #c19057;
    color: #d2a267;
  }

  .reset-feedback.error {
    border-left-color: #c56857;
    color: #d98270;
  }

  .reset-feedback > div {
    min-width: 0;
  }

  .reset-feedback strong {
    color: #e1dfda;
    font-size: 13px;
    font-weight: 600;
  }

  .reset-feedback p {
    margin: 4px 0 0;
    color: #a5aaab;
    font-size: 12px;
    line-height: 1.5;
    overflow-wrap: anywhere;
  }

  .reset-feedback button {
    display: grid;
    width: 44px;
    min-height: 44px;
    place-items: center;
    border: 0;
    background: transparent;
    color: #8f9698;
    cursor: pointer;
  }

  /* Command Deck */
  .command-header {
    display: grid;
    grid-template-columns: max-content minmax(220px, 1fr) minmax(230px, 320px);
    align-items: center;
    height: 102px;
    border-bottom: 1px solid rgba(164, 171, 177, 0.35);
    background: rgba(5, 8, 9, 0.86);
  }

  .brand {
    border: 0;
    background: transparent;
    cursor: pointer;
  }

  .command-brand {
    display: flex;
    min-width: 0;
    height: 100%;
    align-items: center;
    gap: 12px;
    padding-left: 32px;
    color: #aeb0b2;
    font-size: 24px;
    font-weight: 650;
    letter-spacing: 0.17em;
    white-space: nowrap;
  }

  .command-brand img {
    width: 48px;
    height: 48px;
    object-fit: contain;
  }

  .command-nav {
    display: flex;
    min-width: 0;
    height: 100%;
    align-items: stretch;
    gap: 14px;
  }

  .command-nav button {
    position: relative;
    min-width: 94px;
    border: 0;
    padding: 0 14px;
    background: transparent;
    color: #a5a9ae;
    font-size: 18px;
    cursor: pointer;
  }

  .command-nav button.active,
  .command-nav button:hover {
    color: #e9ba79;
  }

  .command-nav button.active::after {
    content: "";
    position: absolute;
    right: 10px;
    bottom: 22px;
    left: 10px;
    height: 2px;
    background: #c5864f;
  }

  .connected {
    display: flex;
    align-items: center;
    gap: 13px;
    min-height: 54px;
    border-left: 1px solid rgba(255, 255, 255, 0.18);
    padding: 0 28px 0 18px;
  }

  .connected i {
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: #79b52f;
    box-shadow: 0 0 0 2px rgba(121, 181, 47, 0.2);
  }

  .connected span {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }

  .connected strong {
    color: #86bd37;
    font-size: 16px;
    font-weight: 500;
  }

  .connected.pending i {
    background: #687177;
    box-shadow: 0 0 0 2px rgba(104, 113, 119, 0.18);
  }

  .connected.pending strong {
    color: #8e979d;
  }

  .connected small {
    color: #a9abad;
    font-size: 14px;
  }

  .command-body {
    display: flex;
    flex-direction: column;
    gap: 20px;
    padding: 28px clamp(20px, 2.35vw, 36px) 44px;
  }

  .command-page {
    min-height: calc(100vh - 102px);
    padding: 28px 36px 44px;
  }

  .command-hero {
    display: grid;
    grid-template-columns: minmax(300px, 410px) minmax(330px, 1fr) minmax(300px, 420px);
    align-items: center;
    min-height: 276px;
  }

  .command-gpu-wrap {
    display: flex;
    height: 265px;
    align-items: center;
    overflow: hidden;
    outline: 1px solid rgba(255, 255, 255, 0.1);
    outline-offset: -1px;
  }

  .command-gpu {
    width: 100%;
    height: 100%;
    object-fit: contain;
    object-position: center;
    transform: scale(1.12, 1.18);
  }

  .command-identity {
    align-self: center;
    padding: 0 44px 0 36px;
  }

  .eyebrow,
  .state-status span {
    color: #a2a6aa;
    font-size: 16px;
  }

  .command-identity h1 {
    margin: 2px 0 24px;
    color: #f0f0ef;
    font-size: clamp(38px, 3.7vw, 56px);
    font-weight: 500;
    line-height: 1.02;
    letter-spacing: -0.025em;
    text-wrap: balance;
  }

  .state-status {
    display: grid;
    grid-template-columns: 165px 230px;
    width: 395px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.28);
    padding-bottom: 22px;
  }

  .state-status > div {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .state-status > div + div {
    border-left: 1px solid rgba(255, 255, 255, 0.3);
    padding-left: 66px;
  }

  .state-status strong {
    color: #79b72e;
    font-size: 40px;
    line-height: 1;
  }

  .state-status .protected {
    display: flex;
    align-items: center;
    gap: 12px;
    font-size: 25px;
    font-weight: 500;
  }

  .state-status .protected.pending { color: #8e979d; }

  .command-identity p {
    margin: 14px 0 0;
    color: #b8bbbe;
    font-size: 20px;
  }

  .command-cta {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 18px;
  }

  .plate-button {
    position: relative;
    width: 335px;
    height: 118px;
    overflow: visible;
    border: 0;
    background: transparent;
    cursor: pointer;
  }

  .plate-button img {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: fill;
    filter: saturate(0.85) brightness(1.1);
  }

  .plate-button span {
    position: relative;
    z-index: 1;
    color: #15110d;
    font-size: 28px;
    font-weight: 600;
  }

  .plate-button:disabled {
    opacity: 0.65;
  }

  .command-run-mode {
    display: grid;
    width: 335px;
    gap: 7px;
  }

  .command-run-mode label {
    position: relative;
    display: grid;
    min-height: 62px;
    grid-template-columns: 1fr auto;
    align-content: center;
    border: 1px solid #4a5054;
    padding: 8px 42px 8px 15px;
    background: rgba(9, 13, 15, 0.82);
  }

  .command-run-mode label > span {
    grid-column: 1 / -1;
    color: #7f8b94;
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.14em;
  }

  .command-run-mode select {
    width: 100%;
    appearance: none;
    border: 0;
    padding: 2px 0 0;
    background: transparent;
    color: #d8dbde;
    font-size: 15px;
    font-weight: 600;
    outline: none;
    cursor: pointer;
  }

  .command-run-mode label :global(svg) {
    position: absolute;
    right: 15px;
    bottom: 15px;
    color: #c58a55;
    pointer-events: none;
  }

  .command-run-mode label:focus-within {
    border-color: #b37c4d;
    box-shadow: 0 0 0 2px rgba(179, 124, 77, 0.16);
  }

  .command-run-mode select:disabled {
    cursor: not-allowed;
    opacity: 0.62;
  }

  .command-run-mode small {
    color: #78838b;
    font-size: 11px;
    line-height: 1.35;
    text-align: center;
  }

  .refine {
    display: flex;
    align-items: center;
    gap: 16px;
    border: 0;
    border-bottom: 1px solid #51575c;
    padding: 0 0 7px;
    background: transparent;
    color: #8f99a4;
    font-size: 19px;
    cursor: default;
  }

  .refine :global(svg) { color: #7eae3a; }

  .command-telemetry {
    display: grid;
    grid-template-columns: repeat(7, minmax(0, 1fr));
    gap: 1px;
    min-height: 0;
    border: 1px solid #62696e;
    background: #555c61;
  }

  .command-metric {
    position: relative;
    min-width: 0;
    min-height: 188px;
    padding: 23px clamp(18px, 1.5vw, 28px) 16px;
    background: rgba(8, 11, 12, 0.94);
  }

  .command-metric + .command-metric {
    border-left: 0;
  }

  .metric-title {
    display: flex;
    align-items: center;
    gap: 13px;
    color: #8f989f;
    font-size: 15px;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }

  .metric-reading {
    display: flex;
    min-width: 0;
    align-items: baseline;
    gap: clamp(6px, 0.65vw, 10px);
    margin: 8px 0 10px clamp(0px, 2.5vw, 36px);
  }

  .metric-reading strong {
    color: #eceeed;
    font-size: clamp(38px, 3.1vw, 45px);
    font-weight: 500;
    line-height: 1;
    font-variant-numeric: tabular-nums;
  }

  .metric-reading span {
    flex: 0 0 auto;
    color: #b4b7b9;
    font-size: 17px;
  }

  .command-metric small {
    display: block;
    margin-top: 2px;
    color: #93989d;
    text-align: right;
    min-height: 1.25rem;
    overflow-wrap: anywhere;
  }

  .section-label {
    display: flex;
    align-items: center;
    gap: 18px;
    height: 24px;
    color: #9da2a6;
    font-size: 14px;
  }

  .section-label::before,
  .section-label::after {
    content: "";
    height: 1px;
    background: #4e555a;
  }

  .section-label::before { width: 18px; }
  .section-label::after { flex: 1; }

  .field-failure {
    min-height: 40px !important;
    margin-top: 0 !important;
    border: 0 !important;
    padding: 0 6px !important;
    background: transparent !important;
    color: #9b9189 !important;
    font-size: 11px !important;
    letter-spacing: 0.03em;
    text-decoration: underline;
    text-underline-offset: 3px;
    cursor: pointer;
  }

  .field-failure:disabled {
    cursor: default;
    opacity: 0.45;
  }

  .command-advanced,
  .instrument-advanced {
    display: grid;
    width: 100%;
    min-height: 66px;
    grid-template-columns: auto 1fr auto;
    align-items: center;
    border: 1px solid #51595f;
    padding: 0 34px;
    background: rgba(6, 9, 10, 0.35);
    color: #c2c5c8;
    cursor: pointer;
    text-align: left;
  }

  .command-advanced > span {
    display: flex;
    align-items: center;
    gap: 24px;
    font-size: 18px;
  }

  .command-advanced > small {
    color: #858b91;
    font-size: 14px;
  }

  /* Instrument Panel */
  .instrument {
    font-family: Bahnschrift, "Arial Narrow", "Segoe UI", sans-serif;
  }

  .instrument-frame {
    display: grid;
    grid-template-columns: 208px 1fr;
    min-height: 100vh;
    border: 2px solid #373c3d;
  }

  .instrument-rail {
    display: flex;
    min-height: 100vh;
    flex-direction: column;
    border-right: 1px solid #5e6260;
    background: rgba(14, 18, 18, 0.66);
    box-shadow: inset -6px 0 18px rgba(0, 0, 0, 0.26);
  }

  .instrument-lockup {
    display: flex;
    height: 204px;
    align-items: center;
    justify-content: center;
    border: 0;
    border-bottom: 1px solid #5b5f5d;
    background: transparent;
    cursor: pointer;
  }

  .instrument-lockup img {
    width: 176px;
    height: 154px;
    object-fit: contain;
  }

  .instrument-rail nav {
    display: flex;
    flex-direction: column;
  }

  .instrument-rail nav button {
    display: flex;
    min-height: 88px;
    align-items: center;
    gap: 22px;
    border: 0;
    border-bottom: 1px solid rgba(255, 255, 255, 0.08);
    padding: 0 30px;
    background: transparent;
    color: #a4a8a7;
    font-size: 16px;
    letter-spacing: 0.03em;
    text-transform: uppercase;
    cursor: pointer;
  }

  .instrument-rail nav button.active {
    border-left: 5px solid #c6a268;
    padding-left: 25px;
    background: rgba(198, 162, 104, 0.08);
    color: #dbba80;
  }

  .rail-gpu {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    margin-top: auto;
    border-top: 1px solid #444947;
    padding: 19px 18px 28px;
    text-align: center;
  }

  .nvidia-mark {
    color: #8cc61f;
    font-weight: 700;
  }

  .rail-gpu strong {
    color: #d2d4d3;
    font-size: 14px;
    font-weight: 500;
  }

  .rail-gpu small { color: #929795; }

  .instrument-content {
    display: grid;
    grid-template-columns: minmax(680px, 1fr) 405px;
    grid-template-rows: 1fr auto;
    gap: 20px 0;
    padding: 32px 14px 51px 22px;
  }

  .instrument-content.diagnostics-view {
    display: block;
    min-width: 0;
    padding: 32px 24px 40px;
  }

  .instrument-page {
    width: 100%;
    min-width: 0;
  }

  .instrument-main-column {
    min-width: 0;
    padding: 19px 0 0;
  }

  .instrument-main-column :global(.forge-progress) {
    --progress-surface: rgba(9, 13, 13, 0.58);
    --progress-outline: rgba(126, 136, 143, 0.34);
    min-width: 0;
    border-radius: 8px;
  }

  .command-body :global(.forge-progress) {
    --progress-surface: rgba(7, 10, 12, 0.58);
    --progress-outline: rgba(126, 136, 143, 0.34);
    min-width: 0;
    border-radius: 11px;
  }

  .instrument-intro h1 {
    margin: 10px 0 15px;
    color: #d9dad8;
    font-family: "Bahnschrift Condensed", "Arial Narrow", Bahnschrift, sans-serif;
    font-size: 56px;
    font-stretch: condensed;
    font-weight: 700;
    letter-spacing: 0.015em;
    transform: scaleX(0.875);
    transform-origin: left center;
  }

  .instrument-intro { margin-right: 32px; padding-left: 24px; }

  .instrument-kicker,
  .panel-kicker {
    color: #d5b478;
    font-size: 14px;
    letter-spacing: 0.09em;
  }

  .instrument-kicker i {
    display: inline-block;
    width: 8px;
    height: 8px;
    margin-right: 10px;
    border-radius: 50%;
    background: #dab878;
  }

  .instrument-intro p {
    display: flex;
    align-items: center;
    gap: 17px;
    margin: 0;
    border-top: 1px solid #555a58;
    padding-top: 18px;
    color: #d2d4d2;
    font-size: 23px;
    font-weight: 600;
  }

  .instrument-intro p :global(svg) { color: #86b63b; }

  .gauge-layout {
    display: grid;
    position: relative;
    left: 16px;
    width: calc(100% - 32px);
    grid-template-columns: 1fr 319px 1fr;
    align-items: center;
    gap: 11px;
    height: 440px;
  }

  .gauge-bezel {
    position: relative;
    width: 386px;
    height: 386px;
    transform: translate(-34px, -27px);
  }

  .gauge-bezel img {
    width: 100%;
    height: 100%;
    object-fit: contain;
  }

  .gauge-value {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    flex-direction: column;
    padding-top: 80px;
  }

  .gauge-value span {
    color: #cdb58d;
    font-size: 12px;
    letter-spacing: 0.08em;
  }

  .gauge-value strong {
    color: #dfdfdc;
    font-size: 65px;
    line-height: 1;
    font-variant-numeric: tabular-nums;
  }

  .gauge-value small {
    color: #d0d0cc;
    font-size: 18px;
  }

  .gauge-side {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 0;
    transform: translateY(13px);
  }

  .gauge-side > div {
    display: flex;
    min-height: 205px;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    border-right: 1px solid #5b605d;
  }

  .gauge-side.right > div:first-child { border-left: 1px solid #5b605d; }
  .gauge-side.right > div:last-child { border-right: 0; }
  .gauge-side.left > div:nth-child(2) { transform: translateX(10px); }
  .gauge-side.right > div:first-child { transform: translateX(-5px); }
  .gauge-side.right > div:last-child { transform: translateX(-4px); }

  .gauge-side span {
    color: #9eb7cb;
    font-size: 12px;
    letter-spacing: 0.05em;
    text-transform: uppercase;
  }

  .gauge-side strong {
    margin-top: 16px;
    color: #dddeda;
    font-size: 45px;
    font-weight: 500;
    line-height: 1;
    font-variant-numeric: tabular-nums;
    transform: translateY(6px);
  }

  .gauge-side small {
    margin-top: 8px;
    color: #cacbc8;
    max-width: 16ch;
    font-size: 14px;
    line-height: 1.3;
    text-align: center;
    transform: translateY(10px);
  }

  .gauge-side :global(.spark) {
    width: 78%;
    margin-top: 23px;
    opacity: 0.72;
    transform: translateY(12px);
  }

  .recommended-panel { margin-top: 7px; }
  .recommended-panel > .instrument-kicker { display: block; padding-left: 24px; }

  .instrument-action-panel {
    display: flex;
    min-width: 0;
    flex-direction: column;
    border: 1px solid #6a6557;
    padding: 25px 27px 16px;
    background: rgba(8, 12, 12, 0.5);
    box-shadow: inset 0 0 0 4px rgba(0, 0, 0, 0.34);
  }

  .instrument-forge {
    display: flex;
    height: 120px;
    align-items: center;
    justify-content: center;
    gap: 25px;
    margin-top: 14px;
    border: 1px solid #a78853;
    background-color: #b6a06b;
    background-image: var(--forge-texture);
    background-blend-mode: soft-light;
    color: #2b271e;
    box-shadow: inset 0 0 0 4px #241f17, inset 0 0 0 6px #b79a65, 0 4px 10px rgba(0, 0, 0, 0.4);
    cursor: pointer;
  }

  .instrument-forge strong { font-size: 28px; }

  .mode-block,
  .safe-loop-block,
  .instrument-runtime {
    border-top: 1px solid #555952;
    margin-top: 24px;
    padding-top: 24px;
  }

  .mode-block { margin-top: 31px; }

  .mode-block label,
  .safe-loop-block > span {
    color: #bbb8ae;
    font-size: 13px;
    letter-spacing: 0.05em;
  }

  .mode-block select {
    width: 100%;
    height: 49px;
    margin: 12px 0 16px;
    border: 1px solid #6b6254;
    padding: 0 15px;
    background: #1b1f1e;
    color: #d2d0ca;
  }

  .mode-block p,
  .safe-loop-block p { margin: 0 0 8px; color: #c0c0bb; font-size: 14px; line-height: 1.5; }
  .mode-block small { color: #8e9390; line-height: 1.5; }

  .safe-loop-block > strong {
    display: flex;
    align-items: center;
    gap: 15px;
    margin: 17px 0 14px;
    color: #88b841;
    font-size: 25px;
  }

  .safe-loop-block > strong.pending { color: #8e9691; }

  .safe-loop-block button {
    display: flex;
    align-items: center;
    gap: 7px;
    border: 0;
    padding: 0;
    background: transparent;
    color: #94b4cf;
    cursor: pointer;
    margin-top: 12px;
  }

  .instrument-runtime {
    display: grid;
    grid-template-columns: 1fr 1fr;
    margin-top: auto;
    min-height: 98px;
    padding-top: 34px;
  }

  .instrument-runtime > div {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 0 13px;
  }

  .instrument-runtime > div + div { border-left: 1px solid #555952; }
  .instrument-runtime span { display: flex; flex-direction: column; gap: 6px; color: #8d9290; font-size: 11px; }
  .instrument-runtime strong { color: #cfd0cd; font-size: 16px; font-weight: 500; font-variant-numeric: tabular-nums; }

  .instrument-apply {
    display: flex;
    width: 145px;
    min-height: 42px;
    align-items: center;
    justify-content: center;
    gap: 25px;
    margin-top: 20px;
    border: 1px solid #816f4e;
    background: #171a19;
    color: #d0b27e;
    cursor: pointer;
  }

  .instrument-advanced {
    grid-column: 1 / -1;
    grid-template-columns: auto 1fr auto auto;
    min-height: 87px;
    gap: 22px;
    border-color: #5e5e58;
    color: #aeb2b0;
  }

  .instrument-advanced > span { display: flex; flex-direction: column; gap: 3px; }
  .instrument-advanced > span strong { font-size: 18px; font-weight: 500; }
  .instrument-advanced small { color: #8d9290; font-size: 13px; }

  /* Quiet Workshop */
  .workshop {
    color: #e8e6e2;
    font-family: "Segoe UI", system-ui, sans-serif;
  }

  .workshop-header {
    display: grid;
    grid-template-columns: 270px 1fr 64px;
    align-items: center;
    height: 70px;
    border-bottom: 1px solid #333636;
    padding: 0 32px;
    background: rgba(5, 8, 8, 0.72);
  }

  .workshop-brand {
    display: flex;
    align-items: center;
    gap: 24px;
    color: #f0efeb;
    font-size: 23px;
    font-weight: 650;
    letter-spacing: 0.08em;
  }

  .workshop-brand :global(svg) { color: #d29063; }

  .workshop-header nav {
    display: flex;
    height: 100%;
    align-items: center;
    gap: 24px;
  }

  .workshop-header nav button {
    position: relative;
    height: 100%;
    border: 0;
    background: transparent;
    color: #8e8f8f;
    font-size: 17px;
    cursor: pointer;
  }

  .workshop-header nav button.active { color: #d89a6e; }
  .workshop-header nav button.active::after { content: ""; position: absolute; right: 0; bottom: 0; left: 0; height: 1px; background: #c37d50; }
  .workshop-settings {
    display: flex;
    min-height: 45px;
    align-items: center;
    justify-content: center;
    border: 0;
    border-left: 1px solid #333636;
    background: transparent;
    color: #b8b8b5;
    cursor: pointer;
  }

  .workshop-settings.active,
  .workshop-settings:hover {
    color: #d29063;
    background: rgba(210, 144, 99, 0.06);
  }

  .workshop-content { min-height: calc(100vh - 70px); }

  .workshop-content.diagnostics-view {
    padding: 30px 35px 48px;
  }

  .workshop-page {
    width: 100%;
    min-width: 0;
  }

  .workshop-hero {
    display: flex;
    min-height: 463px;
    align-items: center;
    justify-content: center;
    flex-direction: column;
    border-bottom: 1px solid #424444;
    text-align: center;
  }

  .workshop-hero > * { transform: translateY(16px); }

  .workshop-hero h1 {
    margin: 8px 0 24px;
    color: #f0efec;
    font-size: clamp(48px, 5.2vw, 72px);
    font-weight: 500;
    line-height: 1;
    letter-spacing: -0.035em;
  }

  .workshop-hero h2 {
    margin: 0;
    color: #898989;
    font-size: 29px;
    font-weight: 400;
  }

  .workshop-hero p {
    display: flex;
    align-items: center;
    gap: 12px;
    margin: 26px 0 52px;
    color: #79bd70;
    font-size: 20px;
  }

  .workshop-hero p i,
  .workshop-current small i {
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: #7bc273;
  }

  .workshop-hero p.pending { color: #8d9492; }
  .workshop-hero p.pending i { background: #737b78; }
  .workshop-hero p.review { color: #cf946c; }
  .workshop-hero p.review i { background: #c78359; }

  .workshop-actions {
    display: flex;
    align-items: center;
    gap: 35px;
  }

  .workshop-forge {
    display: flex;
    width: 300px;
    height: 77px;
    align-items: center;
    justify-content: center;
    gap: 20px;
    border: 1px solid #d59b74;
    background-color: #bd7d55;
    background-image: var(--forge-texture);
    background-blend-mode: soft-light;
    color: #20150f;
    box-shadow: inset 0 0 0 2px rgba(255, 215, 183, 0.18), 0 8px 14px rgba(0, 0, 0, 0.35);
    font-size: 20px;
    cursor: pointer;
  }

  .workshop-actions label {
    position: relative;
    display: flex;
    width: 355px;
    height: 66px;
    align-items: center;
  }

  .workshop-actions select {
    width: 100%;
    height: 100%;
    appearance: none;
    border: 1px solid #545657;
    padding: 0 50px 0 24px;
    background: #151718;
    color: #e0dfdc;
    font-size: 18px;
  }

  .workshop-actions label :global(svg) { position: absolute; right: 20px; pointer-events: none; }

  .workshop-profile {
    display: grid;
    grid-template-columns: 300px repeat(3, minmax(0, 1fr));
    min-height: 252px;
    align-items: center;
    border-bottom: 1px solid #373a3a;
    padding: 0 40px;
  }

  .workshop-current {
    display: flex;
    height: 140px;
    flex-direction: column;
    justify-content: center;
    border-right: 1px solid #484b4b;
    padding-right: 36px;
  }

  .workshop-current > span { color: #c6ad9d; font-size: 14px; }
  .workshop-current > div { display: flex; align-items: center; gap: 20px; margin-top: 17px; }
  .workshop-current > div strong { font-size: 24px; font-weight: 500; }

  .workshop-profile-icon {
    display: flex;
    width: 72px;
    height: 72px;
    align-items: center;
    justify-content: center;
    border: 1px solid #b56f4e;
    border-radius: 50%;
    color: #c98962;
  }

  .workshop-current small { display: flex; align-items: center; gap: 8px; margin: -20px 0 0 94px; color: #70b66a; font-size: 17px; }
  .workshop-current small i { width: 17px; height: 17px; }

  .workshop-telemetry {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(170px, 1fr));
    min-height: 197px;
    align-items: center;
    padding: 0;
  }

  .workshop-telemetry article {
    min-width: 0;
    border-right: 1px solid #444747;
    min-height: 132px;
    padding: 24px 28px;
  }

  .workshop-telemetry article:first-child { padding-left: 40px; }

  .workshop-telemetry article > div { display: flex; align-items: center; gap: 18px; color: #d2b592; }
  .workshop-telemetry article > div > span { display: flex; flex-direction: column; gap: 4px; color: #bebfbd; font-size: 14px; }
  .workshop-telemetry article strong { color: #eeece8; font-size: 23px; font-weight: 500; font-variant-numeric: tabular-nums; }
  .workshop-telemetry article strong small { color: #bbb; font-size: 15px; font-weight: 400; }
  .workshop-telemetry article em {
    max-width: 18ch;
    color: #858b89;
    font-size: 11px;
    font-style: normal;
    line-height: 1.35;
    overflow-wrap: anywhere;
  }

  .workshop-progress-wrap {
    border-bottom: 1px solid #373a3a;
    padding: 24px 40px;
  }

  .workshop-progress-wrap :global(.forge-progress) {
    --progress-surface: rgba(12, 14, 15, 0.64);
    --progress-outline: #373a3a;
    border-radius: 11px;
  }

  .workshop-advanced {
    display: flex;
    align-items: flex-start;
    gap: 18px;
    min-height: 120px;
    border: 0;
    padding: 25px 0 0 38px;
    background: transparent;
    text-align: left;
    cursor: pointer;
  }

  .workshop-advanced span { display: flex; flex-direction: column; gap: 12px; }
  .workshop-advanced strong { color: #e2e1de; font-size: 18px; font-weight: 500; }
  .workshop-advanced small { color: #929594; font-size: 14px; line-height: 1.5; }

  .workshop-footer {
    display: flex;
    min-height: 76px;
    align-items: center;
    justify-content: space-between;
    border-top: 1px solid #383b3b;
    padding: 0 35px;
    color: #969b98;
    font-size: 13px;
  }

  .workshop-footer span { display: flex; align-items: center; gap: 12px; }
  .workshop-footer :global(svg) { color: #71b866; }
  .workshop-footer.pending :global(svg) { color: #777f7c; }

  @media (max-width: 1440px) {
    .command-hero { grid-template-columns: minmax(300px, 390px) minmax(360px, 1fr); }
    .command-cta { grid-column: 1 / -1; flex-direction: row; justify-content: center; }
  }

  @media (min-width: 981px) and (max-width: 1599px) {
    .command-telemetry { grid-template-columns: repeat(12, minmax(0, 1fr)); }
    .command-metric { grid-column: span 3; }
    .command-metric:nth-last-child(-n + 3) { grid-column: span 4; }
  }

  @media (max-width: 1380px) {
    .instrument-content { grid-template-columns: 1fr; }
    .instrument-action-panel { grid-row: 2; }
    .instrument-advanced { grid-row: 3; }
  }

  @media (max-width: 1180px) {
    .command-header { grid-template-columns: 280px minmax(190px, 1fr) 240px; }
    .command-nav { gap: 2px; }
    .command-nav button { min-width: auto; padding-inline: 10px; }
    .command-hero { grid-template-columns: minmax(280px, 330px) 1fr; }
    .workshop-telemetry { grid-template-columns: repeat(3, 1fr); gap: 28px 0; padding-block: 28px; }
  }

  @media (min-width: 561px) and (max-width: 980px) {
    .command-telemetry { grid-template-columns: repeat(2, minmax(0, 1fr)); }
    .command-metric,
    .command-metric:nth-last-child(-n + 3) { grid-column: auto; }
    .command-metric:last-child { grid-column: 1 / -1; }
  }

  @media (max-width: 820px) {
    .full-reset-strip {
      min-height: 0;
      align-items: stretch;
      flex-direction: column;
      gap: 14px;
      padding: 16px;
    }
    .full-reset-copy { grid-template-columns: 1fr; }
    .full-reset-copy > span,
    .full-reset-copy > strong,
    .full-reset-copy > p { grid-column: 1; }
    .full-reset-action { width: 100%; }
    .workshop .full-reset-strip { margin-inline: 24px; padding-inline: 0; }
    .reset-dialog { padding: 20px; }
    .reset-dialog-actions { flex-direction: column-reverse; }
    .reset-dialog-actions button { width: 100%; }
    .reset-confirm { min-width: 0; }
    .reset-feedback { right: 12px; bottom: 12px; width: calc(100% - 24px); }
    .command-header { grid-template-columns: 1fr auto; height: auto; min-height: 82px; }
    .command-brand { padding-left: 20px; }
    .command-nav { grid-column: 1 / -1; order: 3; overflow-x: auto; height: 58px; padding-left: 10px; }
    .command-page { min-height: calc(100vh - 140px); padding: 22px 18px 36px; }
    .connected { display: none; }
    .command-hero { grid-template-columns: 1fr; }
    .command-cta { flex-wrap: wrap; }
    .command-gpu-wrap { height: 220px; }
    .command-identity { padding: 0; }
    .command-telemetry { grid-template-columns: 1fr 1fr; }
    .instrument-frame { grid-template-columns: 1fr; }
    .instrument-rail { min-height: auto; border-right: 0; }
    .instrument-lockup { height: 120px; }
    .instrument-lockup img { height: 110px; }
    .instrument-rail nav { flex-direction: row; overflow-x: auto; }
    .instrument-rail nav button { min-width: 150px; min-height: 70px; }
    .rail-gpu { display: none; }
    .instrument-content { padding: 30px 18px; }
    .gauge-layout { grid-template-columns: 1fr; height: auto; }
    .gauge-side { order: 2; }
    .gauge-bezel { margin-inline: auto; }
    .workshop-header { grid-template-columns: 1fr auto; padding-inline: 18px; }
    .workshop-header nav { grid-column: 1 / -1; order: 3; }
    .workshop-content.diagnostics-view { padding: 22px 18px 36px; }
    .workshop-telemetry { grid-template-columns: 1fr 1fr; }
  }

  @media (max-width: 560px) {
    .command-body { padding-inline: 12px; }
    .command-brand span { font-size: 17px; }
    .command-hero { min-height: 0; }
    .command-identity h1 { font-size: 34px; }
    .state-status { width: 100%; grid-template-columns: 1fr 1fr; }
    .state-status > div + div { padding-left: 20px; }
    .state-status strong { font-size: 30px; }
    .command-cta { flex-direction: column; }
    .plate-button,
    .command-run-mode { width: min(100%, 320px); }
    .command-telemetry { grid-template-columns: 1fr; }
    .command-metric,
    .command-metric:nth-last-child(-n + 3),
    .command-metric:last-child { grid-column: auto; }
    .gauge-side { grid-template-columns: 1fr; }
    .gauge-side > div { min-height: 170px; border-right: 0; border-bottom: 1px solid #5b605d; }
    .gauge-side.right > div:first-child { border-left: 0; }
    .workshop-actions { width: 100%; flex-direction: column; gap: 14px; }
    .workshop-forge,
    .workshop-actions label { width: min(100%, 355px); }
    .workshop-telemetry { grid-template-columns: 1fr; }
    .workshop-progress-wrap { padding-inline: 16px; }
    .workshop-footer { align-items: flex-start; flex-direction: column; gap: 10px; padding-block: 18px; }
  }

  /* Shared profile disclosure: the three visual themes keep their own color language while the
     information hierarchy and responsive behavior stay identical. */
  .forge-profile-card {
    --profile-accent: #c78a54;
    --profile-surface: rgba(7, 10, 12, 0.58);
    min-width: 0;
    overflow: hidden;
    border-radius: 11px;
    background: var(--profile-surface);
    box-shadow: inset 0 0 0 1px rgba(126, 136, 143, 0.34);
  }

  .forge-profile-card + .forge-profile-card {
    margin-top: 8px;
  }

  .forge-profile-card.active {
    background: color-mix(in srgb, var(--profile-accent) 8%, var(--profile-surface));
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--profile-accent) 72%, transparent);
  }

  .forge-profile-card.instrument {
    --profile-accent: #b9905a;
    --profile-surface: rgba(9, 13, 13, 0.58);
    border-radius: 8px;
  }

  .forge-profile-card.workshop {
    --profile-accent: #c98660;
    --profile-surface: rgba(12, 14, 15, 0.64);
  }

  .profile-disclosure {
    display: grid;
    width: 100%;
    min-height: 88px;
    grid-template-columns: 52px minmax(0, 1fr) minmax(120px, auto);
    align-items: center;
    gap: 15px;
    border: 0;
    border-radius: 11px;
    padding: 14px 16px;
    background: transparent;
    color: inherit;
    font: inherit;
    text-align: left;
    cursor: pointer;
    transition: background-color 150ms ease, transform 100ms ease;
  }

  .profile-disclosure:hover:not(:disabled) {
    background: rgba(255, 255, 255, 0.028);
  }

  .profile-disclosure:active:not(:disabled) {
    transform: scale(0.992);
  }

  .profile-disclosure:disabled {
    cursor: default;
    opacity: 1;
  }

  .profile-card-icon {
    display: grid;
    width: 48px;
    height: 48px;
    place-items: center;
    border-radius: 9px;
    background: color-mix(in srgb, var(--profile-accent) 9%, transparent);
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--profile-accent) 30%, transparent);
    color: #8e989f;
  }

  .forge-profile-card.active .profile-card-icon {
    color: var(--profile-accent);
  }

  .profile-card-copy {
    display: flex;
    min-width: 0;
    flex-direction: column;
    gap: 5px;
  }

  .profile-card-copy strong {
    color: #e3e3df;
    font-size: 18px;
    font-weight: 580;
    letter-spacing: -0.01em;
  }

  .profile-card-copy small {
    max-width: 74ch;
    color: #92999d;
    font-size: 12px;
    line-height: 1.5;
    text-wrap: pretty;
  }

  .profile-card-state {
    display: flex;
    min-width: 0;
    align-items: center;
    justify-content: flex-end;
    gap: 10px;
    color: var(--profile-accent);
  }

  .profile-card-state small {
    overflow: hidden;
    font-size: 11px;
    font-weight: 680;
    letter-spacing: 0.04em;
    text-overflow: ellipsis;
    text-transform: uppercase;
    white-space: nowrap;
  }

  .profile-card-state :global(svg) {
    flex: 0 0 auto;
    transition: transform 160ms ease;
  }

  .forge-profile-card.expanded .profile-card-state :global(svg) {
    transform: rotate(180deg);
  }

  .profile-card-details {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    align-items: end;
    gap: 16px;
    margin: 0 14px 14px;
    border-radius: 9px;
    padding: 14px;
    background: rgba(0, 0, 0, 0.2);
    box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.055);
  }

  .profile-card-metrics {
    display: grid;
    grid-template-columns: repeat(5, minmax(105px, 1fr));
    gap: 8px;
  }

  .profile-card-metrics > span {
    display: flex;
    min-width: 0;
    min-height: 62px;
    flex-direction: column;
    justify-content: center;
    gap: 5px;
    border-radius: 8px;
    padding: 9px 10px;
    background: rgba(255, 255, 255, 0.025);
  }

  .profile-card-metrics small {
    color: #737d82;
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }

  .profile-card-metrics strong {
    overflow: hidden;
    color: #d7d9d7;
    font-size: 12px;
    font-weight: 580;
    font-variant-numeric: tabular-nums;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .profile-card-actions {
    display: flex;
    align-items: stretch;
    gap: 8px;
  }

  .profile-card-actions button {
    display: inline-flex;
    min-height: 44px !important;
    align-items: center;
    justify-content: center;
    gap: 7px;
    border: 0 !important;
    border-radius: 8px;
    margin: 0 !important;
    padding: 0 14px;
    background: transparent;
    color: #aeb4b4;
    font: inherit;
    font-size: 11px;
    font-weight: 670;
    cursor: pointer;
    transition: background-color 150ms ease, box-shadow 150ms ease, color 150ms ease, transform 100ms ease;
  }

  .profile-card-actions .profile-apply {
    background: color-mix(in srgb, var(--profile-accent) 12%, transparent);
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--profile-accent) 46%, transparent);
    color: color-mix(in srgb, var(--profile-accent) 78%, white);
  }

  .profile-card-actions .field-failure {
    box-shadow: inset 0 0 0 1px rgba(133, 141, 143, 0.32);
  }

  .profile-card-actions button:hover:not(:disabled) {
    background-color: color-mix(in srgb, var(--profile-accent) 18%, transparent);
    color: #f0eee9;
  }

  .profile-card-actions button:active:not(:disabled) {
    transform: scale(0.96);
  }

  .profile-card-actions button:disabled {
    cursor: default;
    opacity: 0.52;
  }

  .instrument-profile-grid {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: 8px;
    margin-top: 8px;
    border: 0;
    background: transparent;
    box-shadow: none;
  }

  .instrument-profile-grid .forge-profile-card {
    display: block;
    min-height: 0;
    padding: 0;
  }

  .instrument-profile-grid .forge-profile-card + .forge-profile-card {
    border-top: 0;
    border-left: 0;
  }

  .workshop-profile {
    grid-template-columns: 280px minmax(0, 1fr);
    align-items: start;
    gap: 8px 22px;
    padding-block: 30px;
  }

  .workshop-current {
    grid-column: 1;
    grid-row: 1 / span 3;
  }

  .workshop-profile .forge-profile-card {
    grid-column: 2;
  }

  @media (max-width: 1080px) {
    .profile-card-details {
      grid-template-columns: 1fr;
    }
    .profile-card-metrics {
      grid-template-columns: repeat(3, minmax(105px, 1fr));
    }
    .profile-card-actions {
      justify-content: flex-end;
    }
  }

  @media (max-width: 820px) {
    .workshop-profile {
      grid-template-columns: 1fr;
      padding-inline: 24px;
    }
    .workshop-current {
      grid-column: 1;
      grid-row: auto;
      border-right: 0;
    }
    .workshop-profile .forge-profile-card {
      grid-column: 1;
    }
    .profile-disclosure {
      grid-template-columns: 48px minmax(0, 1fr) auto;
      padding-inline: 12px;
    }
    .profile-card-state small {
      display: none;
    }
  }

  @media (max-width: 560px) {
    .workshop-profile {
      padding-inline: 16px;
    }
    .profile-disclosure {
      grid-template-columns: 44px minmax(0, 1fr) auto;
      gap: 11px;
    }
    .profile-card-icon {
      width: 44px;
      height: 44px;
    }
    .profile-card-copy strong {
      font-size: 16px;
    }
    .profile-card-metrics {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }
    .profile-card-actions {
      flex-direction: column;
    }
    .profile-card-actions button {
      width: 100%;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .full-reset-action,
    .reset-dialog-actions button,
    .profile-disclosure,
    .profile-card-state :global(svg),
    .profile-card-actions button {
      transition: none;
    }
  }
</style>
