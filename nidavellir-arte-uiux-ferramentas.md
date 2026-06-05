# Nidavellir – Guia de Ferramentas para Arte, UI e UX

## Objetivo

Este guia define **onde buscar cada tipo de referência, asset ou inspiração** para manter coerência visual e bons resultados na arte, UI e UX do Nidavellir.

A direção visual central do produto é:

```text
Semiconductor fabrication facility
+
Forged metal craftsmanship
+
Runic geometric motifs
+
Premium desktop application
```

O Nidavellir deve parecer uma **forja de precisão para silício**, não uma interface de fantasia.

---

## Princípio de coerência

Use esta frase como filtro principal:

```text
Nidavellir is a premium precision forge for silicon, not a fantasy forge.
```

A estética correta é:

```text
Dark graphite desktop software
+
Subtle forged bronze
+
Steel blue diagnostics
+
Controlled green safety
+
Geometric runic restraint
+
Semiconductor lab precision
```

A estética errada é:

```text
Viking game UI
RGB overclocking app
Cyberpunk neon dashboard
Fantasy MMO forge
Generic hardware monitor
```

---

# 1. Mapa geral das ferramentas

| Necessidade | Melhor ferramenta | Uso no Nidavellir |
|---|---|---|
| Direção visual geral | Pinterest / Dribbble | Moodboard, atmosfera, materiais, referências visuais |
| Hierarquia de UI/UX | Mobbin | Layouts, dashboards, cards, organização de informação |
| Design system real | Figma | Componentes, tokens, variações, handoff |
| Logo e símbolos | Ideogram | Logos, marcas, símbolos com texto melhor controlado |
| Artes conceituais | ChatGPT Image Generator / Leonardo AI | Perfis, splash, fundos, identidade visual |
| Ícones de interface | Lucide Icons | Ícones principais do app |
| Ícones alternativos | Heroicons | Alternativa mais simples e neutra |
| SVGs para adaptar | SVG Repo | Bases vetoriais específicas |
| Texturas | Poly Haven / Textures.com / ambientCG | Metal, graphite, steel, brushed surface |

---

# 2. Pinterest – direção visual e moodboard

Use o Pinterest para **clima visual**, não para UI final.

## Buscar

```text
industrial forge
precision engineering
semiconductor fabrication
dark desktop application
scientific instrumentation ui
brushed metal texture
graphite material design
luxury dashboard ui
industrial control panel
precision instrument design
```

## O que salvar

Salve imagens que tenham:

- Grafite escuro
- Metal escovado
- Instrumentação técnica
- Painéis industriais refinados
- Luzes discretas
- Sensação de laboratório ou fábrica de precisão
- Superfícies premium

## O que evitar

Não salve:

- Guerreiros vikings
- Machados
- Capacetes
- Runas brilhantes demais
- Interface cyberpunk neon
- Setup gamer RGB
- Fantasia medieval

## Como usar no Nidavellir

Pinterest deve responder:

```text
Como o Nidavellir deve parecer emocionalmente?
```

Não deve responder:

```text
Como exatamente a tela deve ser organizada?
```

---

# 3. Mobbin – hierarquia de UI/UX

Use o Mobbin para estudar como produtos sérios organizam informação.

## Buscar

```text
dashboard
analytics
fintech
security
professional software
developer tools
monitoring
control center
settings
device management
```

## O que observar

Procure padrões de:

- Hero section
- Cards de recomendação
- Estados de sistema
- Badges
- Painéis de diagnóstico
- Empty states
- Alertas não agressivos
- Organização de sidebar
- Layouts com bastante informação sem parecer poluído

## O que copiar conceitualmente

Copie apenas a lógica visual:

```text
Como eles destacam uma recomendação?
Como eles diferenciam estado normal, alerta e sucesso?
Como organizam dados densos?
Como criam leitura rápida?
```

## O que não copiar

Não copie estética diretamente.

O Nidavellir não deve parecer fintech, SaaS genérico ou app mobile.

Mobbin serve para **estrutura**, não para identidade.

---

# 4. Dribbble – componentes e inspiração visual refinada

Use o Dribbble para referências de componentes bonitos, mas com cuidado.

## Buscar

```text
dark dashboard
industrial ui
control center
cyber security dashboard
desktop software
hardware dashboard
system monitor ui
premium dashboard
engineering dashboard
```

## O que aproveitar

- Cards premium
- Uso de bordas
- Glow discreto
- Layouts escuros
- Microdetalhes visuais
- Badges
- Painéis técnicos
- Gráficos estilizados

## Risco

Dribbble pode puxar o Nidavellir para:

```text
visual bonito demais
pouca usabilidade
neon demais
cyberpunk demais
dashboard genérico
```

## Regra

Use Dribbble para **acabamento**, não para definir UX.

---

# 5. Figma – onde consolidar tudo

Figma deve ser o local onde a identidade vira design system.

## Usar Figma para

- Paleta de cores
- Tokens visuais
- Tipografia
- Espaçamentos
- Componentes
- Variações de estados
- Mockups da Forge Home
- Comparação entre versões
- Handoff para implementação

## Estrutura recomendada no Figma

```text
01 – Moodboard
02 – Visual Direction
03 – Color Tokens
04 – Typography
05 – Components
06 – Forge Home Mockups
07 – Profile Cards
08 – Safe Loop States
09 – Diagnostics Panels
10 – Implementation Notes
```

## Componentes a criar

```text
HeroStatus
ForgeStateStepper
ProfileCard
StatusBadge
SafetyIndicator
Panel
PrimaryButton
SecondaryButton
DiagnosticsSection
```

## O que não fazer no Figma

Não transformar o Figma em laboratório infinito.

A função dele é consolidar decisões, não ficar eternamente testando estilos.

---

# 6. ChatGPT Image Generator – arte conceitual rápida

Use para gerar rapidamente variações visuais coerentes com a lore.

## Melhor uso

- Logo conceitual
- App icon
- Splash screen
- Backgrounds abstratos
- Arte dos perfis
- Safe Loop iconography
- Texturas estilizadas
- Referência visual para Hero

## Prompts úteis

### Direção geral

```text
A premium dark desktop application visual identity for a GPU tuning app called Nidavellir.
Theme: forged silicon, semiconductor fabrication facility, precision engineering, brushed graphite metal, subtle bronze accents, cool steel blue diagnostics.
No vikings, no warriors, no axes, no fantasy UI, no RGB gamer style.
Clean professional product design, restrained, technical, premium.
```

### Hero visual

```text
A hero status panel for a premium desktop GPU tuning application.
Dark graphite background, forged metal surfaces, subtle bronze highlight, steel blue diagnostic accents, clear state progression from Raw to Forged.
Precision engineering aesthetic, semiconductor lab meets refined forge.
No fantasy, no neon, no gaming RGB.
```

### Profile cards

```text
Three premium desktop UI profile cards for a GPU tuning app:
Godforge, Brokkr's Best, Deep Calm.
Godforge feels powerful and performance-first.
Brokkr's Best feels recommended, balanced, signature craftsmanship.
Deep Calm feels quiet, efficient, elegant and minimalist.
Dark graphite UI, bronze and steel accents, precision engineering style.
No fantasy, no vikings, no RGB.
```

## Regra de uso

Use o gerador para descobrir **linguagem visual**, não para gerar a UI final diretamente.

Depois, leve as melhores ideias para Figma ou para implementação manual.

---

# 7. Ideogram – logo, marca e símbolos

Ideogram é especialmente útil para logos, símbolos e imagens que dependem de melhor controle de texto.

## Usar para

- Logo Nidavellir
- Wordmark
- App icon
- Símbolo de bigorna + chip
- Selo Brokkr's Best
- Marca Safe Loop
- Emblemas simples dos perfis

## Buscar/gerar

```text
minimal premium logo for Nidavellir
forged silicon logo
anvil and microchip symbol
precision forge app icon
dark metal desktop software brand
geometric runic silicon mark
```

## Prompts úteis

```text
Minimal premium app icon for a GPU tuning desktop application called Nidavellir.
Symbol combines a microchip and an anvil in a simple geometric form.
Dark graphite, forged bronze accent, precision engineering, clean vector logo.
No vikings, no axes, no fantasy, no RGB.
```

```text
Premium wordmark logo for Nidavellir, a desktop GPU tuning application.
Visual language: forged silicon, precision engineering, dark graphite metal, subtle bronze.
Clean modern typography, restrained geometric details, no fantasy, no medieval style.
```

## Regra

Ideogram deve ser usado para **marca**, não para telas inteiras.

---

# 8. Leonardo AI – concept art e assets mais ricos

Use Leonardo AI para explorar assets visuais mais elaborados.

## Usar para

- Backgrounds
- Splash screens
- Concept art dos perfis
- Ilustrações abstratas
- Materiais metálicos
- Ambientes “silicon forge”
- Arte promocional

## Melhor para Nidavellir

```text
Semiconductor forge environment
Dark silicon laboratory
Precision metal workshop
Premium hardware control room
Forged microchip close-up
```

## Prompt útil

```text
A premium concept art background for a desktop GPU tuning application.
A restrained semiconductor fabrication facility fused with a precision metal forge.
Dark graphite surfaces, brushed steel, subtle bronze thermal glow, clean engineering atmosphere.
No people, no vikings, no weapons, no fantasy, no cyberpunk neon.
```

## Regra

Use Leonardo para assets visuais ricos, mas depois simplifique.

O app não deve parecer concept art colada em cima da UI.

---

# 9. Lucide Icons – biblioteca principal de ícones

Use Lucide como biblioteca principal de ícones.

## Por que combina com Nidavellir

- Visual limpo
- Ícones de linha
- Boa leitura em UI escura
- Fácil de adaptar por stroke, tamanho e cor
- Combina com React/Svelte/Tauri

## Ícones úteis

### Forge State

```text
Flame
Hammer
Gauge
Activity
Cpu
Zap
Shield
CheckCircle
CircleDot
```

### Safe Loop

```text
ShieldCheck
RotateCcw
RefreshCcw
AlertTriangle
CheckCircle
LifeBuoy
Lock
```

### Diagnostics

```text
Thermometer
Gauge
Activity
Cpu
HardDrive
Radar
LineChart
Sliders
```

## Regra visual

Use Lucide em:

```text
stroke 1.75 ou 2
ícones pequenos e médios
sem preenchimento pesado
cores semânticas consistentes
```

---

# 10. Heroicons – alternativa mais neutra

Heroicons pode ser usado como alternativa se Lucide parecer técnico demais em alguma área.

## Usar quando

- Lucide parecer técnico demais
- Precisar de ícones mais simples
- Quiser mais neutralidade
- Interface precisar ficar menos instrumental

## Melhor uso

- Sidebar
- Navegação
- Ações simples
- Settings
- Botões secundários

## Regra

Não misture Lucide e Heroicons sem critério.

Escolha uma biblioteca como principal.

Recomendação: **Lucide principal, Heroicons somente se necessário**.

---

# 11. SVG Repo – SVGs específicos para adaptar

Use SVG Repo quando precisar de uma base vetorial que não existe bem em Lucide ou Heroicons.

## Usar para

- Bigorna
- Chip estilizado
- Circuitos
- Símbolos geométricos
- Marcas abstratas
- Shapes decorativos

## Regra

Nunca usar SVG baixado diretamente sem adaptação.

Padronize:

```text
stroke
corner radius
espessura visual
nível de detalhe
cor
proporção
```

O objetivo é parecer parte do design system, não um ícone aleatório colado no app.

---

# 12. Poly Haven – texturas de alta qualidade

Use Poly Haven para texturas e materiais de alta qualidade.

## Buscar

```text
brushed metal
steel
rough metal
dark metal
black metal
industrial
concrete
carbon
```

## Usar para

- Backgrounds muito sutis
- Overlays de metal
- Splash screen
- Materiais para arte conceitual
- Textura de cards premium

## Regra

Textura deve ser quase invisível.

Use como:

```text
opacity 2%–8%
overlay
noise sutil
background depth
```

Não transforme o app em uma superfície realista de metal.

---

# 13. ambientCG – alternativa para materiais PBR

ambientCG é útil para materiais PBR, referências e texturas.

## Buscar

```text
metal
brushed
steel
dark metal
floor industrial
carbon
scratched metal
```

## Melhor uso

- Referência de materiais
- Texturas para splash
- Render conceitual
- Backgrounds de baixa opacidade

## Regra

Assim como Poly Haven: usar com extrema sutileza.

---

# 14. Textures.com – texturas industriais

Use Textures.com para procurar materiais específicos.

## Buscar

```text
brushed metal
graphite
steel
industrial panel
scratched metal
carbon fiber
dark concrete
```

## Melhor uso

- Referências de superfície
- Overlay de painel
- Inspiração de material

## Atenção

Verifique licença antes de usar em distribuição final.

---

# 15. Ordem prática recomendada

## Etapa 1 – Moodboard

Ferramentas:

```text
Pinterest
Dribbble
Poly Haven
ambientCG
```

Objetivo:

```text
Definir aparência geral: forged steel + silicon lab + runic geometry.
```

Resultado esperado:

```text
20 a 40 referências visuais
separadas em materiais, UI, iluminação, símbolos e atmosferas
```

---

## Etapa 2 – UX e hierarquia

Ferramenta:

```text
Mobbin
```

Objetivo:

```text
Entender como organizar informação visualmente sem redesenhar o fluxo.
```

Buscar exemplos de:

```text
dashboards
security apps
analytics
developer tools
monitoring apps
```

Resultado esperado:

```text
Referências de Hero section
cards de recomendação
badges
status panels
diagnostics layout
```

---

## Etapa 3 – Exploração de marca

Ferramentas:

```text
Ideogram
ChatGPT Image Generator
```

Objetivo:

```text
Encontrar símbolos e identidade da marca Nidavellir.
```

Gerar:

```text
logo
app icon
anvil + chip
Brokkr's Best seal
Safe Loop symbol
```

Resultado esperado:

```text
3 direções de logo
1 direção escolhida
1 sistema básico de símbolos
```

---

## Etapa 4 – Artes dos perfis

Ferramentas:

```text
ChatGPT Image Generator
Leonardo AI
```

Objetivo:

```text
Criar distinção visual entre Godforge, Brokkr's Best e Deep Calm.
```

Gerar:

```text
Godforge: performance, potência, bronze intenso
Brokkr's Best: recomendado, assinatura, equilíbrio
Deep Calm: silêncio, eficiência, minimalismo
```

Resultado esperado:

```text
1 imagem-referência por perfil
1 paleta de acento por perfil
1 ícone ou símbolo por perfil
```

---

## Etapa 5 – Design system

Ferramenta:

```text
Figma
```

Objetivo:

```text
Transformar direção visual em componentes reutilizáveis.
```

Criar:

```text
Color tokens
Typography scale
Spacing tokens
ProfileCard
HeroStatus
ForgeStateStepper
SafeLoopBadge
DiagnosticsPanel
Button variants
StatusBadge variants
```

Resultado esperado:

```text
Mockup visual completo da Forge Home
componentes principais prontos para implementação
```

---

## Etapa 6 – Implementação visual

Ferramentas:

```text
Codex
Claude Code
Cursor
```

Objetivo:

```text
Aplicar apenas a camada visual aprovada.
```

Instrução para o agente:

```text
Implement only the approved Phase 3 visual identity.
Do not change backend assumptions.
Do not invent new data.
Do not redesign application flow.
Do not add new product concepts.
Only update styling, visual hierarchy, reusable components, and visual states.
```

---

# 16. O que buscar para cada parte do Nidavellir

## Forge Home

Buscar em:

```text
Mobbin
Dribbble
Figma Community
```

Termos:

```text
dark analytics dashboard
security dashboard
developer tool dashboard
system monitoring dashboard
desktop control center
```

Objetivo:

```text
Hierarquia, scanability, organização de cards.
```

---

## Forge State

Buscar em:

```text
Mobbin
Dribbble
ChatGPT Image Generator
```

Termos:

```text
status progression ui
system state dashboard
industrial process indicator
stepper status ui
control panel status
```

Objetivo:

```text
Criar progressão Raw → Forged.
```

---

## Profile Cards

Buscar em:

```text
Mobbin
Dribbble
Figma
```

Termos:

```text
pricing cards dark UI
profile cards dashboard
recommendation cards
product cards dark dashboard
settings cards
```

Objetivo:

```text
Diferenciar Godforge, Brokkr's Best e Deep Calm.
```

---

## Safe Loop

Buscar em:

```text
Mobbin
Lucide
Heroicons
Dribbble
```

Termos:

```text
security status UI
system protection dashboard
recovery status
backup status UI
device safety dashboard
```

Objetivo:

```text
Fazer segurança parecer estado do produto, não alerta solto.
```

---

## Diagnostics

Buscar em:

```text
Mobbin
Dribbble
Lucide
```

Termos:

```text
system diagnostics UI
monitoring dashboard
developer tools diagnostics
hardware monitoring UI
technical metrics panel
```

Objetivo:

```text
Mostrar dados técnicos com clareza e sem poluição.
```

---

## Logo e App Icon

Buscar/gerar em:

```text
Ideogram
ChatGPT Image Generator
SVG Repo
```

Termos:

```text
microchip anvil logo
forged silicon symbol
precision forge icon
dark premium app icon
geometric runic logo
```

Objetivo:

```text
Criar marca simples, reconhecível e aplicável em tamanho pequeno.
```

---

# 17. Critério de aprovação de referências e assets

Antes de aceitar qualquer referência, pergunte:

## 1. Parece software premium?

Se parecer jogo, descartar.

## 2. Parece engenharia de precisão?

Se parecer fantasia medieval, descartar.

## 3. Parece produto de hardware confiável?

Se parecer benchmark gamer genérico, descartar.

## 4. Tem relação com forged silicon?

Se for só “metal bonito”, usar com cuidado.

## 5. Ajuda o usuário a entender estado e recomendação?

Se for apenas decorativo, reduzir ou remover.

---

# 18. Recomendação final de stack

Para o Nidavellir, a ordem recomendada é:

```text
Pinterest
→ mood e materiais

Mobbin
→ hierarquia e UX real

Dribbble
→ acabamento visual e componentes premium

Ideogram
→ logo, app icon e símbolos

ChatGPT Image Generator
→ exploração rápida da identidade visual

Leonardo AI
→ concept art e assets mais ricos

Lucide
→ ícones oficiais da UI

Figma
→ design system final

Codex / Claude Code / Cursor
→ implementação visual depois da aprovação
```

---

# 19. Resumo executivo

A forma mais segura de manter coerência visual é separar cada ferramenta por função:

- **Pinterest** para atmosfera.
- **Mobbin** para hierarquia.
- **Dribbble** para acabamento visual.
- **Ideogram** para marca.
- **ChatGPT Image Generator** para exploração rápida.
- **Leonardo AI** para concept art.
- **Lucide** para ícones finais.
- **Figma** para consolidar design system.
- **Codex / Claude / Cursor** apenas depois da direção aprovada.

O Nidavellir deve comunicar:

```text
forged silicon
precision engineering
premium desktop control
safety-first tuning
restrained runic geometry
```

E nunca deve comunicar:

```text
MMO fantasy
RGB gamer
viking cosplay
cyberpunk neon
hardware monitor genérico
```
