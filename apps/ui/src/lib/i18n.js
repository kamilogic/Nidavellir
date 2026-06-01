import { writable, derived } from "svelte/store";

/** Available UI languages. English is the default; more can be added later. */
export const locales = [
  { id: "en", label: "English" },
  { id: "pt", label: "Português (BR)" },
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
    "nav.capabilities": "Capabilities",
    "nav.forge": "Forge",
    "nav.sensors": "Sensors",
    "nav.safety": "Safety",
    "common.waiting": "Waiting for the service…",

    "forge.title": "GPU Forge",
    "forge.lead":
      "Real GPU tuning on your hardware: read the V/F curve, validate stability (incl. VRAM), find the stable undervolt/OC and the memory bandwidth peak. Detects silent errors, applies a margin, and confirms with a long soak — every step through the Safe Loop.",
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
    "forge.plateau": "Plateau (locked clock): {f} MHz @ {v} mV",
    "forge.curvePoints": "{name} · {n} points on the curve",
    "forge.running": "Running the battery on the GPU… detects silent errors without needing a game.",
    "forge.result": "Result: {r}",
    "forge.stageN": "stage {n}",

    "forge.realSweep": "Real sweep — finds your card's stable undervolt (writes voltage)",
    "forge.runReal": "Run real sweep",
    "forge.stopReal": "Stop sweep",
    "forge.realResult": "Min stable voltage per top frequency",
    "forge.preTitle": "Before the real sweep",
    "forge.preBody":
      "This writes real voltage/clock to your GPU and pushes toward the stability limit. Brief screen flickers are possible (the driver recovers). It's reversible and protected by the Safe Loop. For best results, close all other programs (games, browsers, overlays) first.",
    "forge.preCancel": "Cancel",
    "forge.preFast": "Quick (Bronze)",
    "forge.preThorough": "Thorough (Gold)",
    "forge.voltageIdx": "Voltage",
    "forge.measured": "Measured: {f} MHz @ {v} mV",
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
    "forge.forgeAll": "Forge everything (auto)",
    "forge.forgeAllDesc": "Full pipeline, in order: VRAM check → core undervolt → memory bandwidth → final whole-package soak. Applies the validated profile.",
    "forge.runForge": "Forge everything",
    "forge.stopForge": "Stop",
    "forge.forgePreBody": "Runs the whole pipeline (several minutes). Writes real core voltage and memory clock and pushes to the limit under combined load — flicker/TDR possible (recovered; Safe Loop protected). Close all other programs first.",
    "forge.orderHint": "Or run manually, in this order: 1) Validate stability (VRAM gate) · 2) Real sweep (core) · 3) Memory sweep.",

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
    "nav.capabilities": "Capacidades",
    "nav.forge": "Forja",
    "nav.sensors": "Sensores",
    "nav.safety": "Segurança",
    "common.waiting": "Aguardando o serviço…",

    "forge.title": "Forja de GPU",
    "forge.lead":
      "Tuning real de GPU no seu hardware: lê a curva V/F, valida estabilidade (incl. VRAM), acha o undervolt/OC estável e o pico de banda da memória. Detecta erro silencioso, aplica margem e confirma com um soak longo — cada passo pelo Safe Loop.",
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
    "forge.plateau": "Plateau (clock travado): {f} MHz @ {v} mV",
    "forge.curvePoints": "{name} · {n} pontos na curva",
    "forge.running": "Rodando a bateria na GPU… detecta erro silencioso sem precisar de jogo.",
    "forge.result": "Resultado: {r}",
    "forge.stageN": "estágio {n}",

    "forge.realSweep": "Sweep real — acha o undervolt estável da sua placa (escreve voltagem)",
    "forge.runReal": "Rodar sweep real",
    "forge.stopReal": "Parar sweep",
    "forge.realResult": "Voltagem mínima estável por frequência de topo",
    "forge.preTitle": "Antes do sweep real",
    "forge.preBody":
      "Isto escreve voltagem/clock reais na sua GPU e empurra em direção ao limite de estabilidade. Pequenos flashes de tela são possíveis (o driver se recupera). É reversível e protegido pelo Safe Loop. Para melhores resultados, feche todos os outros programas (jogos, navegadores, overlays) antes.",
    "forge.preCancel": "Cancelar",
    "forge.preFast": "Rápido (Bronze)",
    "forge.preThorough": "Longo (Gold)",
    "forge.voltageIdx": "Tensão",
    "forge.measured": "Medido: {f} MHz @ {v} mV",
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
    "forge.forgeAll": "Forjar tudo (automático)",
    "forge.forgeAllDesc": "Pipeline completo, em ordem: checa VRAM → undervolt de core → banda de memória → soak final do pacote. Aplica o perfil validado.",
    "forge.runForge": "Forjar tudo",
    "forge.stopForge": "Parar",
    "forge.forgePreBody": "Roda o pipeline inteiro (alguns minutos). Escreve voltagem de core e clock de memória reais e empurra ao limite sob carga combinada — flicker/TDR possíveis (recuperados; protegido pelo Safe Loop). Feche todos os outros programas antes.",
    "forge.orderHint": "Ou rode manualmente, nesta ordem: 1) Validar estabilidade (gate de VRAM) · 2) Real sweep (core) · 3) Memory sweep.",

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
