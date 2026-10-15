<script lang="ts">
  import { ask, open as openFile, save as saveFile } from "@tauri-apps/plugin-dialog";
  import { api, errorText, type Account } from "$lib/api";
  import { baseName, plural } from "$lib/format";
  import { providerDef } from "$lib/providers";
  import Icon from "./Icon.svelte";
  import PasswordInput from "./PasswordInput.svelte";

  let { open, accounts, onclose, onadd, onedit, onremove, onreorder, onimported }: {
    open: boolean;
    accounts: Account[];
    onclose: () => void;
    onadd: () => void;
    onedit: (account: Account) => void;
    /** Removes the account; a rejection is shown in the dialog. */
    onremove: (account: Account) => Promise<void>;
    /** Saves a new order of the accounts by id; a rejection is shown in the dialog. */
    onreorder: (ids: string[]) => Promise<void>;
    /** Accounts were added from an exported file. */
    onimported: () => Promise<void>;
  } = $props();

  let dialog: HTMLDialogElement;
  let list = $state<HTMLUListElement>();
  let error = $state("");
  let info = $state("");
  /** Export or import in progress: the password form is shown. */
  let backup = $state<{ mode: "export" } | { mode: "import"; path: string } | null>(null);
  let password = $state("");
  let repeat = $state("");
  let busy = $state(false);

  const BACKUP_FILTERS = [{ name: "Аккаунты Belka", extensions: ["belka"] }];

  function startExport() {
    error = info = password = repeat = "";
    backup = { mode: "export" };
  }

  async function startImport() {
    error = info = password = repeat = "";
    const path = await openFile({ title: "Импорт аккаунтов", filters: BACKUP_FILTERS });
    if (typeof path === "string") backup = { mode: "import", path };
  }

  function accountsText(n: number) {
    return plural(n, "аккаунт", "аккаунта", "аккаунтов");
  }

  async function submitBackup(event: SubmitEvent) {
    event.preventDefault();
    if (!backup || busy) return;
    error = "";
    if (!password) {
      error = "Введите пароль";
      return;
    }
    if (backup.mode === "export" && password !== repeat) {
      error = "Пароли не совпадают";
      return;
    }
    busy = true;
    try {
      if (backup.mode === "export") {
        const date = new Date().toISOString().slice(0, 10);
        const path = await saveFile({
          title: "Экспорт аккаунтов",
          defaultPath: `belka-accounts-${date}.belka`,
          filters: BACKUP_FILTERS,
        });
        if (!path) return;
        const count = await api.exportAccounts(path, password);
        info = `Сохранено: ${accountsText(count)} → ${baseName(path)}`;
      } else {
        const { added, skipped } = await api.importAccounts(backup.path, password);
        if (added) await onimported();
        info = `Добавлено: ${accountsText(added)}` + (skipped ? `, уже были: ${skipped}` : "");
      }
      backup = null;
      password = repeat = "";
    } catch (err) {
      error = errorText(err);
    } finally {
      busy = false;
    }
  }
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
      error = info = password = repeat = "";
      backup = null;
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

<dialog class="modal"
  bind:this={dialog}
  onclose={onclose}
  oncancel={(e) => {
    if (busy) e.preventDefault();
  }}
>
  <div class="body">
    <header>
      <h2>Аккаунты</h2>
      <button class="icon-btn" title="Закрыть" onclick={() => dialog.close()} disabled={busy}>
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
              <Icon name="settings" size={14} /> <span class="label">Настроить</span>
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

    {#if backup}
      <form class="backup" onsubmit={submitBackup}>
        {#if backup.mode === "export"}
          <h3>Экспорт аккаунтов</h3>
          <p class="help">
            Все аккаунты вместе с паролями и ключами доступа сохраняются в один файл, зашифрованный
            паролем. Без пароля файл не открыть, а восстановить пароль нельзя.
          </p>
          <label>
            <span>Пароль файла</span>
            <PasswordInput bind:value={password} autocomplete="new-password" disabled={busy} />
          </label>
          <label>
            <span>Пароль ещё раз</span>
            <PasswordInput bind:value={repeat} autocomplete="new-password" disabled={busy} />
          </label>
        {:else}
          <h3>Импорт аккаунтов</h3>
          <p class="help">
            Из файла <b>{baseName(backup.path)}</b> добавятся аккаунты, которых здесь ещё нет.
          </p>
          <label>
            <span>Пароль файла</span>
            <PasswordInput bind:value={password} autocomplete="current-password" disabled={busy} />
          </label>
        {/if}
        <div class="actions">
          <button type="button" class="btn" onclick={() => (backup = null)} disabled={busy}>Отмена</button>
          <button type="submit" class="btn primary" disabled={busy}>
            {#if backup.mode === "export"}
              {busy ? "Шифрование…" : "Сохранить в файл…"}
            {:else}
              {busy ? "Расшифровка…" : "Импортировать"}
            {/if}
          </button>
        </div>
      </form>
    {/if}

    {#if error}
      <div class="error">{error}</div>
    {:else if info}
      <div class="info">{info}</div>
    {/if}

    <footer>
      <button class="btn" onclick={startImport} disabled={busy}>
        <Icon name="download" size={14} /> Импорт…
      </button>
      <button class="btn" onclick={startExport} disabled={busy || !accounts.length}>
        <Icon name="upload" size={14} /> Экспорт…
      </button>
      <span class="spacer"></span>
      <button class="btn primary" onclick={onadd}><Icon name="plus" size={14} /> Добавить аккаунт</button>
    </footer>
  </div>
</dialog>

<style>
  dialog {
    width: min(560px, calc(100vw - 32px));
    max-height: calc(100vh - 32px);
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
    width: 28px;
    height: 28px;
    border-radius: 7px;
    font-size: 11px;
  }

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
    gap: 8px;
  }

  .spacer {
    flex: 1;
  }

  .backup {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 12px;
    border: 1px solid var(--border);
    border-radius: 8px;
  }

  h3 {
    margin: 0;
    font-size: 14px;
    font-weight: 600;
  }

  .help {
    margin: 0;
    color: var(--muted);
    font-size: 12px;
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }

  .info {
    padding: 8px 10px;
    border-radius: 6px;
    background: var(--hover);
    word-break: break-word;
  }
</style>
