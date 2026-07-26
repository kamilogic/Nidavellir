# Texture Hop e Endurance — especificação do teste atual

> Estado documentado: árvore de trabalho local em 2026-07-23
> Contrato de qualificação: `F2_QUALIFICATION_CONTRACT_VERSION = 25`
> Escopo: Forge F2 ativo no Windows; caminhos legados F1, FSGL1/2/3, DX11 e
> `TransitionShock` são citados apenas quando necessário para deixar claro que não fazem parte do
> gate obrigatório atual.

## 1. Resumo executivo

O Forge atual usa dois testes complementares no caminho de publicação de perfis:

- **Texture Hop v13-r3** é o detector obrigatório e relativamente rápido. Ele procura corrupção
  silenciosa, perda de dispositivo, instabilidade e fragilidade de scheduling enquanto combina
  textura/ROP, pressão de VRAM, render de potência e concorrência entre contextos GPU. Ele é usado
  tanto durante a descida da fronteira quanto no par exato de Apply.
- **Endurance** é o soak contínuo do par final. Ele reutiliza as mesmas cargas determinísticas, mas
  organiza uma sequência mais longa, pesada e termicamente acumulativa. Ele roda somente no par
  exato de Apply, depois de Texture, e é obrigatório para publicação.

Um ponto só pode ser publicado quando, na mesma execução e no mesmo par exato
`(target_mhz, apply_mv)`:

1. Texture retorna evidência `Pass` do contrato v25;
2. Endurance também termina validado;
3. o ponto sustenta telemetria de clock, tensão e potência utilizável;
4. a transação retorna a GPU ao stock e limpa o boot flag;
5. a evidência possui proveniência reproduzível;
6. os gates de potência, regime, confiança e condenação também aceitam o ponto.

`Inconclusive` não aprova e não reprova eletricamente: mantém o sistema fail-closed, sem inventar
instabilidade. `SilentError`, `Unstable` e `DeviceLost` são falhas físicas, mas possuem efeitos de
recuperação e persistência diferentes, detalhados adiante.

## 2. Onde o teste entra no programa

### 2.1 Ação do usuário e IPC

Na tela Forge, os modos ativos são:

| Seleção | IPC | Política de teste |
|---|---|---|
| Standard | `StartPowerSweep` | prova compacta recomendada |
| Long | `StartPowerSweepLong` | prova exaustiva |
| Clean Run | `StartPowerSweepClean` | mesma duração do Standard, ignorando aprendizado positivo anterior |

O método legado `StartPowerSweepFast` continua aceito no wire protocol, mas hoje é apenas alias de
Standard. Não existe mais um modo Fast sem qualificação.

Referências: `apps/ui/src/lib/views/Forge.svelte`, `crates/service/src/ipc_server.rs` e
`PowerSweepMode::f2_policy` em `crates/service/src/gpu_power_sweep.rs`.

### 2.2 Pré-condição stock antes de testar candidatos

Depois do preheat e da leitura da curva V/F, mas antes da primeira escrita de candidato, o Forge:

1. captura por 2 s cada golden stock de `PowerRender`, `BoostEdge`, `TextureRop`,
   `FrameCadence`, `GeometryDepth` e `TextureStream`;
2. exige pelo menos quatro frames e checksum determinístico durante cada captura;
3. executa o **Texture Hop completo por 60 s em stock**, incluindo a concorrência persistente
   entre o Texture Stack primário e o canary TextureRop secundário;
4. aborta a forja antes de qualquer candidato se o backend/driver não sustentar os goldens ou a
   sequência completa.

Uma falha nesse preflight é incapacidade ambiental, não prova de um bin V/F ruim. Por isso ela não
deve produzir blacklist de candidato.

### 2.3 Uso durante a descida da fronteira

Para cada clock alvo e bin de tensão elegível:

1. o motor arma a recuperação, escreve a curva ancorada e verifica o write;
2. executa 10 s de `PowerRender` para descoberta/potência;
3. se o ponto é sustentado e está fora do regime de power cap, executa **Texture Hop**;
4. se Texture passa, a descida tenta o próximo bin físico de tensão abaixo;
5. se Texture reprova, o clock para naquele limite ou realiza recuperação para cima quando veio de
   um salto;
6. se Texture é inconclusivo, preserva o último ponto raso já qualificado, encerra somente aquele
   clock e continua a forja multi-clock.

Duração de Texture na fronteira:

| Modo | Duração por bin elegível | Passes obrigatórios |
|---|---:|---:|
| Standard / Clean | 30 s | 1 |
| Long | 60 s | 1 |

O contrato v25 mantém duas fronteiras:

- **fronteira física**: ponto de descoberta mais profundo, útil para aprendizado;
- **fronteira publicável**: ponto mais profundo que também possui Texture atual completo.

Assim, um ponto mais profundo porém inconclusivo não apaga um ponto mais raso já qualificado.

### 2.4 Uso no gate exato de Apply

Depois de sintetizar Godforge, Brokkr's Best e Deep Calm, o programa deduplica os pares exatos
selecionados. Cada par ainda não aprovado na execução atual passa por:

```text
planejar par exato
  -> safety/ledger precheck
  -> armar boot flag
  -> aplicar curva ancorada
  -> verificar write, ceiling de clock e lock da tensão física selecionada
  -> Texture Hop no par exato
  -> reset stock confirmado
  -> checar envelope de publicação
  -> Endurance no mesmo par exato
  -> reset stock confirmado
  -> reconciliar clock, potência, regime e confiança
  -> publicar ou ressintetizar
```

Durações por par exato único:

| Modo | Texture exato | Endurance | Total nominal do gate |
|---|---:|---:|---:|
| Standard / Clean | 120 s | 300 s | 7 min |
| Long | 300 s | 1.200 s | 25 min |

Não existe mais watchdog global de 59/60 minutos. A execução termina o plano derivado do hardware,
a menos que haja Stop manual, falha terminal ou recuperação pendente.

Texture sempre roda antes de Endurance. Se Texture já mede potência acima do teto de publicação
(`94%` do board power limit), o par é classificado como power-bound e Endurance é evitado. Isso não
gera blacklist: o par sai da seleção da execução e o Forge tenta ressintetizar/reparar.

## 3. Motor comum e segurança transacional

Texture e Endurance usam o mesmo motor confirmado de candidato. A ordem real em
`run_confirmed_f2_step` é:

```rust
arm_boot_flag();
apply_positive_offset();
verify();
let dwell = dwell();
reset_to_stock();
clear_boot_flag(); // somente após reset confirmado
```

Propriedades importantes:

- o boot flag é armado **antes** da escrita;
- Apply e verify falham fechados;
- `DeviceLost` tenta reset, registra a falha e preserva o boot flag para recuperação no startup;
- `SilentError` e `Unstable` nunca viram validação positiva;
- `ClockDrop` e `Inconclusive` não são tratados como crash;
- uma evidência positiva só é reutilizável quando `reset_to_stock_ok == true` e
  `boot_flag_cleared == true`;
- falha de reset ou de limpeza do boot flag domina qualquer resultado anterior como
  `ResetFailed`.

Durante a descida, descoberta e Texture podem compartilhar uma única escrita ativa e realizar um
único cleanup ao final da transação. No gate de Apply, Texture e Endurance são passos confirmados
separados, cada um com seu próprio ciclo arm/apply/verify/dwell/reset.

## 4. Texture Hop v13-r3

### 4.1 Identidade e objetivo

- Enum interno: `VfQualifierPattern::V8Texture`.
- Label persistido: `v13-texture-hop`.
- Pattern lógico: `F2QualificationPattern::Texture`.
- Fingerprint atual: `f2q-texhop-v13-r3/v13-persistent-field-concurrency`.
- Strength exigida: `Fsgl4`.

O nome interno `V8Texture` foi mantido por compatibilidade de fonte. O comportamento ativo é Texture
Hop v13-r3, não o antigo plano v8.

O objetivo não é simplesmente elevar watts. O teste procura simultaneamente:

- corrupção silenciosa no caminho texture/TMU/ROP;
- erro no acesso a textura cache-resident e VRAM-resident;
- sensibilidade a droop em transições de frame;
- pressão de DRAM/controlador de memória no rail compartilhado;
- instabilidade sob duas filas/contextos independentes e persistentes;
- perda do device ou TDR natural;
- incapacidade de sustentar o clock alvo;
- telemetria contaminada pelo power cap.

### 4.2 Distribuição da carga

O plano tem peso total 100. Portanto, cada peso também é a porcentagem do dwell.

| # | Fase persistida | Workload | Peso | 30 s fronteira | 120 s Apply Standard | 300 s Apply Long |
|---:|---|---|---:|---:|---:|---:|
| 1 | `power-opening` | PowerRender | 1% | 0,3 s | 1,2 s | 3 s |
| 2 | `texture-rop` | TextureRop | 15% | 4,5 s | 18 s | 45 s |
| 3 | `idle-pulse` | IdlePulse | 2% | 0,6 s | 2,4 s | 6 s |
| 4 | `field-concurrency` | CompositeGameLoad | 50% | 15 s | 60 s | 150 s |
| 5 | `texture-rop` | TextureRop | 9% | 2,7 s | 10,8 s | 27 s |
| 6 | `heavy-spike` | HeavySpike | 3% | 0,9 s | 3,6 s | 9 s |
| 7 | `texture-rop` | TextureRop | 7% | 2,1 s | 8,4 s | 21 s |
| 8 | `mixed-game` | MixedGame | 3% | 0,9 s | 3,6 s | 9 s |
| 9 | `frame-cadence` | FrameCadence | 2% | 0,6 s | 2,4 s | 6 s |
| 10 | `compute-burst` | ComputeBurst | 1% | 0,3 s | 1,2 s | 3 s |
| 11 | `boost-edge` | BoostEdge | 4% | 1,2 s | 4,8 s | 12 s |
| 12 | `vram-pressure` | VramPressure | 2% | 0,6 s | 2,4 s | 6 s |
| 13 | `power-closing` | PowerRender | 1% | 0,3 s | 1,2 s | 3 s |

Agrupado por tipo, o teste dedica 50% a Field Concurrency, 31% a TextureRop e 19% às cargas de
abertura, potência, transição, compute e VRAM. A intenção é entrar cedo no detector de textura e
manter metade do dwell no cenário derivado de falha de campo.

### 4.3 Como cada carga funciona

#### TextureRop

- renderiza offscreen em `1536 x 1536`;
- usa uma textura determinística `1024 x 1024`;
- executa shader específico de textura/ROP, quatro instâncias e alpha blending;
- introduz pausas de droop após sequências de 2, 3, 5 e 7 frames, com gaps de 2, 5, 11 e 3 ms;
- compara o framebuffer com a golden stock por redução/checksum GPU;
- divergência bit a bit vira `SilentError`.

#### Field Concurrency / CompositeGameLoad

É o núcleo do Texture Hop atual. Dois lados rodam simultaneamente:

**Contexto primário residente**

- executa uma Texture Stack com três lanes no mesmo encoder/submit:
  - TextureRop cache-resident;
  - TextureStream em fonte `8192 x 8192`, residente em VRAM;
  - PowerRender;
- a lane final gira entre as três imagens, permitindo comparar cada saída com sua golden stock;
- adiciona no **mesmo submit** um scattered gather compute sobre um pool de VRAM;
- tenta até 48 buffers de até 256 MiB cada (aproximadamente 12 GiB máximos), parando de forma
  OOM-guarded no que a placa comporta;
- mantém um fence/poll por frame semelhante a uma fronteira de present para não fabricar TDR por
  fila ilimitada.

**Contexto secundário concorrente**

- espera inicialmente 250 ms;
- cria **um único** `GpuCtx` — device/context/queue independentes — depois que a carga primária inicia;
- mantém esse contexto residente e executa o mesmo canary TextureRop durante o restante da fase;
- usa self-reference: o primeiro checksum vira referência e janelas posteriores precisam coincidir;
- não recria nem destrói devices dentro do loop de carga;
- checksum, device loss e resultado final do secundário participam do verdict;
- falha ao inicializar o secundário, panic do worker ou cobertura insuficiente tornam a fase
  `Inconclusive` no nível ambiental e removem a fase da cobertura; nunca viram instabilidade do bin.

O caminho não possui timeout preventivo de frame para o Texture Stack ou para o canary. O fence
controla a fila, mas corrupção, `DeviceLost` e Windows TDR continuam sendo desfechos naturais.

#### MixedGame

Executa BoostEdge, TextureRop e PowerRender no mesmo encoder/frame/submit. A última lane gira, e sua
imagem é comparada com a golden correspondente.

#### FrameCadence

Executa um frame pesado limitado, espera a conclusão e introduz gaps de 2, 4, 6 e 8 ms. Isso produz
arestas heavy -> idle -> heavy em cadência semelhante a frame/present, procurando droop de VRM.

#### BoostEdge

Modela um lobby/high-FPS leve: cada frame é drenado, medido e seguido por uma bolha CPU menor que
1 ms (`0`, `200`, `500`, `900`, `300`, `800` microssegundos). Além do checksum stock, queda sustentada
de frame time acima de 2x a referência stock pode virar `Unstable`.

A fase recebe 4% do Texture atual. No dwell de 30 s isso gera aproximadamente 1,2 s, ou cerca de 40
amostras NVML a 30 ms, suficiente para o mínimo contratual de 20.

#### PowerRender e HeavySpike

Usam render offscreen pesado com oito instâncias, overdraw e fragment ALU. O frame é deliberadamente
limitado: vários frames curtos mantêm ocupação e potência sem transformar um único submit em um
comando próximo do watchdog de 2 s.

#### ComputeBurst

Executa compute ALU determinístico e registra um known-answer check. Não depende de golden de render.

#### VramPressure

Executa gathers determinísticos sobre buffers grandes com resposta conhecida. Também é self-verified,
sem golden de framebuffer.

### 4.4 Execução no código

`vf_qualifier_plan(target_ms, V8Texture)` converte os pesos em milissegundos. Todas as fases, exceto a
última, recebem divisão inteira proporcional; a última recebe o restante, preservando exatamente o
dwell solicitado.

`run_vf_qualifier_stress_with_phase_pattern_goldens_and_cancel` então:

1. grava o código da fase atual para o sampler NVML;
2. despacha compute conhecido para `ComputeBurst`/`VramPressure`;
3. chama `run_field_concurrency_profile` para `CompositeGameLoad`;
4. chama `run_render_profile` para os demais workloads;
5. acumula frames e relatórios de fase;
6. encerra imediatamente na primeira fase não estável;
7. retorna `Stable` somente se todo o plano terminar.

Em paralelo, `load_and_measure_for` amostra aproximadamente a cada 30 ms:

```text
(clock_mhz, power_w, power_capped, temperature_c, qualifier_phase, thermal_throttled)
```

e agrega p5/p50/p95 de clock, p95/p99/peak de potência, temperatura, residência no alvo e contraste
entre fases.

## 5. Endurance

### 5.1 Identidade e objetivo

- Enum: `VfQualifierPattern::Endurance`.
- Pattern lógico: `F2QualificationPattern::Endurance`.
- Fingerprint atual: `f2q-texhop-v13-r3/endurance-persistent-field-concurrency`.
- Strength exigida: `Fsgl4`.
- Escopo: somente exact-Apply; nunca faz parte da descida da fronteira.

Endurance é um dwell contínuo “worst-realistic”: mais agressivo que uma carga média de jogo, mas sem
ser um power virus arbitrário. O objetivo é acumular calor e repetir transientes elétricos no par que
será publicado.

O mesmo contexto wgpu do dwell percorre toda a sequência. A função não faz reset entre segmentos;
por isso a saturação térmica e o estado do rail acumulam ao longo do teste. O reset acontece apenas
quando a transação confirmada de Endurance termina.

### 5.2 Composição geral

O plano possui peso total 152:

| Grupo | Peso | Fração aproximada |
|---|---:|---:|
| HeavySpike | 41 | 27,0% |
| TextureRop | 38 | 25,0% |
| Field Concurrency | 24 | 15,8% |
| BoostEdge | 20 | 13,2% |
| MixedGame | 10 | 6,6% |
| FrameCadence | 8 | 5,3% |
| IdlePulse | 6 | 3,9% |
| PowerRender de abertura/fechamento | 5 | 3,3% |

Os dez primeiros segmentos formam um tier de rejeição antecipada: TextureRop, Field Concurrency e
choques Heavy/Idle aparecem antes da parte longa de saturação. Um candidato ruim pode cair cedo; um
candidato aprovado sempre completa o plano inteiro.

### 5.3 Sequência completa

Os tempos abaixo reproduzem a mesma divisão inteira usada pelo código; valores são aproximados em
segundos.

| # | Fase | Workload | Peso | Standard 300 s | Long 1.200 s |
|---:|---|---|---:|---:|---:|
| 1 | `power-opening` | PowerRender | 2 | 3,947 | 15,789 |
| 2 | `texture-rop` | TextureRop | 10 | 19,736 | 78,947 |
| 3 | `field-concurrency` | CompositeGameLoad | 12 | 23,684 | 94,736 |
| 4 | `texture-rop` | TextureRop | 8 | 15,789 | 63,157 |
| 5 | `heavy-spike` | HeavySpike | 3 | 5,921 | 23,684 |
| 6 | `idle-pulse` | IdlePulse | 2 | 3,947 | 15,789 |
| 7 | `heavy-spike` | HeavySpike | 3 | 5,921 | 23,684 |
| 8 | `idle-pulse` | IdlePulse | 2 | 3,947 | 15,789 |
| 9 | `heavy-spike` | HeavySpike | 3 | 5,921 | 23,684 |
| 10 | `texture-rop` | TextureRop | 8 | 15,789 | 63,157 |
| 11 | `heavy-spike` | HeavySpike | 14 | 27,631 | 110,526 |
| 12 | `boost-edge` | BoostEdge | 10 | 19,736 | 78,947 |
| 13 | `frame-cadence` | FrameCadence | 8 | 15,789 | 63,157 |
| 14 | `mixed-game` | MixedGame | 10 | 19,736 | 78,947 |
| 15 | `texture-rop` | TextureRop | 6 | 11,842 | 47,368 |
| 16 | `heavy-spike` | HeavySpike | 12 | 23,684 | 94,736 |
| 17 | `heavy-spike` | HeavySpike | 3 | 5,921 | 23,684 |
| 18 | `idle-pulse` | IdlePulse | 2 | 3,947 | 15,789 |
| 19 | `heavy-spike` | HeavySpike | 3 | 5,921 | 23,684 |
| 20 | `field-concurrency` | CompositeGameLoad | 12 | 23,684 | 94,736 |
| 21 | `boost-edge` | BoostEdge | 10 | 19,736 | 78,947 |
| 22 | `texture-rop` | TextureRop | 6 | 11,842 | 47,368 |
| 23 | `power-closing` | PowerRender | 3 | 5,929 | 23,695 |

Standard não executa apenas os dez primeiros segmentos: ele executa **a sequência completa comprimida
para 5 minutos**. Long executa a mesma sequência completa expandida para 20 minutos.

### 5.4 O que Endurance acrescenta a Texture

Texture maximiza o tempo em Field Concurrency e TextureRop para localizar rapidamente o detector
empírico mais sensível. Endurance redistribui a carga para:

- mais HeavySpike sustentado;
- mais alternância heavy/light;
- dois blocos de Field Concurrency em estados térmicos diferentes;
- BoostEdge prolongado para telemetria de cap e regime;
- repetição de TextureRop como detector gracioso entre blocos elétricos;
- potência p99/peak representativa do pior gate, usada depois na seleção e publicação.

O maior p99/peak do conjunto completo Texture + Endurance pode elevar a base de potência do ponto.
Isso impede que um perfil pareça eficiente usando apenas uma medição curta e fria de PowerRender.

## 6. Aprovação, reprovação e inconclusão

O sistema possui três camadas de decisão. “A carga terminou” não é suficiente para publicar.

### 6.1 Camada 1 — resultado físico do workload

| Resultado de baixo nível | Significado | Resultado F2 |
|---|---|---|
| `Stable` | todas as fases terminaram sem divergência ou device loss | segue para cobertura |
| `SilentError` | checksum/golden divergiu sem crash | reprova fisicamente |
| `Unstable` | stall/degradação/comportamento não estável sem device loss | reprova fisicamente |
| `Crash` | wgpu device lost, panic capturado ou TDR observado | `DeviceLost`, falha terminal |

A primeira fase não estável encerra o pattern; as fases posteriores não são executadas.

### 6.2 Camada 2 — cobertura e qualidade da evidência

Mesmo quando a GPU não falha, `qualification_coverage_from_run` só emite `Pass` se todos os gates
abaixo forem satisfeitos.

#### Reprovação (`Fail`)

- o resultado físico do pattern não foi `Stable`;
- reason persistido: `workload_failed`;
- a fase física causadora é registrada em `failure_phase` quando conhecida.

Na prática, `classify_f2_stress_dwell` já transforma crash, silent error ou unstable no outcome F2
correspondente antes de consultar o verdict de cobertura.

#### Inconclusão (`Inconclusive`)

Em ordem de avaliação da cobertura:

| Reason | Condição |
|---|---|
| `phase_not_completed` | nem todas as fases únicas do plano terminaram estáveis |
| `checksum_coverage_low` | soma de checks menor que o número de fases esperadas |
| `telemetry_missing` | nenhuma amostra válida associada a fase |
| `target_residency_low` | menos de 35% das amostras ficaram no clock alvo exato ou acima |
| `boost_edge_telemetry_low` | Texture/Endurance tiveram menos de 20 amostras BoostEdge |
| `boost_edge_power_bound` | p95 de potência em BoostEdge atingiu pelo menos 99% do power limit |
| `phase_contrast_low` | contraste heavy-light ficou abaixo de 3 W |

O gate de autoridade da tensão roda depois do builder de cobertura. No Forge, ele transforma um
workload estável em outcome F2 `Inconclusive` quando há menos de três amostras de tensão, não existe
máximo mensurável ou o máximo ultrapassa o bin selecionado. No Detector Lab, os mesmos casos aparecem
no journal/status como `voltage_telemetry_low`, `voltage_telemetry_missing` ou
`voltage_ceiling_exceeded`.

Se o board power limit numérico não está disponível, `boost_edge_power_bound` usa como fallback mais
de 20% das amostras BoostEdge com flag de cap. Quando o limite numérico existe, a flag bruta é apenas
diagnóstica: prevalece `BoostEdge p95 >= 0,99 * power_limit`.

Para Texture e Endurance, o contraste é:

```text
maior potência média entre fases pesadas
  menos
menor potência média entre IdlePulse e BoostEdge
```

As fases pesadas consideradas incluem Field Concurrency, MixedGame, FrameCadence, TextureRop,
VramPressure, HeavySpike e PowerRender de abertura/fechamento.

Além da cobertura, o classificador geral retorna `Inconclusive` quando:

- Stop/cancel foi solicitado;
- p95 de clock ficou mais de 15 MHz **acima** do alvo, indicando que o ceiling não valeu;
- na descoberta, p5 ficou abaixo do clock alvo exato;
- a telemetria de tensão não comprovou pelo menos três amostras no bin selecionado ou abaixo;
- no exact-Apply há `thermal_throttled` e o p5 caiu abaixo do alvo;
- a cobertura retornou qualquer reason inconclusivo.

Uma flag térmica isolada não invalida o exact-Apply se o p5 ainda sustenta o alvo. Na descoberta de
potência, por outro lado, thermal throttling sempre invalida a calibração.

#### Aprovação (`Pass`)

A cobertura aprova quando:

- o workload terminou `Stable`;
- todas as fases únicas esperadas terminaram;
- há cobertura agregada de checksum suficiente;
- há telemetria;
- a residência no alvo é suficiente;
- há pelo menos 20 amostras BoostEdge;
- BoostEdge não está numericamente power-bound;
- o contraste heavy-light não é menor que 3 W.

Texture possui 11 fases únicas esperadas; Endurance possui 9. O código atual exige
`checksum_count >= phases_expected` de forma agregada. As métricas registram
`checksum_missing` por fase, mas o verdict global não exige explicitamente um checksum em **cada**
fase individual.

### 6.3 Camada 3 — validade transacional e publicação

Uma observação positiva só conta como evidência atual quando:

- kind correto (`Qualification` na fronteira, `ApplyQualification` no par final);
- `qualification_contract_version == 25`;
- outcome validado;
- strength `Fsgl4`;
- pattern correto;
- coverage verdict `Pass`;
- reset stock comprovado;
- boot flag limpo;
- proveniência reproduzível: build/revision, workload fingerprint, backend, adapter, driver,
  checksum method e golden config presentes.

Para um perfil final, ainda são exigidos:

- Texture exato atual;
- Endurance atual, no mesmo `run_id`, target, Apply mV e GPU;
- p95 sustentado mensurável;
- potência completa do gate mensurável;
- confiança/validation count suficientes;
- ausência de recusa de blacklist/ledger;
- respeito ao envelope de potência e ao regime provado.

Somente então `profiles_qualified` pode ficar `true` e o Apply é destravado.

## 7. Política de retry e recuperação

### 7.1 Texture durante a fronteira

- orçamento: até 2 retentativas adicionais para inconclusões transitórias;
- retentativa usa 1,5x o dwell base;
- `boost_edge_power_bound` e `phase_contrast_low` não repetem o mesmo workload no mesmo par, pois são
  formas determinísticas da carga;
- telemetria ausente, BoostEdge insuficiente ou baixa residência podem usar o orçamento;
- um par já esgotado como inconclusivo na mesma run não é martelado novamente;
- inconclusão nunca vira blacklist e não pode ser usada como `last_good` para semear o clock seguinte.

### 7.2 Texture no exact-Apply

- também admite até 2 eventos inconclusivos;
- depois do primeiro inconclusivo, o pattern acumula “dívida” e precisa de **dois passes limpos
  consecutivos**;
- as novas tentativas usam 1,5x o dwell base;
- se a dívida não for quitada dentro do orçamento, o gate termina inconclusivo e a run fica
  incompleta sem inferir reparo elétrico.

### 7.3 Endurance

- executa uma única transação contínua por passagem completa do gate;
- não possui retry local específico;
- `Validated` prossegue;
- `Inconclusive` encerra o gate como inconclusivo;
- falha física reset-clean reprova o par;
- `DeviceLost`, reset/apply/verify/arm failure abortam conforme a segurança transacional.

Se o ledger marcou o par em quarentena sob contrato atual ou mais forte, o par pode exigir duas
passagens completas do gate — cada passagem contém Texture + Endurance. A segunda só roda se a
primeira aprovar.

### 7.4 Depois de uma reprovação no exact-Apply

- power-bound: exclui o par desta seleção sem blacklist e evita Endurance;
- SilentError reset-clean: pode gerar quarentena durável no condemnation ledger;
- outras falhas físicas reset-clean orientam a execução atual sem fabricar uma classe de ledger
  incorreta;
- o reparo vertical tenta o próximo bin V/F viável **acima**, no mesmo clock;
- cada ponto reparado deve repetir o gate completo;
- resultados apenas inconclusivos não autorizam reparo vertical e deixam a execução incompleta.

## 8. Limitações e observações do comportamento atual

### 8.1 IdlePulse não produz o idle longo descrito pelos comentários durante qualificação

`IdlePulse` seleciona o mesmo `RENDER_SHADER`, oito instâncias e o mesmo caminho de render de
`HeavySpike`. O branch de pausa de 100 ms a cada 750 ms exige `!golden_mode`, porém Texture e
Endurance sempre rodam com goldens stock. Assim, no qualifier atual:

- `idle_pulses == true`, mas o branch longo não entra;
- aplica-se o gap genérico de golden: 4 ms a cada 6 frames;
- HeavySpike também recebe esse gap genérico;
- a diferença prática entre os dois segmentos é principalmente o label de fase/telemetria, não um
  verdadeiro release longo do rail.

Portanto, a documentação histórica de “cap-slam HeavySpike <-> IdlePulse” descreve a intenção, mas o
código ativo não cria hoje a separação heavy/light longa sugerida por esse texto. O gate de contraste
pode usar BoostEdge como fase leve, mas não se deve interpretar `IdlePulse` como true-idle no contrato
atual.

### 8.2 Coverage de checksum é agregado

O verdict exige uma contagem total pelo menos igual ao número de fases esperadas. Embora as métricas
individuais registrem `checksum_missing`, a aprovação não faz hoje `all(phases.checksum_count > 0)`.
Isso é relevante ao interpretar “cobertura completa”: fases precisam terminar estáveis, mas a regra
de checksum é agregada.

### 8.3 O teste reduz risco; não prova estabilidade infinita

Texture e Endurance são failure-seeking e fecham gaps reais observados, mas continuam sendo amostras
finitas. O sistema complementa o gate com margem de Apply, Safe Loop, Sentinel, recuperação de startup,
condemnation ledger e feedback de falha em campo.

## 9. Mapa do código

| Responsabilidade | Arquivo / símbolo |
|---|---|
| enums, fingerprints e pesos | `crates/gpu-stress/src/lib.rs` — `VfQualifierPattern`, `VfWorkload`, `vf_qualifier_workload_fingerprint`, `vf_qualifier_plan` |
| execução sequencial dos segmentos | `crates/gpu-stress/src/lib.rs` — `run_vf_qualifier_stress_with_phase_pattern_goldens_and_cancel` |
| render/checksum e cargas | `crates/gpu-stress/src/lib.rs` — `run_render_profile` |
| concorrência entre contextos | `crates/gpu-stress/src/lib.rs` — `run_field_canary_worker`, `run_field_concurrency_profile` |
| captura stock e preflight | `crates/service/src/gpu_power_sweep.rs` — `capture_fsgl3_render_goldens`, `validate_v24_texture_hop_stock` |
| sampler e cobertura | `crates/service/src/gpu_power_sweep.rs` — `load_and_measure_for`, `qualification_coverage_from_run` |
| classificação final do dwell | `crates/service/src/gpu_undervolt.rs` — `classify_f2_stress_dwell` |
| motor arm/apply/verify/reset | `crates/service/src/gpu_undervolt.rs` — `run_confirmed_f2_step` |
| Texture na fronteira | `crates/service/src/gpu_undervolt.rs` — `qualify_active_anchored_candidate`, `run_confirmed_f2_clock_discovery` |
| Texture + Endurance exact-Apply | `crates/service/src/gpu_undervolt.rs` — `gate_anchored_candidate_fsgl3`, `run_confirmed_f2_apply_qualification` |
| validade de evidência/publicação | `crates/core/src/f2_observation.rs` — `is_current_qualification_pass`, `is_current_apply_qualification_pass`, `point_has_current_endurance_qualification` |
| síntese, reparo e publicação | `crates/service/src/gpu_power_sweep.rs` — fluxo `apply-qualify` em `measure_multiclock_undervolt_forge` |

## 10. Fluxo resumido de decisão

```text
START FORGE
  |
  +-- preheat + curva V/F
  |
  +-- goldens stock + Texture Hop completo 60 s
  |     \-- falhou: aborta antes de candidatos
  |
  +-- para cada clock/bin elegível
  |     +-- PowerRender discovery 10 s
  |     +-- Texture 30 s (Standard) ou 60 s (Long)
  |           +-- Pass: desce um bin
  |           +-- Fail físico: fecha/repara a fronteira do clock
  |           \-- Inconclusive: preserva ponto raso qualificado e pula o clock
  |
  +-- sintetiza os três perfis usando fronteira publicável
  |
  +-- para cada par Apply único
  |     +-- Texture 120/300 s
  |     |     +-- potência > 94% do cap: exclui sem Endurance/blacklist
  |     |     +-- Fail: rejeita/repara
  |     |     \-- Inconclusive: run incompleta
  |     |
  |     +-- Endurance 300/1200 s
  |     |     +-- Pass: reconcilia telemetria
  |     |     +-- Fail: rejeita/repara
  |     |     \-- Inconclusive: run incompleta
  |     |
  |     \-- reset stock + proveniência + p95/p99 + regime
  |
  +-- todos os perfis completos: profiles_qualified = true
  \-- qualquer gate ausente: Apply permanece bloqueado
```

## 11. Definições curtas

- **Boundary/frontier point**: menor tensão de descoberta comprovada para um clock alvo.
- **Apply pair**: clock alvo e bin V/F real depois da margem de Apply.
- **Golden**: checksum determinístico capturado em stock para comparar a saída do candidato.
- **Self-reference**: primeiro checksum do próprio run vira referência para detectar divergência
  posterior sem depender de golden stock.
- **SilentError**: saída incorreta detectada por checksum sem device loss.
- **Field Concurrency**: Texture Stack residente em um contexto enquanto um segundo contexto
  independente permanece ativo com o canary TextureRop.
- **Power-bound**: carga encostou no limite energético e não prova adequadamente o bin elétrico.
- **Inconclusive**: evidência insuficiente ou contaminada; não é pass nem falha física.
- **Reset-clean**: GPU voltou ao stock e o boot flag foi limpo com sucesso.
- **Publicável**: evidência current-contract, reset-clean, reproduzível, completa e aceita por todos os
  gates de perfil.
