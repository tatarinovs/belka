<script lang="ts" module>
  import type { IconName } from "./Icon.svelte";

  export type MenuItem =
    | {
        label: string;
        icon?: IconName;
        shortcut?: string;
        disabled?: boolean;
        danger?: boolean;
        action: () => void;
      }
    | "separator";
</script>

<script lang="ts">
  import Icon from "./Icon.svelte";

  let { x, y, items, onclose }: {
    x: number;
    y: number;
    items: MenuItem[];
    onclose: () => void;
  } = $props();

  let menu: HTMLDivElement;
  let left = $state(0);
  let top = $state(0);

  // Keep the menu inside the window.
  $effect(() => {
    const { width, height } = menu.getBoundingClientRect();
    left = Math.max(4, Math.min(x, window.innerWidth - width - 4));
    top = Math.max(4, Math.min(y, window.innerHeight - height - 4));
    menu.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
  });

  function pick(item: Exclude<MenuItem, "separator">) {
    onclose();
    item.action();
  }

  function onkeydown(event: KeyboardEvent) {
    if (event.key === "Escape") {
      event.preventDefault();
      onclose();
      return;
    }
    if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
    event.preventDefault();
    const buttons = [...menu.querySelectorAll<HTMLButtonElement>("button:not(:disabled)")];
    const index = buttons.indexOf(document.activeElement as HTMLButtonElement);
    const next = event.key === "ArrowDown" ? index + 1 : index - 1;
    buttons[(next + buttons.length) % buttons.length]?.focus();
  }

  function outside(event: PointerEvent) {
    if (!menu.contains(event.target as Node)) onclose();
  }
</script>

<svelte:window onpointerdown={outside} onblur={onclose} onresize={onclose} />

<div class="menu" role="menu" tabindex="-1" bind:this={menu} style:left="{left}px" style:top="{top}px" {onkeydown}>
  {#each items as item, i (i)}
    {#if item === "separator"}
      <div class="sep" role="separator"></div>
    {:else}
      <button role="menuitem" class:danger={item.danger} disabled={item.disabled} onclick={() => pick(item)}>
        <span class="ico">{#if item.icon}<Icon name={item.icon} size={14} />{/if}</span>
        <span class="label">{item.label}</span>
        {#if item.shortcut}<span class="key">{item.shortcut}</span>{/if}
      </button>
    {/if}
  {/each}
</div>

<style>
  .menu {
    position: fixed;
    z-index: 100;
    min-width: 220px;
    padding: 4px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--surface);
    box-shadow: var(--shadow);
    outline: none;
  }

  button {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    height: 28px;
    padding: 0 8px;
    border: 0;
    border-radius: 5px;
    background: none;
    text-align: left;
  }

  button:hover:not(:disabled),
  button:focus-visible {
    background: var(--hover);
    outline: none;
  }

  button:disabled {
    opacity: 0.45;
    cursor: default;
  }

  button.danger {
    color: var(--danger);
  }

  .ico {
    display: inline-grid;
    width: 14px;
    color: var(--muted);
  }

  .danger .ico {
    color: inherit;
  }

  .label {
    flex: 1;
  }

  .key {
    color: var(--muted);
    font-size: 11px;
  }

  .sep {
    height: 1px;
    margin: 4px 6px;
    background: var(--border);
  }
</style>
