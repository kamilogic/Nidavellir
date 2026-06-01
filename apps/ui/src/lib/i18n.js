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

    "forge.title": "Forge — GPU sweep",
    "forge.lead":
      "Maps the minimum stable voltage per frequency by bisecting the stability frontier, detects silent compute errors (not just crashes), and synthesizes the three profiles. Every step goes through the Safe Loop.",
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

    "forge.title": "Forja — sweep de GPU",
    "forge.lead":
      "Mapeia a voltagem mínima estável por frequência via bisseção da fronteira, detecta erros computacionais silenciosos (não só crashes) e sintetiza os três perfis. Cada passo passa pelo Safe Loop.",
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
