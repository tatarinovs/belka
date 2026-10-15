<script lang="ts">
  import type { Account } from "$lib/api";
  import { providerDef } from "$lib/providers";
  import Icon from "./Icon.svelte";

  let {
    accounts,
    selectedId,
    onselect,
    onadd,
    onremove,
  }: {
    accounts: Account[];
    selectedId: string | null;
    onselect: (account: Account) => void;
    onadd: () => void;
    onremove: (account: Account) => void;
  } = $props();

  let confirmingId = $state<string | null>(null);
</script>

<div class="accounts">
  <header>
    <span>Аккаунты</span>
    <button class="icon-btn" title="Добавить аккаунт" onclick={onadd}>
      <Icon name="plus" />
    </button>
  </header>

  <ul>
    {#each accounts as account (account.id)}
      <li class:selected={account.id === selectedId}>
        {#if confirmingId === account.id}
          <div class="confirm">
            <span>Удалить «{account.name}»?</span>
            <div>
              <button class="btn danger" onclick={() => { confirmingId = null; onremove(account); }}>
                Удалить
              </button>
              <button class="btn" onclick={() => (confirmingId = null)}>Отмена</button>
            </div>
          </div>
        {:else}
          <button class="row" onclick={() => onselect(account)}>
            <span class="badge" data-kind={account.kind}>{providerDef(account.kind).short}</span>
            <span class="text">
              <span class="name">{account.name}</span>
              <span class="kind">{providerDef(account.kind).title}</span>
            </span>
          </button>
          <button
            class="icon-btn remove"
            title="Удалить аккаунт"
            onclick={() => (confirmingId = account.id)}
          >
            <Icon name="trash" size={14} />
          </button>
        {/if}
      </li>
    {:else}
      <li class="empty">
        <p>Нет подключённых облаков.</p>
        <button class="btn primary" onclick={onadd}><Icon name="plus" size={14} /> Добавить</button>
      </li>
    {/each}
  </ul>
</div>

<style>
  .accounts {
    display: flex;
    flex-direction: column;
    min-height: 0;
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 8px 6px 14px;
    color: var(--muted);
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  ul {
    flex: 1;
    margin: 0;
    padding: 0 6px 8px;
    overflow-y: auto;
    list-style: none;
  }

  li {
    position: relative;
    display: flex;
    align-items: center;
    border-radius: 6px;
  }

  li:hover {
    background: var(--hover);
  }

  li.selected {
    background: var(--selected);
  }

  .row {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
    padding: 6px 8px;
    border: 0;
    background: none;
    text-align: left;
  }

  .text {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .name,
  .kind {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .kind {
    color: var(--muted);
    font-size: 11px;
  }

  .badge {
    flex: none;
    display: grid;
    place-items: center;
    width: 28px;
    height: 28px;
    border-radius: 7px;
    color: #fff;
    font-size: 11px;
    font-weight: 700;
    background: #6b7280;
  }

  .badge[data-kind="yandex"] { background: #e8492f; }
  .badge[data-kind="mailru"] { background: #1a73e8; }
  .badge[data-kind="google"] { background: #1e8e3e; }
  .badge[data-kind="webdav"] { background: #7c4dcc; }
  .badge[data-kind="s3"] { background: #c77700; }
  .badge[data-kind="crypt"] { background: #455a64; }

  .remove {
    visibility: hidden;
    margin-right: 4px;
  }

  li:hover .remove,
  .remove:focus-visible {
    visibility: visible;
  }

  .confirm {
    display: flex;
    flex-direction: column;
    gap: 6px;
    width: 100%;
    padding: 8px;
  }

  .confirm div {
    display: flex;
    gap: 6px;
  }

  .empty {
    flex-direction: column;
    align-items: flex-start;
    padding: 8px;
    color: var(--muted);
  }

  .empty:hover {
    background: none;
  }

  .empty p {
    margin: 0 0 8px;
  }
</style>
