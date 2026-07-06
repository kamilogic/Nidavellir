# Plano — Qualificação v8, margem por perfil e refinamento pós-crash

Estado: aprovado em 2026-07-04 (análise da sessão de revisão do algoritmo F2).
**Progresso: Fase 1 COMPLETA em código (2026-07-05, contrato v9) — itens 1.1–1.6 implementados;
hardware gate pendente.** Desvios do plano: VramPressure usa alvo fixo de até ~2 GB em tabelas de
256 MB com degradação por error-scope (não 70% da VRAM — orçamento via NVML fica para um passo
futuro se a evidência pedir); telemetria 1.5 tem o agregador puro
(`qualification_failure_histogram`, core) — wiring de log/UI fica com a Fase 2.3. Item 1.6
verificado: MixedGame decompõe em BoostEdge/TextureRop/PowerRender (cada um com golden própria) e
ComputeBurst é known-answer — cobertura de silent error confirmada sem mudança.
Dono: backend/algoritmo (Claude). Frontend via `docs/contracts/ui-backend.md` (Codex).

## Contexto e objetivo

A qualificação v7 (High-FPS / Texture / Transitions + golden-sample + exact-Apply 3×5 min)
ainda deixa passar pontos que crasham/TDR em jogo real. Objetivo do produto: usuário leigo
aperta "Forjar", recebe 3 perfis (Godforge / Brokkr's Best / Deep Calm) que não crasham em uso
diário, sem precisar entender nada.

Diagnóstico raiz (sessão 2026-07-04):

1. **Gap determinístico dos workloads** — os padrões v7 não exercitam:
   - transientes de droop em cadência de frame (jogos oscilam carga a cada 6–16 ms; o
     `idle_pulses` atual pulsa a cada 750 ms com sleep de 100 ms — `run_render_profile`,
     `crates/gpu-stress/src/lib.rs`);
   - pressão de VRAM/memory controller (textura fonte 1024² + RT 1536² ≈ residente em L2;
     jogos usam 7–11 GB com streaming);
   - mix de unidades além de fragment/ALU/TMU/ROP (sem vertex/geometry/depth);
   - silício frio (qualificação roda com a placa quente; boost NVIDIA sobe clock a frio —
     crash clássico em menu/launch).
2. **Gap estatístico** — Vmin é taxa de erro rara em escala de horas; nenhum teste finito
   (nem o próprio jogo por 1 h) prova 100 h. Resolve-se com guard band + correção por evento,
   não com mais dwell.
3. **Fragilidade fail-closed composto** — gates individualmente corretos compõem runs que
   terminam com ZERO perfis (ex.: flag térmica de hotspot de memória, handoff 2026-07-04).
   Para o leigo, perfil conservador provisório > nenhum perfil.

Decisão de direção: Fase 1 (workloads v8) primeiro; Fase 4 (refinamento por evento) é seguro
para a cauda estatística e fica condicionada ao resultado medido da Fase 1. Fase 4 NÃO é
monitoramento durante gameplay — é reação a evento TDR/reboot que o Safe Loop já observa.

## Critério de aceitação global (Fase 1)

Os pontos conhecidos que **passam v7 hoje mas crasham em jogo** no rig de teste (3060 Ti)
formam a suíte de regressão física. v8 é aceita quando rejeita esses pontos na mesma tensão
em que v7 os aprovava. Cada crash em jogo futuro vira novo caso da suíte (registrar
clock/mV/contexto em `memory.md`). Sem esse critério, "melhorar o stress" não é mensurável.

### Suíte de regressão física — ground truth do usuário (2026-07-05, RTX 3060 Ti)

| Ponto | Status real (jogo) | v8 (run 2026-07-05) |
|---|---|---|
| 1800 MHz @ 875 mV | ESTÁVEL (daily driver, meses) | nunca testado (warm-start entrou em 837, falhou, clock descartado) |
| 1800 MHz @ 868 mV | INSTÁVEL em jogo | — |
| 1830 MHz @ 875 mV | INSTÁVEL em jogo | — |
| 1815 MHz @ 843–856 mV | (implícito instável: mais fundo que 1800@868) | **APROVADO 4×60 s** ← falso negativo |
| 1860 MHz @ 875 mV | (implícito instável: 1830@875 já falha) | **APROVADO 4×60 s** ← falso negativo |

**Gap medido: ~4–5 bins (~25–32 mV)** entre o Vmin do v8 (4×60 s) e o Vmin de jogo real.
Critério para a próxima iteração: rejeitar 1815@856 e 1860@875; a fronteira de 1800 deve
aterrissar em ~875 (±1 bin). A reconciliação p95 estrita impediu a publicação dos falsos
negativos nesta run — NÃO relaxar antes de fechar o gap de fidelidade.

## Fase 1.7–1.9 — carga realista (aprovado 2026-07-05, prioridade sobre Fases 2–3)

Aprendizado da run: TODAS as falhas v8 dispararam na fase `texture-rop` — o caminho TMU é o
detector sensível deste chip. Mas a textura fonte atual é 1024² (4 MB ≈ residente em L2);
jogo amostra GBs com cache miss constante (TMU + latência DRAM simultâneos sob droop).

- **1.7 Texture sob pressão de memória real (maior alavancagem)**: o caminho texture-rop passa
  a amostrar de um conjunto de texturas GRANDE residente em VRAM (alvo 512 MB–1 GB, alocação
  OOM-guarded em degraus como a VramPressure), com UVs dependentes cache-defeating. TMU +
  memory controller no MESMO shader (concorrente, como jogo) — não em segmentos sequenciais.
  Golden própria (acesso determinístico). Aplicar primeiro ao workload TextureRop; avaliar
  estender ao MixedGame depois do gate físico.
- **1.8 Recuperação para cima na descida**: quando o bin INICIAL de um clock (warm-start/
  predição isotônica) falha a qualificação, subir 1 bin e re-tentar (até ~4 subidas) antes de
  descartar o clock. Na run, 1800 MHz foi descartado inteiro porque o warm-start entrou abaixo
  da fronteira real — perdendo exatamente o melhor ponto conhecido (1800@875).
- **1.9 Gate físico**: re-run Standard e comparar com a tabela acima. Só então recalibrar as
  margens da Fase 2.1 com o gap residual (se 1.7 recuperar 2–3 bins, +2/+3 bins de margem
  cobrem a cauda estatística; senão, margens maiores).

Filosofia mantida (decisão do usuário): failure-seeking — o qualifier deve FORÇAR o erro para
sobrar margem de segurança; um rail "sujo" de testes anteriores que ainda aprova ponto é
indício de carga insuficiente, não de excesso de rigor.

## Confirmação de regime (aprovado 2026-07-06 — implementar APÓS os dados da run v11)

Problema: um ponto anchorado em (T, V) entrega sustained S = T + 1–2 bins (compensação térmica
do boost NVIDIA); o regime elétrico real é (S, V), que hoje nunca é testado diretamente — a
reconciliação cruzada de p95 cascateia até excluir tudo (run 2026-07-05) e morre no topo da
fronteira. Desenho acordado (proposta do usuário, refinada):

1. Descida inalterada (sustained S já é medido por bin qualificado).
2. Na síntese, para cada um dos 3 pares selecionados (T, V_apply), ANTES do exact-Apply:
   **confirmação de regime** = re-anchorar (S′, V_apply), S′ = bin real ≥ sustained medido, e
   rodar o conjunto de qualificação 4×60 s. Barato (4 min) antes do caro (20 min).
   Anchorar S′ entrega ~S′+compensação ⇒ o teste é automaticamente mais duro que o uso real
   (perfil aplicado = âncora T, regime real ≤ S) — um nível de promoção basta, sem recursão.
3. Falha na confirmação ⇒ subir o boundary daquele clock para o próximo bin validado da descida
   (estoque de fallback já qualificado) e ressintetizar — nunca cascata/zero perfis.
4. Exact-Apply continua validando a curva exata aplicada (âncora T). Reconciliação cruzada de
   p95 é substituída por "p95 observado ≤ regime provado do próprio par" (auto-contida).
5. S′ − base(V) além do envelope físico ⇒ fail-closed (par rejeitado).
6. Observation store ganha o campo de regime provado por par. Contrato v12; safety audit
   obrigatório (muda semântica de gate).

Racional da colocação (perfis, não boundary/descida): precisão só importa para o que é
aplicável; confirmar cedo gasta dwell em pontos que a seleção descarta. Se a prática mostrar
falhas de regime frequentes (muitas iterações de re-síntese), promover a confirmação para o
boundary de cada clock é mudança de um ponto de chamada. Custo esperado: +12 min por run limpa.

---

## Fase 1 — Workloads v8 (a alternativa preferida do usuário)

Tudo em `crates/gpu-stress/src/lib.rs` + fiação em `crates/service/src/gpu_undervolt.rs` /
`gpu_power_sweep.rs`. Bump de `F2_QUALIFICATION_CONTRACT_VERSION` (evidência v7 não destrava
Apply v8; negativos continuam valendo — mesmo padrão do bump v4→v7).

### 1.1 Segmento de oscilação em cadência de frame (maior prioridade)

- Novo `VfWorkload::FrameCadence` (ou variante de timing do RENDER_SHADER): burst pesado de
  ~8–12 ms seguido de idle de 2–8 ms (poll + sleep curto), repetido por todo o segmento.
  Alvo físico: droop di/dt na transição carga→alívio→carga, onde o clock salta com a tensão
  ainda afundada.
- Requisitos: manter golden-checksum (mesmo shader/geometria do PowerRender ⇒ reusa
  `goldens.power`); frame de trabalho limitado (lição do comentário DIM/overdraw: frames
  curtos e muitos, nunca um frame gigante perto do watchdog de 2 s).
- Variar o período do ciclo dentro do segmento (ex.: 5/8/12/16 ms) — droop ressoa com a
  malha LC do VRM em frequências específicas; varrer o período cobre mais assinaturas.

### 1.2 Workload de pressão de VRAM

- Novo `VfWorkload::VramPressure`: alocar ~70–75% da VRAM livre em texturas grandes
  (várias de 512 MB–1 GB); fragment shader amostra com stride pseudo-aleatório determinístico
  (derrota L2, força DRAM). Core + memory controller simultâneos mudam o ruído no rail.
- Golden: padrão de acesso determinístico ⇒ checksum determinístico; capturar golden própria
  no stock (estender `RenderGoldens` com campo `vram`).
- Fail-safe: falha de alocação → reduzir alvo em degraus (50%, 30%); abaixo de 30%, segmento
  vira no-op registrado em log (nunca aborta a run por isso).
- Cuidado com TDR: acesso a DRAM é lento; dimensionar draw para manter frames < ~100 ms.

### 1.3 Mix de unidades — passo de geometria/depth

- Novo workload com malha instanciada (grid procedural, alguns milhões de vértices) +
  depth attachment + culling: exercita vertex fetch/raster/ROP-depth que hoje não existem.
  wgpu suporta tudo isso.
- Golden: render determinístico ⇒ checksum ok.
- Gap documentado e aceito (sem RT/tensor/mesh shaders — exigiria D3D12 puro; reavaliar
  apenas se a suíte de regressão física apontar falhas que v8 não reproduz).

### 1.4 Padrões v8

- `V8HighFps` / `V8Texture` / `V8Transitions`: evoluções dos v7 inserindo sub-segmentos
  FrameCadence (especialmente em Transitions, que hoje alterna em escala de segundos).
- Novo `V8Memory`: VramPressure dominante + FrameCadence + TextureRop.
- Qualificação de fronteira: 4 padrões × 60 s (Standard/Long). Exact-Apply: 4 × 5 min.
  Custo adicional: +1 min por candidato de fronteira, +5 min por par de Apply — aceitável.

### 1.5 Telemetria por fase de falha

- Agregar `failure_phase` por (clock, mV, padrão, fase) no observation store
  (`crates/core/src/f2_observation.rs`). Uso: (a) saber quais fases realmente predizem
  crash e reponderar padrões com dados; (b) alimentar margem adaptativa (Fase 2.3).

### 1.6 Verificar cobertura golden do MixedGame

- `golden_for_workload` retorna `None` para `MixedGame`; confirmar se o caminho de
  verificação própria dele cobre o modo golden. Se o segmento mais "parecido com jogo"
  roda sem detecção de silent error na v7, corrigir na v8.

### Validação Fase 1

- Automática: `cargo check/test --workspace`; testes puros de plano de segmentos (pesos,
  durações, golden por workload); teste de alocação VRAM com mock.
- Manual (usuário aprova cada passo de hardware): capturar goldens stock v8; rodar a suíte
  de regressão física (pontos crash-em-jogo conhecidos) e confirmar rejeição; rodar Forge
  Standard completo e comparar fronteira v8 vs v7 (esperado: fronteira ~1–2 bins mais alta).

---

## Fase 2 — Margem por perfil + degradação graciosa

### 2.1 Margem de Apply por perfil

- Hoje: `APPLY_MARGIN_MV = 12` (~2 bins) igual para os três perfis — provado insuficiente.
- Novo: margem em BINS por perfil — Godforge +2, Brokkr's Best +3, Deep Calm +3
  (constantes nomeadas; Deep Calm gasta ~5 W a mais por bin, irrelevante perto da promessa
  "esquece que existe").
- A qualificação exact-Apply continua rodando no bin margeado (semântica atual preservada).
- Expor `apply_margin_bins` por perfil no payload (aditivo) — atualizar
  `docs/contracts/ui-backend.md`.

### 2.2 Degradação graciosa (mata o modo "ZERO perfis")

- Exact-Apply `Inconclusive`/ambíguo após retries: em vez de excluir o par, publicar com
  +1 bin EXTRA de margem e status `provisional`. Fail-closed permanece absoluto para
  segurança de hardware (write/verify/reset/blacklist) — a distinção é qualidade de
  evidência, não segurança.
- `finished` passa a admitir perfis provisionais (campo aditivo `qualification_status:
  qualified | provisional` por perfil). Contrato UI: badge "provisório" com copy leiga.
- Promoção provisional→qualified: via Fase 4 (X horas sem crash) ou re-run.

### 2.3 (Depois, com dados da 1.5) Margem adaptativa pela forma do penhasco

- Falhas espalhadas por ≥3 bins antes do hard-fail (penhasco raso) ⇒ +1 bin de margem.
  Penhasco seco (limpo até DeviceLost) ⇒ margem padrão. Heurística pura + testável.

---

## Fase 3 — Verificação cold-start

- No primeiro boot com perfil aplicado após um forge (placa fria), rodar 60–90 s de
  verificação v8 (FrameCadence + HighFps curto) antes de considerar o perfil confirmado
  a frio. Gancho: caminho de reapply-on-boot existente do Safe Loop.
- Falha a frio ⇒ step-up de 1 bin + re-apply + log/notificação (usa o mecanismo da Fase 4.2;
  se Fase 4 for adiada, apenas registra e marca o perfil `provisional`).
- UX: rodar em idle logo após o boot, com notificação discreta; nunca bloquear login.
  Documentar comportamento no contrato UI.

---

## Fase 4 — Refinamento por evento (seguro para a cauda estatística)

Condicionada ao resultado da Fase 1: se a suíte de regressão física zerar E o uso real do
rig de teste ficar semanas sem crash, pode ser adiada. Recomendação de engenharia: shippar
mesmo assim — custo em gameplay é zero (nada roda durante o jogo; é reação a evento que o
Safe Loop já observa hoje: classes 0x116/0x117, boot flag, recuperação pós-reboot).

### 4.1 Atribuição de crash a perfil aplicado

- Persistir "perfil X aplicado desde T" no estado do Safe Loop. TDR/reboot com perfil
  aplicado + classe OC/TDR ⇒ atribuído ao perfil (mesma disciplina conservadora que o
  probe F2 já usa: crash não relacionado não conta).

### 4.2 Step-up automático limitado

- Crash atribuído ⇒ subir o perfil 1 bin, re-aplicar, notificar ("detectamos instabilidade
  e reforçamos seu perfil — nada a fazer"). Idempotente; máx. 3 steps por perfil; no 4º,
  reverter para stock + marcar perfil inválido + sugerir re-forge.
- Cada step-up alimenta a suíte de regressão (1.6) e o observation store.

### 4.3 Promoção por horas de uso

- N horas de gameplay sem crash atribuído ⇒ `provisional` → `qualified`; opcionalmente
  OFERECER (nunca automático) aperto de 1 bin.

### 4.4 Contrato UI

- Notificações, histórico de ajustes por perfil, status qualified/provisional/reinforced.
  Tudo via `docs/contracts/ui-backend.md`; sem edição direta de UI.

---

## Fase 5 — Eficiência da discovery (independente; qualquer momento)

- **Descida grossa→fina**: descer 2–3 bins por passo enquanto `Stable`; na primeira falha
  não destrutiva (ClockDrop/SilentError), refinar 1 bin para cima. Nunca bisseção agressiva
  (custo de DeviceLost é alto).
- **Monotonicidade entre clocks**: Vmin(f) é monótona — last_good do clock acima é lower
  bound para o clock abaixo dentro da mesma run (verificar se o warm-start já cobre isso
  entre targets; se não, é a economia de dwell mais barata disponível).
- **Poda por dominância**: a síntese só precisa de 3 regiões da fronteira (topo/joelho/fundo);
  grade grossa primeiro, refino ao redor do joelho de Brokkr.

## Fase 6 — Forge Knowledge / priors de frota (longo prazo, opt-in)

- Telemetria anonimizada por modelo de GPU: distribuição de Vmin por clock. Usos: seed do
  start_mv (corta discovery), margem informada por percentil populacional. Requer decisão
  de produto (privacidade/backend) antes de qualquer código.

---

## Sequenciamento e gates de decisão

1. **Fase 1** (workloads v8) — implementar 1.1 → 1.6 nesta ordem; 1.1 sozinha já merece um
   teste de hardware intermediário contra a suíte de regressão.
2. **Fase 2.1 + 2.2** — pequenas, mecânicas, podem andar em paralelo à Fase 1 (writer/
   qualificação não mudam; muda seleção de bin e status).
3. **Gate A** (após Fase 1+2 validadas em hardware): suíte de regressão física zerou?
   - Sim → Fase 3, e Fase 4 vira recomendação (decisão do usuário).
   - Não → Fase 4 obrigatória para fechar a cauda.
4. **Fase 5** intercalada quando conveniente (reduz custo de re-runs de teste).
5. **Fase 6** só após decisão de produto.

## Regras de segurança e validação (todas as fases)

- Nenhum stress/VF write/Apply/Forge roda automaticamente; hardware só com aprovação
  explícita do usuário, com comandos exatos, resultado esperado e logs a capturar.
- Auditoria `nidavellir-safety-auditor` antes de merge em: mudanças de classificação de
  dwell, semântica de gates de publish, step-up automático (4.2), cold-start (Fase 3).
- Bumps de contrato (qualificação v8) seguem o padrão existente: positivos antigos não
  destravam Apply; negativos permanecem conservadores.
- `cargo check/test --workspace` + testes puros novos por item; sem mudanças de UI diretas.
