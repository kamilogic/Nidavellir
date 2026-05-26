# Nidavellir ⚒️

> *O reino dos anões ferreiros na mitologia nórdica. Foi lá que forjaram Mjolnir, Gungnir e Draupnir — armas e artefatos lendários que superavam os limites do material bruto.*

**Nidavellir** é uma ferramenta open-source que analisa seu hardware, submete-o a testes de estresse, aprende seus limites individuais (silicon lottery) e gera 3 perfis de otimização aplicados em nível de hardware via UEFI + Windows.

Nenhum slider manual. Nenhum chute. Apenas o que seu silício pode entregar.

---

## Os 3 Perfis

Após a fase de aprendizado (30min–4h), o modelo da curva de silício do seu hardware gera automaticamente:

| Perfil | Objetivo | CPU | GPU | RAM |
|---|---|---|---|---|
| **Mjolnir** ⚡ | Máxima performance sustentada | Turbo máx, PL alto, C-states OFF | Clock máx estável, PL alto | Timing mais agressivo validado |
| **Draupnir** ♻️ | Máxima eficiência (perf/watt) | Undervolt ótimo, PL no joelho da curva | V/F sweet spot eficiente | Timings equilibrados |
| **Gungnir** 🍃 | Economia sem perda perceptível (≥95% stock) | Undervolt + underclock leve, PL baixo | Power limit reduzido | JEDEC ou XMP conservador |

Os nomes dos perfis podem ser alterados. O ponto central: são **derivados do aprendizado**, não presets genéricos.

---

## Arquitetura

### 2 Fases Automatizadas

```
FASE 1: WINDOWS
  CPU: MSR sweep (FIVR offset, PL1/PL2, turbo ratios, C-states)
  GPU: NVAPI/ADLX (curva V/F, power limit)
  RAM: diagnostics + SPD read (via SMBus)
  ReBAR: detecção via PCIe + alerta se OFF
  Monitor: WHEA, watchdog, temp, power
  Saída: silicon_profile.json

  ──→ REBOOT AUTOMÁTICO ──→

FASE 2: UEFI
  Carrega perfil do ESP
  RAM: tuning de timings + frequência (memory controller)
  CPU: validação em ambiente isolado
  Refina perfil → marca confiança (Bronze/Silver/Gold)
  Saída: silicon_profile_refined.json

  ──→ REBOOT → WINDOWS → PERFIS PRONTOS
```

### 3 Layers de Acesso a Hardware

```
Layer 1 (Universal) — Cobre ~97% dos ganhos
  MSR, PCIe, SMBus, WMI, NVAPI, ADLX
  ✅ Tudo que importa para os 3 perfis
  ✅ Implementado 100% no Windows (sem reboot)

Layer 2 (UEFI NVRAM DB) — Settings de BIOS
  Resizable BAR, XMP, C-state enables
  Database comunitária por placa-mãe + versão de BIOS
  Futuro: parser IFR automático para mapeamento

Layer 3 (VRM/EC) — Aprofundamento
  LLC, DIGI VRM, fan curves
  Só implementado se houver contribuição na DB
  ❌ Não necessário para nenhum perfil
```

### Crash Handling (Loop Seguro)

```
WHEA monitor → detecta erro corrigível → reverte ANTES do crash
Boot flag    → detecta crash pós-reboot no próximo início
Bugcheck     → analisado → parâmetro marcado inválido no modelo
Próxima iteração evita a região de instabilidade
```

---

## Stack Tecnológica

| Camada | Tecnologia | Motivo |
|---|---|---|
| Desktop framework | Tauri v2 | ~5MB, seguro, IPC nativo Rust ↔ UI |
| Backend | Rust | Memória safety, performance, acesso a MSR/IO |
| Frontend | Svelte 5 | Reativo, compilado, baixo boilerplate |
| Kernel driver | WinRing0 / PawnIO | MSR + PCI config + SMBus |
| GPU API | NVAPI + ADLX bindings Rust | Curva V/F, power limit |
| Otimização | argmin crate (Rust) | Bayesian optimization + pattern search |
| UEFI module | EDK2 / Rust UEFI | Memory controller, boot driver |

---

## Roadmap

| Release | Módulos | Entrega |
|---|---|---|
| v0.1 | HW Detector + Monitor | Detecta e exibe sensores |
| v0.2 | Manual Tuning | Sliders CPU/GPU via MSR+NVAPI |
| v0.3 | Stress Test Engine | CPU FFT, GPU compute, RAM patterns |
| v0.4 | Auto Sweep (Layer 1) | Varredura automatizada CPU+GPU |
| v0.5 | 3 Perfis Gerados | Curva modelada → perfis funcionais |
| v0.6 | ReBAR checker | Notificação se OFF |
| v0.7 | UEFI Boot Driver | RAM timing tuning + validação |
| v0.8 | Full Auto Pipeline | Fase1→reboot→Fase2→perfis prontos |
| v0.9 | Background Learning | Coleta dados durante uso normal |
| v1.0 | Community Database | Bootstrap + envio anônimo |

---

## Estrutura do Repositório

```
nidavellir/
├── src-tauri/              # Rust backend
│   ├── src/
│   │   ├── detector/       # HW detection (CPUID, WMI, SPD, SMBus)
│   │   ├── tuner/          # MSR, NVAPI, ADLX control
│   │   ├── stress/         # CPU/GPU/RAM stress tests
│   │   ├── optimizer/      # Bayesian optimization engine
│   │   ├── profile/        # Profile save/load/apply
│   │   ├── monitor/        # WHEA, watchdog, sensors
│   │   └── service/        # Windows service (auto-apply on boot)
│   ├── driver/             # WinRing0/PawnIO integration
│   └── Cargo.toml
├── src/                    # Svelte 5 frontend (Tauri webview)
│   ├── lib/
│   │   ├── components/     # UI components
│   │   ├── stores/         # Svelte stores (reactive state)
│   │   └── views/          # Page views (Dashboard, Tuner, Benchmark)
│   └── App.svelte
├── uefi/                   # EDK2 UEFI boot driver
│   ├── src/
│   ├── nidavellir.dsc      # EDK2 package declaration
│   └── nidavellir.inf      # EDK2 module definition
└── README.md
```

---

## Licença

**GPLv3** — aberto para forks, contribuições e auditoria.
