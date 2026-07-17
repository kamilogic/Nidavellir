<script>
  import { classifyForgeLogLine } from "../../logTone.js";

  let { title = "nidavellir", status = "idle", live = false, lines = [], runningText = null } = $props();

  function autoscroll(node, _dep) {
    const toBottom = () => {
      node.scrollTop = node.scrollHeight;
    };
    toBottom();
    return { update: toBottom };
  }

  const dep = $derived((lines?.length ?? 0) + (runningText ? 1 : 0));
</script>

<div class="terminal">
  <div class="term-head">
    <span class="dots"><i></i><i></i><i></i></span>
    <span class="term-title">{title}</span>
    <span class="term-status" class:live>{status}</span>
  </div>
  <div class="term-body" use:autoscroll={dep}>
    {#each lines as line, i}
      <div class={`tline ${classifyForgeLogLine(line)}`}>
        <span class="gutter">{(i + 1).toString().padStart(2, "0")}</span>
        <span class="tlead">{line}</span>
      </div>
    {/each}
    {#if runningText}
      <div class="tline running">
        <span class="gutter">&gt;</span>
        <span class="cursor"></span>
        <span class="tlead">{runningText}</span>
      </div>
    {/if}
  </div>
</div>

<style>
  .terminal {
    font-family: "Cascadia Code", "Consolas", ui-monospace, monospace;
    font-size: 0.8rem;
    background: rgba(5, 7, 11, 0.92);
    border: 1px solid var(--forge-line);
    border-radius: 10px;
    overflow: hidden;
    box-shadow: inset 0 0 0 1px rgba(214, 168, 93, 0.04), 0 8px 24px rgba(0, 0, 0, 0.35);
  }
  .term-head {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    padding: 0.4rem 0.7rem;
    background: rgba(214, 168, 93, 0.055);
    border-bottom: 1px solid var(--forge-line);
  }
  .dots {
    display: inline-flex;
    gap: 0.32rem;
  }
  .dots i {
    width: 0.62rem;
    height: 0.62rem;
    border-radius: 50%;
    background: var(--nord-dim);
    opacity: 0.6;
  }
  .dots i:nth-child(1) {
    background: var(--nord-danger);
  }
  .dots i:nth-child(2) {
    background: var(--nord-ember-bright);
  }
  .dots i:nth-child(3) {
    background: var(--nord-aurora);
  }
  .term-title {
    color: var(--nord-mist);
    font-size: 0.74rem;
    letter-spacing: 0.04em;
  }
  .term-status {
    margin-left: auto;
    font-size: 0.68rem;
    text-transform: lowercase;
    color: var(--nord-dim);
    padding: 0.08rem 0.5rem;
    border-radius: 999px;
    border: 1px solid var(--border);
  }
  .term-status.live {
    color: var(--nord-ember-bright);
    border-color: rgba(235, 203, 139, 0.4);
    background: rgba(235, 203, 139, 0.08);
  }
  .term-body {
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
    padding: 0.55rem 0.7rem;
    max-height: 340px;
    overflow-y: auto;
    scroll-behavior: smooth;
  }
  .term-body::-webkit-scrollbar {
    width: 8px;
  }
  .term-body::-webkit-scrollbar-thumb {
    background: rgba(214, 168, 93, 0.18);
    border-radius: 8px;
  }
  .tline {
    display: flex;
    align-items: baseline;
    gap: 0.75rem;
    padding: 0.12rem 0;
    color: var(--muted);
    font-variant-numeric: tabular-nums;
    border-radius: 4px;
  }
  .gutter {
    color: var(--nord-dim);
    opacity: 0.55;
    min-width: 1.4rem;
    text-align: right;
    user-select: none;
    flex-shrink: 0;
  }
  .tlead {
    min-width: 16rem;
    color: var(--text);
    overflow-wrap: anywhere;
  }
  .tline.bad .tlead {
    color: #ef8078;
  }
  .tline.bad .gutter {
    color: #d96862;
    opacity: 0.88;
  }
  .tline.good .tlead {
    color: #79d29a;
  }
  .tline.good .gutter {
    color: #53b77a;
    opacity: 0.82;
  }
  .cursor {
    display: inline-block;
    width: 0.5rem;
    height: 0.85rem;
    background: var(--nord-ember-bright);
    align-self: center;
    animation: blink 1s steps(2, start) infinite;
    flex-shrink: 0;
  }
  .tline.running {
    color: var(--nord-ember-bright);
  }
  @keyframes blink {
    50% {
      opacity: 0;
    }
  }
</style>
