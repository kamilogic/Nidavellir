<script>
  import { Activity, CircleCheck, Database, ShieldCheck, TriangleAlert } from "@lucide/svelte";

  let { label, variant = "neutral", compact = false, symbol = null } = $props();
  const iconSize = $derived(compact ? 12 : 14);
</script>

<span class={`status-badge ${variant}`} class:compact>
  {#if symbol === "shield"}
    <ShieldCheck size={iconSize} strokeWidth={1.9} />
  {:else if symbol === "check"}
    <CircleCheck size={iconSize} strokeWidth={1.9} />
  {:else if symbol === "attention"}
    <TriangleAlert size={iconSize} strokeWidth={1.9} />
  {:else if symbol === "activity"}
    <Activity size={iconSize} strokeWidth={1.9} />
  {:else if symbol === "knowledge"}
    <Database size={iconSize} strokeWidth={1.9} />
  {:else}
    <span class="badge-dot" aria-hidden="true"></span>
  {/if}
  <span>{label}</span>
</span>

<style>
  .status-badge {
    --badge-fg: var(--forge-muted);
    --badge-border: var(--forge-line);
    --badge-bg: rgba(255, 255, 255, 0.03);
    display: inline-flex;
    align-items: center;
    gap: 0.38rem;
    width: fit-content;
    border: 1px solid var(--badge-border);
    border-radius: 999px;
    background: var(--badge-bg);
    color: var(--badge-fg);
    padding: 0.38rem 0.7rem;
    font-size: 0.7rem;
    font-weight: 800;
    letter-spacing: 0.08em;
    line-height: 1;
    text-transform: uppercase;
    white-space: nowrap;
  }
  .badge-dot {
    width: 0.4rem;
    height: 0.4rem;
    border-radius: 999px;
    background: currentColor;
    box-shadow: 0 0 12px var(--badge-glow, rgba(255, 255, 255, 0.08));
    flex-shrink: 0;
  }
  .compact {
    padding: 0.28rem 0.55rem;
    font-size: 0.62rem;
    letter-spacing: 0.07em;
  }
  .raw {
    --badge-fg: var(--forge-steel);
    --badge-border: rgba(145, 158, 174, 0.28);
    --badge-bg: rgba(145, 158, 174, 0.08);
  }
  .forging {
    --badge-fg: var(--forge-blue);
    --badge-border: rgba(126, 173, 190, 0.42);
    --badge-bg: rgba(126, 173, 190, 0.1);
    --badge-glow: rgba(126, 173, 190, 0.28);
  }
  .tempered {
    --badge-fg: var(--forge-copper);
    --badge-border: rgba(185, 117, 75, 0.42);
    --badge-bg: rgba(185, 117, 75, 0.1);
    --badge-glow: rgba(185, 117, 75, 0.24);
  }
  .refined {
    --badge-fg: var(--forge-green);
    --badge-border: rgba(157, 191, 145, 0.42);
    --badge-bg: rgba(157, 191, 145, 0.1);
    --badge-glow: rgba(157, 191, 145, 0.24);
  }
  .forged,
  .recommended {
    --badge-fg: var(--forge-gold);
    --badge-border: rgba(214, 168, 93, 0.52);
    --badge-bg: rgba(214, 168, 93, 0.11);
    --badge-glow: rgba(214, 168, 93, 0.3);
  }
  .protected,
  .active {
    --badge-fg: var(--forge-green);
    --badge-border: rgba(157, 191, 145, 0.46);
    --badge-bg: rgba(157, 191, 145, 0.1);
    --badge-glow: rgba(157, 191, 145, 0.26);
  }
  .recovery {
    --badge-fg: var(--forge-blue);
    --badge-border: rgba(126, 173, 190, 0.42);
    --badge-bg: rgba(126, 173, 190, 0.1);
  }
  .recovered {
    --badge-fg: var(--forge-gold);
    --badge-border: rgba(214, 168, 93, 0.46);
    --badge-bg: rgba(214, 168, 93, 0.1);
  }
  .attention {
    --badge-fg: var(--forge-red);
    --badge-border: rgba(197, 111, 115, 0.48);
    --badge-bg: rgba(197, 111, 115, 0.1);
    --badge-glow: rgba(197, 111, 115, 0.26);
  }
</style>
