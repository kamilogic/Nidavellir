# Nidavellir Phase 3 Visual Direction

Status: exploratory mockup direction, not implementation.

This document defines the Phase 3 visual identity direction for the existing
Forge Home structure. It does not introduce new backend assumptions, new data,
or new product flow.

## Design Intent

Nidavellir should feel like a precision forge for silicon:

- premium desktop application
- forged metal, not fantasy metal
- precision engineering, not gamer tuning
- calm trust, not alarm
- master craftsmanship, not decoration

The forge theme should appear through material, hierarchy, and terminology. Rune
influence should be limited to subtle engraved signatures or small symbolic
markers. Never use runes as wallpaper, ornament bands, or fantasy UI trim.

## Screen Hierarchy

Forge Home should scan in this order:

1. GPU identity and forge state
2. current profile and safety state
3. recommended next action
4. profile comparison
5. forge knowledge
6. advanced diagnostics

The page should not feel like a set of tools. It should feel like a product
state machine around one GPU.

Navigation direction should use GPU-first product language:

```text
Forge / Profiles / Knowledge / Diagnostics / Settings
```

For the current implementation pass, do not create new top-level tabs or new
product flows. If navigation is shown in static mockups, use the GPU-first
direction above rather than legacy hardware/reporting labels.

## Hero State

The hero should make Forge State the dominant product milestone.

Wireframe:

```text
+-------------------------------------------------------------+
| GPU Forge Home                                      FORGED   |
| GeForce RTX 3060 Ti                                 Protected|
|                                                             |
| This GPU has completed the forging process.                 |
|                                                             |
| Current Profile       Forge State          Safety           |
| Brokkr's Best         FORGED               Protected        |
| From applied profile  Ready for daily use   Safe Loop ready |
+-------------------------------------------------------------+
```

The state should carry more visual weight than the raw clock/voltage data.

## Forge State Progression

Use a restrained progression, not a game rarity scale.

| State | Meaning | Visual treatment |
| --- | --- | --- |
| Raw | GPU detected; no meaningful knowledge exists yet | neutral graphite, low emphasis |
| Forging | measurement is active | steel-blue active line or pulse |
| Tempered | stable knowledge exists | warm amber, cautious confidence |
| Refined | profiles are credible | bronze, stronger confidence |
| Forged | process completed | gold-bronze, highest milestone emphasis |

The state strip should show progression without implying XP, levels, or fantasy
rarity. Use small engineering ticks or etched markers.

## Safety States

Safe Loop should feel integrated into the product. Safety states are product
states, not warnings unless action is needed.

| State | Meaning | Visual treatment |
| --- | --- | --- |
| Protected | normal safe condition | calm green/steel badge |
| Recovery Ready | risky validation is armed | active steel-blue/amber badge |
| Recovered Successfully | recovery happened and system is safe | calm green with recovery note |
| Needs Attention | user should review before tuning | muted red, clear but not panic UI |

## Profile Identity

All profile cards share structure:

1. profile name
2. philosophy line
3. expected result
4. technical details
5. apply/applied state

Technical details stay visible but secondary.

### Godforge

Performance first.

Visual identity:

- darker steel surface
- warm edge highlight
- strong but controlled contrast
- communicates strength, not risk-taking

Expected Result:

- highest sustainable performance
- higher power consumption
- higher thermal output

### Brokkr's Best

Recommended signature profile.

Visual identity:

- bronze/gold accent
- strongest recommendation badge
- active/applied state feels crafted and complete
- should be the visual center of the profile row

Expected Result:

- strong gaming performance
- lower power draw
- lower temperatures
- lower fan noise

### Deep Calm

Efficiency first.

Visual identity:

- cool steel accent
- calmer contrast
- quiet, precise, elegant

Expected Result:

- maximum efficiency
- lowest power consumption
- cooler and quieter operation

## Design System Proposal

### Tokens

```css
--forge-void: #05070b;
--forge-iron: #0b1018;
--forge-graphite: #111a24;
--forge-steel: #253344;
--forge-line: rgba(170, 188, 205, 0.16);

--forge-bronze: #b8874c;
--forge-gold: #d7b46a;
--forge-ember: #c97945;
--forge-blue: #79a9c7;
--forge-green: #8fbf8f;
--forge-red: #c26b72;

--text-primary: #e7ecf2;
--text-secondary: #a8b3c1;
--text-muted: #687789;
```

### Components

Panel:

- dark graphite surface
- 1px metal line border
- 8-12px radius
- consistent title, support copy, content rhythm

StatusBadge:

- compact capsule
- state-specific accent
- label first, no icons required
- used for Forge State and Safety State

ProfileCard:

- shared structure
- variant accent by profile
- active card style
- `Applied ✓` disabled state

ActionButton:

- primary, secondary, danger, applied, disabled
- action labels must reflect state

DiagnosticsDisclosure:

- collapsed by default
- clear summary text
- contains raw logs, sweep tables, V/F curve, benchmark internals
- should feel like an engineer drawer

## Mockup Artifact

See `docs/ui/phase3-forge-home-mockup.html`.

This file is a standalone static mockup. It must not hardcode MHz/mV/W sample
values. Technical value rows should represent existing UI payloads or honest
empty states such as "Not forged yet" / "Not learned yet." It is not wired into
the Svelte app and should not be treated as final styling.

## Rationale

This direction makes the existing Phase 2 information architecture feel more
like Nidavellir:

- the user immediately sees where their GPU is in the forge journey
- Brokkr's Best becomes the signature default without hiding alternatives
- safety is visible as a product state
- technical diagnostics remain available but secondary
- forge theming is expressed as material, precision, and state progression
