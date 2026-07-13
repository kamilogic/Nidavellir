import { writable, derived } from "svelte/store";

/** Available UI languages. English is the default; more can be added later. */
export const locales = [
  { id: "en", label: "English" },
];

function initialLocale() {
  try {
    const saved = localStorage.getItem("nidavellir-locale");
    if (saved && locales.some((l) => l.id === saved)) return saved;
  } catch {}
  return "en";
}

export const locale = writable(initialLocale());
locale.subscribe((v) => {
  try {
    localStorage.setItem("nidavellir-locale", v);
  } catch {}
});

const dict = {
  en: {
    "app.tagline": "Where silicon is forged to its prime.",
    "nav.forge": "Forge",
    "nav.sensors": "Sentinel",
    "nav.safety": "Safety",
    "common.waiting": "Waiting for the service…",

    "forge.title": "GPU Forge",
    "forge.lead":
      "Real GPU tuning on your hardware: read the core V/F curve, validate stability, synthesize transparent profiles, and keep risky steps behind Safe Loop protection.",
    "forge.simulated": "simulated",
    "forge.start": "Start sweep",
    "forge.stop": "Stop",
    "forge.simNote":
      "Simulated backend: the engine runs end to end without writing to the GPU. Real V/F curve writes (NVAPI) are a future increment — no offset is applied to hardware now.",
    "forge.phase": "Phase",
    "forge.frequency": "Frequency",
    "forge.testingNow": "Testing now",
    "forge.tradeoffs": "Min voltage per frequency",
    "forge.profiles": "Synthesized profiles",

    "forge.realTitle": "Real comparison (NVAPI) — your actual GPU",
    "forge.advanced": "Advanced",
    "forge.readCurve": "Read real curve",
    "forge.validate": "Validate stability (real)",
    "forge.validating": "Validating…",
    "forge.expand": "Expand",
    "forge.close": "Close",
    "forge.plateau": "Frequency plateau: {f} MHz target · Curve anchor: {v} mV",
    "forge.curveAnchorNote": "Not a hard voltage cap. Measured voltage can vary by workload.",
    "forge.vfElastic": "Elastic V/F ceiling supported — Nidavellir shapes the boost curve by flattening frequency above the curve anchor, keeping the GPU's power-management elasticity.",
    "forge.vfFallback": "Elastic V/F ceiling unavailable on this GPU/driver — undervolt falls back to a global offset + clock cap.",
    "forge.curvePoints": "{name} · {n} points on the curve",
    "forge.running": "Running the battery on the GPU… detects silent errors without needing a game.",
    "forge.result": "Result: {r}",
    "forge.stageN": "stage {n}",

    "forge.realSweep": "Legacy real sweep — developer-only path hidden from the product flow",
    "forge.runReal": "Run legacy sweep",
    "forge.stopReal": "Stop sweep",
    "forge.realResult": "Min stable voltage per top frequency",
    "forge.preTitle": "Before the real sweep",
    "forge.preBody":
      "This legacy diagnostic is hidden from the normal product flow. The current Forge GPU path uses the core VF forge and Safe Loop protection.",
    "forge.preCancel": "Cancel",
    "forge.preFast": "Quick (Bronze)",
    "forge.preThorough": "Thorough (Gold)",
    "forge.voltageIdx": "Voltage",
    "forge.measured": "Measured clock: {f} MHz · measured voltage: {v} mV",
    "forge.tempC": "Temp: {t} °C",
    "forge.memSweep": "Memory sweep — finds the GDDR6 effective-bandwidth peak (writes mem clock)",
    "forge.runMem": "Run memory sweep",
    "forge.stopMem": "Stop",
    "forge.baseline": "Baseline",
    "forge.bandwidth": "Bandwidth",
    "forge.memOffset": "Mem offset",
    "forge.peakResult": "Bandwidth peak: +{o} MHz → {g} GB/s",
    "forge.memPreBody":
      "This writes the real memory clock and pushes toward the limit (artifacts/flicker possible; reversible, Safe Loop protected). It stops at the bandwidth peak — past it, GDDR6 error-correction reduces real throughput, so more MHz is pointless. Close other programs first.",
    "forge.apply": "Apply",
    "forge.applyMem": "Apply mem peak",
    "forge.reset": "Reset to stock",
    "forge.appliedNow": "Applied: {label}",
    "forge.appliedNone": "Nothing applied (stock)",
    "forge.applyHint": "Applied profiles are re-applied automatically on every boot (Safe Loop protected).",
    "forge.forgeAll": "Legacy full pipeline (hidden)",
    "forge.forgeAllDesc": "Legacy full-system pipeline kept out of the current GPU-first product flow.",
    "forge.runForge": "Run legacy pipeline",
    "forge.stopForge": "Stop",
    "forge.forgePreBody": "The legacy full pipeline is not part of the current GPU-first product flow.",
    "forge.orderHint": "Use Forge GPU as the primary path. Diagnostics remain available under Advanced Diagnostics.",
    "forge.benchTitle": "Benchmark — before / after",
    "forge.benchDesc": "Runs a fixed battery at stock and again with the applied profile, then reports the real gains (FPS, clock, power, perf/watt, bandwidth). Apply a profile first.",
    "forge.benchRun": "Run benchmark",
    "forge.benchStop": "Stop",
    "forge.benchMetric": "Metric",
    "forge.benchFps": "Render FPS",
    "forge.benchClock": "Avg clock",
    "forge.benchPower": "Avg power",
    "forge.benchPerfWatt": "Perf/watt (fps/W)",
    "forge.benchBandwidth": "Bandwidth (GB/s)",
    "forge.benchTemp": "Max temp",
    "forge.benchPowerCap": "Power-capped",
    "forge.benchLimit": "Power limit: {w} W (enforced cap the card throttles against).",
    "forge.powerTitle": "Power sweep — best perf under the power cap",
    "forge.powerDesc": "Holds the stock clock and lowers the voltage step by step under a max-power load, finding the lowest stable voltage and the real power it draws. Same performance, less power — the real per-chip undervolt, no formula.",
    "forge.powerRun": "Run power sweep",
    "forge.powerCap": "Detected power cap: {w} W.",
    "forge.powerApply": "Apply recommended (knee)",
    "forge.powerStock": "Stock baseline (under load): {c} MHz.",
    "forge.prof_godforge": "Godforge",
    "forge.prof_brokkrs": "Brokkr's Best",
    "forge.prof_deep_calm": "Deep Calm",

    "phase.idle": "Idle",
    "phase.baseline": "Baseline (thermal equilibrium)",
    "phase.vram_diagnostic": "VRAM diagnostic",
    "phase.voltage_bisection": "Voltage bisection",
    "phase.synthesis": "Profile synthesis",
    "phase.done": "Done",
    "phase.aborted": "Aborted",

    "val.stable": "Stable (0 errors)",
    "val.silent_error": "SILENT ERROR detected",
    "val.crash": "Crash / device lost",
    "stage.stable": "✓ stable",
    "stage.silent_error": "✗ silent error",
    "stage.crash": "✗ crash",
  },
  pt: {
    "app.tagline": "Onde o silício é forjado ao seu auge.",
    "nav.forge": "Forja",
    "nav.sensors": "Sensores",
    "nav.safety": "Segurança",
    "common.waiting": "Aguardando o serviço…",

    "forge.title": "Forja de GPU",
    "forge.lead":
      "Tuning real de GPU no seu hardware: le a curva V/F de core, valida estabilidade, sintetiza perfis transparentes e mantem passos de risco atras da protecao do Safe Loop.",
    "forge.simulated": "simulado",
    "forge.start": "Iniciar sweep",
    "forge.stop": "Parar",
    "forge.simNote":
      "Backend simulado: a engine roda de ponta a ponta sem escrever na GPU. A escrita real de curva V/F (NVAPI) entra num incremento futuro — nenhum offset é aplicado ao hardware agora.",
    "forge.phase": "Fase",
    "forge.frequency": "Frequência",
    "forge.testingNow": "Testando agora",
    "forge.tradeoffs": "Voltagem mínima por frequência",
    "forge.profiles": "Perfis sintetizados",

    "forge.realTitle": "Comparação real (NVAPI) — sua GPU de verdade",
    "forge.advanced": "Avançado",
    "forge.readCurve": "Ler curva real",
    "forge.validate": "Validar estabilidade (real)",
    "forge.validating": "Validando…",
    "forge.expand": "Destacar",
    "forge.close": "Fechar",
    "forge.plateau": "Plateau de frequencia: {f} MHz alvo · Ancora da curva: {v} mV",
    "forge.curveAnchorNote": "Nao e um limite rigido de tensao. A tensao medida pode variar por carga.",
    "forge.vfElastic": "Teto V/F elastico suportado — Nidavellir molda a curva de boost achatando a frequencia acima da ancora da curva, mantendo a elasticidade de gestao de energia da GPU.",
    "forge.vfFallback": "Teto V/F elástico indisponível nesta GPU/driver — o undervolt recai em offset global + cap de clock.",
    "forge.curvePoints": "{name} · {n} pontos na curva",
    "forge.running": "Rodando a bateria na GPU… detecta erro silencioso sem precisar de jogo.",
    "forge.result": "Resultado: {r}",
    "forge.stageN": "estágio {n}",

    "forge.realSweep": "Sweep legado — caminho de desenvolvedor fora do fluxo principal",
    "forge.runReal": "Rodar sweep legado",
    "forge.stopReal": "Parar sweep",
    "forge.realResult": "Voltagem mínima estável por frequência de topo",
    "forge.preTitle": "Antes do sweep real",
    "forge.preBody":
      "Este diagnóstico legado fica fora do fluxo normal do produto. O caminho atual Forge GPU usa a forja VF de core com proteção do Safe Loop.",
    "forge.preCancel": "Cancelar",
    "forge.preFast": "Rápido (Bronze)",
    "forge.preThorough": "Longo (Gold)",
    "forge.voltageIdx": "Tensão",
    "forge.measured": "Clock medido: {f} MHz · tensao medida: {v} mV",
    "forge.tempC": "Temp: {t} °C",
    "forge.memSweep": "Sweep de memória — acha o pico de banda efetiva da GDDR6 (escreve clock de mem)",
    "forge.runMem": "Rodar sweep de memória",
    "forge.stopMem": "Parar",
    "forge.baseline": "Base",
    "forge.bandwidth": "Banda",
    "forge.memOffset": "Offset de mem",
    "forge.peakResult": "Pico de banda: +{o} MHz → {g} GB/s",
    "forge.memPreBody":
      "Isto escreve o clock real da memória e empurra até o limite (artefatos/flicker possíveis; reversível, protegido pelo Safe Loop). Para no pico de banda — depois dele, a correção de erro da GDDR6 reduz o throughput real, então mais MHz é inútil. Feche os outros programas antes.",
    "forge.apply": "Aplicar",
    "forge.applyMem": "Aplicar pico de mem",
    "forge.reset": "Voltar ao stock",
    "forge.appliedNow": "Aplicado: {label}",
    "forge.appliedNone": "Nada aplicado (stock)",
    "forge.applyHint": "Perfis aplicados são reaplicados automaticamente a cada boot (protegido pelo Safe Loop).",
    "forge.forgeAll": "Pipeline legado completo (oculto)",
    "forge.forgeAllDesc": "Pipeline legado de sistema completo mantido fora do fluxo GPU-first atual.",
    "forge.runForge": "Rodar pipeline legado",
    "forge.stopForge": "Parar",
    "forge.forgePreBody": "O pipeline completo legado nao faz parte do fluxo GPU-first atual.",
    "forge.orderHint": "Use Forge GPU como caminho principal. Diagnosticos continuam disponiveis em Advanced Diagnostics.",
    "forge.benchTitle": "Benchmark — antes / depois",
    "forge.benchDesc": "Roda uma bateria fixa em stock e de novo com o perfil aplicado, e reporta os ganhos reais (FPS, clock, potência, perf/watt, banda). Aplique um perfil primeiro.",
    "forge.benchRun": "Rodar benchmark",
    "forge.benchStop": "Parar",
    "forge.benchMetric": "Métrica",
    "forge.benchFps": "FPS de render",
    "forge.benchClock": "Clock médio",
    "forge.benchPower": "Potência média",
    "forge.benchPerfWatt": "Perf/watt (fps/W)",
    "forge.benchBandwidth": "Banda (GB/s)",
    "forge.benchTemp": "Temp máx",
    "forge.benchPowerCap": "Em power-cap",
    "forge.benchLimit": "Limite de potência: {w} W (teto que faz a placa throttlar).",
    "forge.powerTitle": "Power sweep — melhor desempenho sob o teto de potência",
    "forge.powerDesc": "Mantém o clock do stock e baixa a tensão passo a passo sob carga máxima, achando a mínima tensão estável e a potência real que ela puxa. Mesma performance, menos watts — o undervolt real do teu chip, sem fórmula.",
    "forge.powerRun": "Rodar power sweep",
    "forge.powerCap": "Teto de potência detectado: {w} W.",
    "forge.powerApply": "Aplicar recomendado (joelho)",
    "forge.powerStock": "Baseline stock (sob carga): {c} MHz.",
    "forge.prof_godforge": "Godforge",
    "forge.prof_brokkrs": "Brokkr's Best",
    "forge.prof_deep_calm": "Deep Calm",

    "phase.idle": "Ocioso",
    "phase.baseline": "Baseline (equilíbrio térmico)",
    "phase.vram_diagnostic": "Diagnóstico de VRAM",
    "phase.voltage_bisection": "Bisseção de voltagem",
    "phase.synthesis": "Síntese dos perfis",
    "phase.done": "Concluído",
    "phase.aborted": "Abortado",

    "val.stable": "Estável (0 erros)",
    "val.silent_error": "ERRO SILENCIOSO detectado",
    "val.crash": "Crash / driver perdido",
    "stage.stable": "✓ estável",
    "stage.silent_error": "✗ erro silencioso",
    "stage.crash": "✗ crash",
  },
};

/** Reactive translator: `$t('key', { var: value })`. Falls back to en, then key. */
export const t = derived(locale, ($l) => (key, vars) => {
  let s = (dict[$l] && dict[$l][key]) ?? dict.en[key] ?? key;
  if (vars) {
    for (const k in vars) s = s.replaceAll(`{${k}}`, vars[k]);
  }
  return s;
});
