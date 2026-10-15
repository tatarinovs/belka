<script lang="ts">
  import { ask } from "@tauri-apps/plugin-dialog";
  import { errorText, type Account } from "$lib/api";
  import { providerDef } from "$lib/providers";
  import Icon from "./Icon.svelte";

  let { open, accounts, onclose, onadd, onedit, onremove, onreorder }: {
    open: boolean;
    accounts: Account[];
    onclose: () => void;
    onadd: () => void;
    onedit: (account: Account) => void;
    /** Removes the account; a rejection is shown in the dialog. */
    onremove: (account: Account) => Promise<void>;
    /** Saves a new order of the accounts by id; a rejection is shown in the dialog. */
    onreorder: (ids: string[]) => Promise<void>;
  } = $props();

  let dialog: HTMLDialogElement;
  let list = $state<HTMLUListElement>();
  let error = $state("");
  /** The account being dragged and the order the list shows meanwhile. */
  let drag = $state<{ id: string; ids: string[] } | null>(null);

  const shown = $derived(
    drag
      ? drag.ids.map((id) => accounts.find((a) => a.id === id)).filter((a) => a !== undefined)
      : accounts,
  );

  async function reorder(ids: string[]) {
    if (ids.join() === accounts.map((a) => a.id).join()) return;
    error = "";
    try {
      await onreorder(ids);
    } catch (err) {
      error = errorText(err);
    }
  }

  function move(index: number, by: number) {
    const ids = accounts.map((a) => a.id);
    const [id] = ids.splice(index, 1);
    ids.splice(index + by, 0, id);
    reorder(ids);
  }

  function startDrag(event: PointerEvent, account: Account) {
    if (event.button !== 0) return;
    event.preventDefault();
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
    drag = { id: account.id, ids: accounts.map((a) => a.id) };
  }

  /** Puts the dragged row among the others by where the pointer is. */
  function dragMove(event: PointerEvent) {
    if (!drag || !list) return;
    const others = [...list.querySelectorAll<HTMLElement>("li[data-id]")].filter(
      (row) => row.dataset.id !== drag?.id,
    );
    const index = others.filter((row) => {
      const box = row.getBoundingClientRect();
      return box.top + box.height / 2 < event.clientY;
    }).length;
    const ids = others.map((row) => row.dataset.id!);
    ids.splice(index, 0, drag.id);
    if (ids.join() !== drag.ids.join()) drag.ids = ids;
  }

  function endDrag() {
    if (!drag) return;
    const ids = drag.ids;
    drag = null;
    reorder(ids);
  }

  $effect(() => {
    if (open && !dialog.open) {
      error = "";
      dialog.showModal();
    } else if (!open && dialog.open) {
      dialog.close();
    }
  });

  /** Where an account's files are, shown under its name. */
  function describe(account: Account) {
    const title = providerDef(account.kind).title;
    if (!account.crypt) return title;
    const base = accounts.find((a) => a.id === account.crypt?.base);
    return `${title}: ${base?.name ?? "?"} · ${account.crypt.root}`;
  }

  async function remove(account: Account) {
    const confirmed = await ask(`Удалить аккаунт «${account.name}»?\n\nФайлы в облаке останутся.`, {
      title: "Удаление аккаунта",
      kind: "warning",
      okLabel: "Удалить",
      cancelLabel: "Отмена",
    });
    if (!confirmed) return;
    error = "";
    try {
      await onremove(account);
    } catch (err) {
      error = errorText(err);
    }
  }
</script>

<dialog bind:this={dialog} onclose={onclose}>
  <div class="body">
    <header>
      <h2>Аккаунты</h2>
      <button class="icon-btn" title="Закрыть" onclick={() => dialog.close()}>
        <Icon name="close" />
      </button>
    </header>

    {#if accounts.length}
      <ul bind:this={list} class:dragging={drag !== null}>
        {#each shown as account, i (account.id)}
          <li data-id={account.id} class:dragged={drag?.id === account.id}>
            <span
              class="grip"
              title="Перетащите, чтобы изменить порядок"
              role="button"
              tabindex="-1"
              aria-label="Перетащить"
              onpointerdown={(e) => startDrag(e, account)}
              onpointermove={dragMove}
              onpointerup={endDrag}
              onpointercancel={() => (drag = null)}
            >
              <Icon name="grip" size={16} />
            </span>
            <span class="badge" data-kind={account.kind}>{providerDef(account.kind).short}</span>
            <span class="text">
              <span class="name">{account.name}</span>
              <span class="kind">{describe(account)}</span>
            </span>
            <span class="order">
              <button class="icon-btn" title="Выше" disabled={i === 0 || drag !== null} onclick={() => move(i, -1)}>
                <Icon name="up" size={14} />
              </button>
              <button
                class="icon-btn"
                title="Ниже"
                disabled={i === shown.length - 1 || drag !== null}
                onclick={() => move(i, 1)}
              >
                <Icon name="down" size={14} />
              </button>
            </span>
            <button class="btn" onclick={() => onedit(account)}>
              <Icon name="settings" size={14} /> Настроить
            </button>
            <button class="icon-btn danger" title="Удалить аккаунт" onclick={() => remove(account)}>
              <Icon name="trash" size={15} />
            </button>
          </li>
        {/each}
      </ul>
    {:else}
      <p class="empty">Нет подключённых облаков.</p>
    {/if}

    {#if error}
      <div class="error">{error}</div>
    {/if}

    <footer>
      <button class="btn primary" onclick={onadd}><Icon name="plus" size={14} /> Добавить аккаунт</button>
    </footer>
  </div>
</dialog>

<style>
  dialog {
    width: min(560px, calc(100vw - 32px));
    max-height: calc(100vh - 32px);
    padding: 0;
    border: 1px solid var(--border);
    border-radius: 10px;
    background: var(--surface);
    color: var(--text);
    box-shadow: var(--shadow);
  }

  dialog::backdrop {
    background: rgb(0 0 0 / 0.35);
  }

  .body {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 14px 20px 18px;
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  h2 {
    margin: 0;
    font-size: 16px;
    font-weight: 600;
  }

  ul {
    display: flex;
    flex-direction: column;
    max-height: 60vh;
    margin: 0;
    padding: 0;
    overflow-y: auto;
    list-style: none;
    border: 1px solid var(--border);
    border-radius: 8px;
  }

  li {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 10px;
  }

  li + li {
    border-top: 1px solid var(--border);
  }

  .grip {
    display: grid;
    place-items: center;
    width: 18px;
    height: 28px;
    margin-right: -4px;
    color: var(--muted);
    cursor: grab;
    touch-action: none;
  }

  .dragging,
  .dragging .grip {
    cursor: grabbing;
    user-select: none;
  }

  li.dragged {
    background: var(--selected);
  }

  .order {
    display: flex;
  }

  .text {
    flex: 1;
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
  .badge[data-kind="synology"] { background: #2f6f9f; }
  .badge[data-kind="crypt"] { background: #455a64; }

  .btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }

  .icon-btn.danger:hover {
    color: #d93025;
  }

  .empty {
    margin: 0;
    color: var(--muted);
  }

  footer {
    display: flex;
    justify-content: flex-end;
  }
</style>
