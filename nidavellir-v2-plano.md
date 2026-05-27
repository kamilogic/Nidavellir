# Nidavellir v2 — Planejamento de Arquitetura, Interface e Roadmap

> Redesign completo de um auto-tuner de hardware (CPU / GPU / RAM) com foco em
> segurança, automação inteligente e honestidade técnica sobre o que é
> realmente possível em Windows vs. UEFI.

---

## 0. Estado do projeto

**Snapshot:** 2026-05-26 · commit de referência v1: `aae94bc`

| Módulo v1 | Status v1 | Status v2 (alvo) |
|---|---|---|
| UI (Tauri + Svelte 5) | Implementado (monolito) | Greenfield — UI sem admin, IPC com service |
| Detecção HW | Parcial (registry/WMI) | v0.1 — portar + fingerprint |
| Capability probe | Ausente | v0.1 — passagem 1 (spec) |
| Sensores | Parcial (WHEA, clocks) | v0.1 — via service |
| Driver | WinRing0 (blocklist) | v0.1 — PawnIO |
| Core Service | Stub vazio | v0.1 — esqueleto funcional |
| Safe Loop | Ausente | v0.2 |
| GPU tuning | Ausente | v0.3 |
| Otimizador | Stub | v0.6 |
| Persistência | profiles.json | v0.7 — SQLite |
| UEFI | Stub | v0.8 |

**Working tree:** clean slate intencional — código v1 removido do disco, consultável via git (`aae94bc`). v1 **não será evoluído**; apenas referência do que não repetir.

**Próximo passo ativo:** v0.1 — Fundações (detecção, capability probe, PawnIO, Core Service, shell UI).

---

## 1. Princípios de design

Cinco regras que decidem qualquer dúvida de arquitetura ao longo do projeto:

1. **Segurança é feature número um, não enfeite.** O paraquedas (detecção e
   recuperação de crash) é construído *antes* de qualquer feature de tuning.
2. **Sempre varrer do lado seguro para o limite.** O sweep parte do estado
   *stock* e avança em direção ao agressivo. Um crash sempre significa "o último
   passo foi longe demais" — e reverter sempre leva a um ponto conhecido-bom.
3. **Honestidade de capacidade.** O app detecta o que é de fato ajustável
   naquele hardware e nunca promete o que não pode entregar. Onde Windows não
   alcança, ele recomenda/aplica via BIOS — mas diz isso claramente.
4. **Privilégio mínimo.** A UI nunca roda como admin. Só um serviço isolado
   toca em MSR/PCI/SMBus.
5. **Tudo é reversível ou recuperável.** Nenhuma operação sem caminho de volta
   definido antes de executá-la.

---

## 2. Realidade técnica e restrições (ler antes de tudo)

O projeto original falha porque o README descreve um produto que ignora estas
travas. O plano v2 as encara de frente.

### 2.1 Undervolt de CPU é parcialmente bloqueado

| Plataforma | Situação | Caminho viável |
|---|---|---|
| Intel 6ª–10ª gen | OC Mailbox (MSR `0x150`) funciona **se** não estiver locked pela BIOS/microcode | Windows-side direto |
| Intel 11ª gen+ | Mailbox restringido após o **Plundervolt** (CVE-2019-11157); microcode bloqueia undervolt | BIOS-only / UEFI |
| AMD Ryzen | Curve Optimizer fica no domínio do SMU; acesso runtime existe mas é arriscado e majoritariamente liberado só pela BIOS (PBO2) | Preferir BIOS/UEFI |

**Consequência arquitetural:** "undervolt automático no Windows" só funciona num
subconjunto do hardware. O app precisa de um **capability probe** que descobre o
que está destravado e escolhe o caminho (Windows-side vs. BIOS/UEFI vs.
indisponível) por máquina.

### 2.2 Treino de RAM não acontece no Windows

Timings e frequência são treinados pelo **MRC (Memory Reference Code)** durante
o POST. Depois disso o memory controller está travado. Implicações:

- Ler SPD/XMP no Windows: **possível** (informativo).
- Mexer em timings primários (CL/tRCD/tRP/tRAS) em runtime: **inviável de forma
  estável** — o controller já foi treinado.
- Treino real ("achar timings do zero" como faz o MRC): **fora de escopo** —
  seria reimplementar firmware proprietário.
- O que é viável: **escrever variáveis NVRAM da BIOS** (frequência, XMP,
  timings) que o MRC lê no próximo POST, **e validar** com memtest isolado.

Portanto Nidavellir **recomenda e aplica via BIOS, e valida** — não "treina"
RAM como o firmware faz. Vender outra coisa é desonesto.

### 2.3 GPU é o terreno limpo

NVAPI (NVIDIA) e ADLX (AMD) permitem ajustar curva V/F, offsets de clock e power
limit **inteiramente pelo Windows, sem reboot e reversível**. É a feature de
menor risco e maior retorno — por isso vem cedo no roadmap.

### 2.4 Acesso a kernel está mais difícil

- **WinRing0** tem o certificado comprometido e cai na *Microsoft Vulnerable
  Driver Blocklist*; também dispara anti-cheats. Não é base sustentável.
- **PawnIO**: alternativa moderna desenhada para ser segura (bytecode
  sandboxed para acesso a MSR). Boa opção interina.
- **Driver WDF próprio com attestation signing** (exige conta de dev + EV cert):
  o caminho correto de longo prazo.

### 2.5 UEFI e Secure Boot

Um módulo UEFI próprio não é assinado pela Microsoft → o usuário precisa
**desabilitar Secure Boot** ou enrolar chaves. Fricção real de UX que precisa
estar no onboarding, não escondida.

### 2.6 Escrita em NVRAM da BIOS é a operação mais perigosa

Mapear offsets de variáveis Setup exige parsear o IFR da BIOS. Errar **brica a
placa**. Tem que ser opt-in, gated por banco de dados comunitário verificado, e
nunca o caminho padrão.

---

## 3. Arquitetura de alto nível

Quatro componentes, com fronteiras de privilégio claras:

```
┌─────────────────────────────────────────────────────────┐
│  UI (Tauri v2 + Svelte 5)          — usuário, sem admin   │
│  Dashboard, fluxo de forja, perfis, central de segurança  │
└───────────────┬───────────────────────────────────────────┘
                │ IPC local (named pipe + token de sessão)
┌───────────────▼───────────────────────────────────────────┐
│  Nidavellir Core Service           — Windows Service SYSTEM│
│  • dono do driver de kernel                                │
│  • motor de sweep + otimizador                             │
│  • máquina de estados do safe loop                         │
│  • watchdog + apply-on-boot                                │
│  • knowledge base (SQLite)                                 │
└───┬───────────────┬───────────────┬───────────────────────┘
    │               │               │
┌───▼────┐   ┌──────▼──────┐   ┌────▼─────────────┐
│ Driver  │   │ GPU vendor  │   │ Módulo UEFI       │
│ (PawnIO │   │ APIs        │   │ (uefi-rs, na ESP) │
│ → WDF)  │   │ NVAPI/ADLX  │   │ apply pré-OS +    │
│ MSR/PCI │   │             │   │ memtest isolado   │
│ /SMBus  │   │             │   │                   │
└─────────┘   └─────────────┘   └───────────────────┘
```

### Por que o serviço separado é central

- **Boot-time apply**: o serviço sobe antes do login e aplica o perfil
  validado. A UI nem precisa estar aberta.
- **Watchdog independente**: vigia estabilidade mesmo se a UI travar.
- **Superfície de privilégio mínima**: só o serviço tem o driver. Se a UI for
  comprometida, não há acesso direto a MSR.
- Resolve o `service.rs` que hoje é stub vazio — ele *é* o coração do produto.

---

## 4. Modelo de segurança — o Safe Loop

O recurso mais importante do produto. É uma máquina de estados que **sobrevive a
reboots**.

### 4.1 Máquina de estados

```
IDLE ──► PROBING ──► APPLYING(ponto P) ──► DWELL ──► VALIDATED
                          │                  │
                          │                  └─► UNSTABLE (soft fail)
                          ▼
                    [crash duro = reboot]
```

### 4.2 Protocolo de cada passo do sweep

1. **Antes de aplicar P**: grava em disco `{intent: P, fase, timestamp}` e
   **arma a boot-flag**.
2. **Aplica P** via driver/GPU API.
3. **DWELL**: roda o stressor por uma janela, monitorando WHEA e heartbeat.
4. **Se passou**: limpa a boot-flag, registra P como estável na knowledge base.
5. **Se WHEA correctable subiu**: *soft fail* — reverte sem crash, marca P como
   instável, recua.

### 4.3 Detecção de crash em 4 camadas

| Camada | Pega | Quando |
|---|---|---|
| WHEA correctable (delta) | instabilidade incipiente | durante o DWELL, **antes** do crash |
| Heartbeat / watchdog | freeze total | UI/stressor para de responder |
| Análise de minidump pós-reboot | BSOD | no próximo boot — bugcheck `0x101` (clock watchdog) e `0x124` (WHEA) ⇒ instabilidade de OC |
| Hardware watchdog timer | travamento sem BSOD | último recurso |

### 4.4 Recuperação pós-crash

No boot, o **serviço sobe primeiro** e lê a boot-flag:

- **Flag armada** ⇒ o último apply causou crash. Registra P como ponto de
  crash, faz *blacklist* da região ao redor, **não reaplica**, recua para o
  último ponto validado.
- **Flag limpa** ⇒ aplica o último perfil validado (se houver) ou continua o
  sweep.
- **3 crashes seguidos** ⇒ entra em **Safe Mode**: aplica perfil *stock*, não
  toca em mais nada, e avisa o usuário na UI.

Sempre existe um **perfil stock/seguro** guardado como destino de recuperação.

---

## 5. Motor de otimização — sweep inteligente

O erro do projeto antigo: sweep linear de **um único parâmetro** e um
`optimizer.rs` vazio prometendo "Bayesian". A v2 propõe uma busca **estagiada e
multidimensional**, pragmática.

Por que não Bayesian puro de cara: estabilidade é uma função-penhasco (cliff),
não suave. GP puro sofre com fronteiras duras. A abordagem certa modela a
**fronteira de estabilidade** explicitamente.

### Fase A — Fingerprint + capability probe
Identifica CPU/GPU/placa/BIOS e descobre o que está destravado (mailbox? API de
GPU? variáveis NVRAM conhecidas?). Define quais eixos são ajustáveis.

### Fase B — Priors
Carrega pontos de partida conhecidos-bons do histórico local e do banco
comunitário para aquele silício. Em vez de varrer −200 mV às cegas, começa em
−50 mV. Isso é o "inteligente" de verdade.

### Fase C — Mapeamento de fronteira (por eixo)
Para cada eixo independente, **bisseção** até achar o penhasco de instabilidade:
aplica → DWELL com stressor → checa WHEA/crash → estreita o intervalo. Acha
`V_crash` e aplica **margem de segurança** (recuo fixo + percentual).

### Fase D — Otimização multi-eixo
Os eixos interagem (menos Vcore reduz calor, o que muda o PL sustentável).
Algumas rodadas de **coordinate descent** em torno dos ótimos por eixo. Opcional
mais tarde: BO restrita (um GP de viabilidade + um GP de objetivo).

### Fase E — Validação
Runs longos de confirmação nos pontos escolhidos. **Confidence grading:**
Bronze (validado curto) / Silver (validado longo) / Gold (validado + UEFI
memtest, quando aplicável).

### Fase F — Síntese dos perfis
Combina os eixos validados nos 3 perfis nomeados. Estes **sim** são
multidimensionais (Godforge = C-states OFF + PL alto + turbo máx + undervolt,
tudo junto), ao contrário do projeto antigo onde só um eixo mudava.

### Stressor — exercitar o que importa
O loop escalar do projeto antigo (`sin`/`sqrt`) não valida nada. A v2 precisa de:
- kernel **AVX/AVX-512** (instabilidade de undervolt aparece sob AVX),
- teste de **cache e memória**,
- carga de **GPU** dedicada.
Padrão da indústria: estilo Linpack / y-cruncher.

---

## 6. Camadas de acesso a hardware

Reaproveita a ideia de 3 camadas do projeto original, com escopo honesto.

### Camada 1 — Universal (Windows, sem reboot) — ~maior parte do ganho
- **CPU**: PL1/PL2, turbo ratios, C-states (MSR direto — funciona em tudo);
  undervolt via OC Mailbox / Curve Optimizer (**gated por capability probe**).
- **GPU**: curva V/F, offsets de clock, power limit via NVAPI/ADLX (funciona,
  reversível).
- **Leitura**: SPD/XMP via SMBus, ReBAR via PCI config, sensores.

### Camada 2 — UEFI / NVRAM (precisa de reboot)
- Aplicar variáveis Setup da BIOS: frequência de RAM, XMP, ReBAR, enables de
  C-state.
- Gated por banco comunitário por placa + versão de BIOS.
- IFR parser para descobrir offsets (longo prazo).

### Camada 3 — Profundidade (pesquisa / avançado)
- Timings secundários/terciários via registradores do IMC, LLC, VRM.
- Só com contribuição comunitária; nunca caminho padrão; alto risco.

---

## 7. Persistência — knowledge base e o "modelo de silício"

O ativo de longo prazo do projeto. Não é só salvar `profiles.json`.

- **Store**: SQLite no lado do serviço.
- **Fingerprint da máquina**: hash de CPU + GPU + placa + versão de BIOS — chave
  de tudo.
- **Por fingerprint, guarda**: a fronteira de estabilidade mapeada (pontos
  estáveis e de crash), perfis validados com confidence grade, histórico de runs
  e de crashes.
- **Portabilidade**: export/import de perfis em JSON assinado.
- **Priors**: alimenta a Fase B do otimizador — quanto mais runs, mais esperto o
  sweep fica.
- **Banco comunitário** (v1.0): submissão anônima opt-in do fingerprint +
  fronteira; vira priors para outros usuários do mesmo hardware.

---

## 8. UEFI e integração com BIOS

### Módulo UEFI (uefi-rs em Rust, ou EDK2)
Colocado na ESP, disparado por uma entrada **BootNext one-shot** (roda uma vez,
não vira boot permanente). Valor real e honesto:
- Aplicar settings num ambiente pré-OS limpo.
- Rodar **memtest isolado** (estilo MemTest86) para validar RAM.
- Servir como ponto de rollback seguro: roda antes de o OS poder ser corrompido.

Não faz: re-treino de MRC do zero (fora de escopo).

### Pipeline completo (v0.8+)
```
Windows: sweep + perfil  ──►  agenda BootNext  ──►  reboot
   ▲                                                  │
   │                                                  ▼
   └──  reboot  ◄──  Windows  ◄──  UEFI: aplica + memtest + grava resultado
```

### Fricções a comunicar no onboarding
- Secure Boot precisa ser desligado (ou chaves enroladas).
- Escrita em NVRAM é a operação mais perigosa — opt-in explícito, dupla
  confirmação, e nunca sem entrada verificada no banco comunitário.

---

## 9. Interface (UX)

Tauri v2 + Svelte 5. Princípio central: **mostrar o que É possível, não o que
não é**. O app nunca frustra — sempre entrega valor dentro das capacidades reais
do hardware.

### 9.1 Relatório de capacidade — a tela mais importante

É a primeira coisa que o usuário vê após o onboarding. Divide os achados em
três buckets com linguagem clara e não técnica:

**Automático** (verde) — Nidavellir faz sozinho, sem ação do usuário.
Exemplos: GPU undervolt, CPU power limits, turbo ratios, apply-on-boot.

**Precisa da sua ação** (âmbar) — Uma mudança simples na BIOS libera ganho
real. O app detecta o estado, calcula o ganho estimado e entrega o passo a passo
específico para o modelo de placa detectado. Exemplos típicos: XMP desligado
(RAM rodando na frequência JEDEC padrão), Resizable BAR inativo.
O botão "como fazer" abre instruções inline — o usuário nunca precisa procurar
no Google.

**Bloqueado por hardware** (cinza) — Mostra o que existe dentro dos limites
atuais e, quando aplicável, qual versão do Nidavellir vai desbloquear via UEFI.
Não promete o que não pode entregar. Nunca esconde a limitação.

**Por que isso funciona para placas budget**: uma Biostar H610 com DDR4-2133
ainda recebe valor real — GPU undervolt funciona, XMP pode ser habilitado pelo
usuário, PL/turbo via MSR funciona. A seção cinza explica o chipset H-series
sem undervolt de CPU sem parecer erro do software.

**Framing motivacional**: não é "aqui estão as features". É "aqui está o que
você está deixando na mesa agora" — RAM rodando a 2133 MHz em vez de 3600 MHz
é dinheiro perdido que o app ajuda a recuperar.

**Probe em duas passagens**:
- Passagem 1 (por especificação, instantânea, sem driver): estima capacidades
  pelo chipset/vendor detectado.
- Passagem 2 (por tentativa, requer driver): confirma o que está de fato
  destravado — escreve um offset de teste de −5 mV e verifica se tomou efeito.
  Isso pega casos onde a BIOS trava o mailbox em chipsets que deveriam suportá-lo.

### 9.2 Onboarding (wizard de 3 passos)
1. Detectar hardware e rodar probe.
2. Mostrar relatório de capacidade + aceite de risco explícito.
3. Escolher objetivo inicial (GPU primeiro, tudo, ou modo explorar).

### 9.3 Dashboard
Sensores ao vivo: clock, utilização, temperatura, potência, WHEA counter, estado
da boot-flag, perfil ativo.

### 9.4 Fluxo "Forja"
Usuário escolhe o objetivo → app roda o sweep estagiado → **visualização da
fronteira de estabilidade sendo mapeada em tempo real** (não só barra de %).
Mostrar cada passo: "testando −120 mV… estável. testando −130 mV… instável,
recuando." O usuário entende o que está acontecendo sem precisar ser expert.

### 9.5 Perfis
Os 3 perfis gerados + custom, com badge de confiança (Bronze/Silver/Gold),
botões aplicar / reverter / definir-no-boot. Mostrar o delta em relação ao stock
em linguagem concreta: "−18°C · −38 W · mesma performance".

### 9.6 Central de Segurança
Histórico de crashes, estado atual da boot-flag, botão de **pânico (revert
imediato para stock)**, indicador de Safe Mode, log de aplicações de perfil.

### 9.7 Modo Avançado
Ajuste manual por eixo, curva V/F bruta, leitura de MSR — escondido por padrão,
para usuários que já sabem o que querem.

### 9.8 Comunidade
Navegar e contribuir perfis para a sua placa. Ver cobertura do banco por
modelo de board.

---

## 10. Roadmap

Reordenado por **risco e dependência** — segurança antes de tuning, GPU antes
de RAM.

| Versão | Entrega | Por quê nessa ordem |
|---|---|---|
| **v0.1 — Fundações** | Detecção de HW + capability probe, sensores, integração do driver (PawnIO), esqueleto do Core Service, shell da UI | Base; nada funciona sem isso |
| **v0.2 — Safe Loop** | Máquina de estados com boot-flag, watchdog, análise de minidump, recuperação pós-crash, Safe Mode | **Paraquedas antes do salto** |
| **v0.3 — Tuning de GPU** | Sweep completo de undervolt/OC de GPU, Windows-side | Menor risco, retorno imediato, reversível |
| **v0.4 — CPU power/turbo** | PL1/PL2, turbo ratios, C-states + otimizador estagiado para esses eixos | MSR universal, funciona em todo hardware |
| **v0.5 — Undervolt de CPU** | OC Mailbox (Intel) / Curve Optimizer (AMD), gated por capability probe | Mais arriscado e dependente de hardware |
| **v0.6 — Otimizador v2** | Coordinate descent multi-eixo, modelo de fronteira, síntese dos 3 perfis, confidence grading | Precisa dos eixos das versões anteriores |
| **v0.7 — Persistência** | Knowledge base SQLite, modelo de silício, portabilidade de perfis, apply-on-boot | Transforma runs em inteligência |
| **v0.8 — Módulo UEFI** | Apply pré-OS + memtest isolado, BootNext one-shot | Habilita o caminho de RAM |
| **v0.9 — BIOS/NVRAM (Camada 2)** | Escrita de variáveis Setup (RAM/ReBAR/XMP), gated por banco comunitário | Operação mais perigosa — por último, bem protegida |
| **v1.0 — Plataforma comunitária** | Banco opt-in, sweeps guiados por priors, polimento | Efeito de rede; depende de massa de dados |

---

## 11. Stack tecnológica

| Camada | Escolha | Observação |
|---|---|---|
| Driver de kernel | PawnIO (interino) → WDF próprio com attestation signing | Evitar WinRing0 (blocklist) |
| Core Service | Rust, Windows Service | Dono do driver, roda como SYSTEM |
| UI | Tauri v2 + Svelte 5 | Manter — boa escolha |
| IPC UI↔Service | Named pipe + token de sessão | UI sem privilégio |
| API de GPU | NVAPI + ADLX (bindings Rust) | Windows-side, reversível |
| Módulo UEFI | uefi-rs (Rust) ou EDK2 | One-shot via BootNext |
| Otimização | Busca estagiada própria (bisseção + coordinate descent); BO restrita opcional depois | Não prometer "Bayesian" cedo demais |
| Persistência | SQLite (knowledge base) + JSON (perfis portáveis) | — |
| Stressor | Kernel AVX próprio + teste de memória + carga de GPU | Estilo Linpack/y-cruncher |
| Testes | Testes unitários na lógica de perfil e no bit-packing de MSR | Inegociável para software que escreve MSR |

---

## 12. Motor de silicon — GPU (referência técnica)

> Esta seção detalha por que e como o Nidavellir supera o undervolting manual
> de ferramentas como MSI Afterburner no eixo de GPU.

### O problema fundamental das ferramentas atuais

Afterburner/EVGA Precision operam assim: o usuário arrasta um ponto na curva
V/F, roda Furmark 15 min, se travar recua 10 mV, repete. Dois problemas graves:

1. **Detectam instabilidade pelo crash** — encontram o penhasco caindo nele.
2. **Aplicam offset plano** — assumem que a curva de silício é uniforme em toda
   a faixa de frequência. Não é.

### Erros computacionais silenciosos — o gap real

Entre "perfeitamente estável" e "trava o driver" existe uma zona de **erros
computacionais silenciosos**: a GPU produz resultados errados sem crashar. Um
shader que calcula física, um render que devia ser determinístico, um modelo de
IA rodando inferência — todos podem estar gerando lixo sem nenhum aviso visível.

As ferramentas atuais não detectam isso. O Nidavellir sim, via **compute
validation**: roda kernels com resultados conhecidos e verifica o retorno.
Qualquer divergência é instabilidade detectada *antes* do crash.

### A curva V/F não é plana — ela tem forma

Cada GPU tem uma relação voltagem × frequência que não é uniforme. A 1800 MHz
pode ser estável com −150 mV. A 2100 MHz pode precisar de apenas −80 mV. Um
offset plano de −100 mV pode ser conservador demais em baixa frequência e
agressivo demais em boost máximo simultaneamente.

O Nidavellir mapeia múltiplos pontos da curva e gera um **perfil custom em
formato nativo** da API (NVAPI/ADLX), não um offset plano. O resultado é um
silício mais explorado e mais estável ao mesmo tempo.

### A dimensão esquecida — VRAM

VRAM tem voltagem e timings próprios. Erros de VRAM são comuns (Micron GDDR6
em RTX 30xx era famoso por isso) e causam artefatos visuais, corrupção de
textura e crashes que parecem instabilidade de core mas não são.

O Nidavellir testa VRAM separadamente antes de tocar no core. Sem isso, você
pode passar horas ajustando Vcore enquanto o problema real é a memória.

### Temperatura como variável de primeiro nível

A fronteira de estabilidade se move com temperatura. Silicon quente precisa de
mais voltagem. O sweep precisa ser feito em **equilíbrio térmico** — não no
cold-start. E o perfil final precisa ter uma margem de temperatura embutida
para o pior caso (verão, case quente, overclock de memória adicional).

### Filosofia do sweep — undervolt primeiro, não OC

O objetivo não é maximizar clock — é maximizar o clock **sustentado** com
mínima voltagem. A distinção é crítica:

- Ferramentas atuais (Afterburner) trabalham com peak boost clock: o que a GPU
  faz por 3 segundos antes de throttle térmico/power. Não é o que importa.
- O Nidavellir trabalha com **average sustained clock**: o que a GPU mantém em
  equilíbrio térmico. Esse é o número real de performance.

O paradoxo comprovado: undervolt → menos calor → menos throttle → clock médio
sustentado MAIOR que stock. UV bem feito frequentemente supera OC puro.

### Pipeline completo de caracterização de GPU

```
FASE 0 — Baseline real (não o peak boost)
  ├─ Fingerprint: VRAM vendor (Samsung/Micron/Hynix), UUID do chip, TDP nominal
  ├─ Aquecer GPU até equilíbrio térmico (10–15min de carga)
  ├─ Medir average sustained clock nos últimos 5min (não o pico)
  ├─ Medir voltagem média, power draw, temperatura de GPU e VRAM
  └─ → "stock real" — referência de comparação para todos os perfis

FASE 1 — Diagnóstico de VRAM [decisão: Opção B]
  ├─ Alocar buffer cobrindo toda a VRAM disponível via Vulkan
  ├─ Escrever padrões determinísticos (walking bit, March C)
  ├─ Ler de volta e verificar bit a bit
  ├─ Se VRAM falhar em stock: reportar ao usuário e PARAR
  │   → "VRAM instável em configuração stock. Tuning de core não fará
  │      diferença — o problema está nos chips de memória."
  └─ Se passar: confirmar que a base é sólida antes de qualquer UV

FASE 2 — Teto do silício (remove voltagem como restrição)
  ├─ Elevar voltagem +25mV temporariamente
  ├─ Rodar carga → medir clock máximo limitado só por térmico/power
  └─ → "teto do silício" — o que esse chip consegue sem restrição de V

FASE 3 — Bisseção de voltagem mínima no teto
  ├─ Fixar V/F ceiling no teto da Fase 2 (flat curve via NVAPI/ADLX)
  ├─ Começar na voltagem stock para aquela frequência
  ├─ Descer em steps de 5–10mV
  ├─ Cada step: estabilizar temperatura → compute validation com workload diverso
  │   ├─ ALU pesado: matrix multiply, operações transcendentais
  │   ├─ Memória-bound: large stride access, random read/write em buffer VRAM
  │   └─ Mixed: texture sampling simulado — pega instabilidade de memory controller
  ├─ Detecção de instabilidade: divergência no resultado (não crash)
  └─ Achar cliff → aplicar margem: buffer fixo + coeficiente de temperatura

FASE 4 — Fallback por steps de clock
  ├─ Se Fase 3 não estabilizar no teto: descer 25–50MHz
  ├─ Critério de parada: UV estável com ≤1–2% vs. sustained baseline de stock
  ├─ Cada ponto (freq, V_mínimo) é registrado no knowledge base
  └─ → mapa de tradeoffs freq × voltagem do silício

FASE 5 — Síntese dos perfis
  ├─ Godforge: teto máximo com V mínimo estável — curva V/F em formato nativo
  │   (todos os pontos ≥ target freq nivelados ao V encontrado via NVAPI)
  ├─ Brokkr's Best: joelho da curva perf/watt nos dados da Fase 4
  └─ Deep Calm: freq onde ≥95% do sustained baseline é mantido com menor watt
```

### Por que o workload diverso pega o memory controller

O memory controller compartilha o trilho de voltagem com os shader cores no
NVIDIA. Um undervolt agressivo pode desestabilizar o MC antes dos shaders —
gerando corrupção de dados em trânsito sem crashar o driver. Um kernel de ALU
puro passa; um kernel com acesso memória-bound falha. A Fase 1 (VRAM stock)
mais o workload misto na Fase 3 cobrem esse caso sem precisar de um sweep
separado de VRAM como eixo.

### O que o usuário recebe

- **Godforge**: clock máximo que o silício sustenta com mínima voltagem — curva
  nativa, não offset plano. Mais estável e geralmente mais rápido que stock.
- **Brokkr's Best**: joelho da curva perf/watt. Clock onde cada MHz ainda vale
  o watt gasto. Ideal para silêncio e eficiência.
- **Deep Calm**: mínimo de watt com performance imperceptivelmente diferente
  do stock. Delta real de temperatura e ruído de cooler.

### Stack técnica para GPU

- **Vulkan compute**: cross-vendor (NVIDIA + AMD), kernels de validação e stress.
- **NVAPI** (NVIDIA): leitura/escrita da curva V/F via `NvAPI_GPU_SetVFPCurve`,
  power limit, leitura de voltagem real em tempo real.
- **ADLX** (AMD): equivalente para RDNA2+, acesso a tuning por ponto da curva.
- **GPU-Z–style queries**: VRAM vendor via memória SPD-equivalent, UUID do chip.

---

## 13. Riscos principais

| Risco | Mitigação |
|---|---|
| Lockdown Plundervolt impede undervolt de CPU | Capability probe + fallback para caminho BIOS/UEFI; comunicar no onboarding |
| Secure Boot bloqueia o módulo UEFI | Avisar cedo; oferecer modo só-Windows como alternativa funcional |
| Blocklist de driver vulnerável | Migrar de WinRing0 para PawnIO e depois driver próprio assinado |
| Brick por escrita errada em NVRAM | Camada 2 opt-in, dupla confirmação, gating por banco comunitário verificado |
| Falso-positivo de anti-cheat | Documentar; recomendar não rodar tuning com jogos competitivos abertos |
| Garantia / responsabilidade | Aceite de risco explícito; perfil stock sempre recuperável; GPLv3 já cobre o "sem garantia" |
| Crash duro instantâneo não dá tempo de reverter | Por isso o safe loop é baseado em sobreviver ao reboot, não em "reverter antes" |

---

## Resumo de uma linha

Construa o paraquedas (v0.2) antes de saltar; entregue valor cedo pela GPU
(v0.3); seja honesto sobre o que Windows não alcança; e trate a knowledge base
de silício como o ativo de longo prazo do projeto.
