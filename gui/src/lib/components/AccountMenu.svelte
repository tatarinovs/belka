<script lang="ts">
  import type { Account } from "$lib/api";
  import { providerDef } from "$lib/providers";
  import AccountList from "./AccountList.svelte";
  import Icon from "./Icon.svelte";

  let {
    accounts,
    current,
    onselect,
    onadd,
    onremove,
  }: {
    accounts: Account[];
    current: Account;
    onselect: (account: Account) => void;
    onadd: () => void;
    onremove: (account: Account) => void;
  } = $props();

  let open = $state(false);
  let root: HTMLDivElement;

  function outside(event: PointerEvent) {
    if (open && !root.contains(event.target as Node)) open = false;
  }
</script>

<svelte:window onpointerdown={outside} onkeydown={(e) => open && e.key === "Escape" && (open = false)} />

<div class="menu" bind:this={root}>
  <button class="trigger" onclick={() => (open = !open)} aria-expanded={open} title="Сменить аккаунт">
    <span class="badge" data-kind={current.kind}>{providerDef(current.kind).short}</span>
    <span class="name">{current.name}</span>
    <Icon name="chevronDown" size={14} />
  </button>

  {#if open}
    <div class="popover">
      <AccountList
        {accounts}
        selectedId={current.id}
        onselect={(a) => {
          open = false;
          onselect(a);
        }}
        onadd={() => {
          open = false;
          onadd();
        }}
        {onremove}
      />
    </div>
  {/if}
</div>

<style>
  .menu {
    position: relative;
    margin-right: 4px;
  }

  .trigger {
    display: flex;
    align-items: center;
    gap: 8px;
    max-width: 220px;
    height: 28px;
    padding: 0 8px 0 4px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--surface);
  }

  .trigger:hover {
    background: var(--hover);
  }

  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .badge {
    flex: none;
    display: grid;
    place-items: center;
    width: 20px;
    height: 20px;
    border-radius: 5px;
    color: #fff;
    font-size: 9px;
    font-weight: 700;
    background: #6b7280;
  }

  .badge[data-kind="yandex"] { background: #e8492f; }
  .badge[data-kind="mailru"] { background: #1a73e8; }
  .badge[data-kind="google"] { background: #1e8e3e; }
  .badge[data-kind="webdav"] { background: #7c4dcc; }
  .badge[data-kind="s3"] { background: #c77700; }
  .badge[data-kind="crypt"] { background: #455a64; }

  .popover {
    position: absolute;
    top: calc(100% + 4px);
    left: 0;
    z-index: 20;
    display: flex;
    width: 280px;
    max-height: 420px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--surface);
    box-shadow: var(--shadow);
  }

  .popover > :global(*) {
    flex: 1;
  }
</style>
