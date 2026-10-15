<script lang="ts">
  import { onMount, tick } from "svelte";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { ask } from "@tauri-apps/plugin-dialog";
  import { revealItemInDir } from "@tauri-apps/plugin-opener";
  import { api, errorText, type Account, type Entry, type Listing } from "$lib/api";
  import { dnd, paneAt } from "$lib/dnd.svelte";
  import { parentPath, splitPath } from "$lib/format";
  import { providerDef } from "$lib/providers";
  import { transfers } from "$lib/transfers.svelte";
  import AccountList from "$lib/components/AccountList.svelte";
  import AccountMenu from "$lib/components/AccountMenu.svelte";
  import AddAccountDialog from "$lib/components/AddAccountDialog.svelte";
  import FilePane, { type PaneOps } from "$lib/components/FilePane.svelte";
  import Icon from "$lib/components/Icon.svelte";
  import TransferPanel from "$lib/components/TransferPanel.svelte";

  const KEY_ACCOUNT = "lastAccount";
  const KEY_LOCAL = "lastLocalPath";

  function stored(key: string): string | null {
    try {
      return localStorage.getItem(key);
    } catch {
      return null;
    }
  }

  function remember(key: string, value: string) {
    try {
      localStorage.setItem(key, value);
    } catch {
      // Remembering is a convenience only.
    }
  }

  let accounts = $state<Account[]>([]);
  let loaded = $state(false);
  let selectedId = $state<string | null>(null);
  let adding = $state(false);
  let notice = $state("");
  let busy = $state("");
  /** A message about a finished action, hidden after a while. */
  let info = $state("");
  let infoTimer: ReturnType<typeof setTimeout> | undefined;

  function showInfo(text: string) {
    info = text;
    clearTimeout(infoTimer);
    infoTimer = setTimeout(() => (info = ""), 10_000);
  }

  let cloudPane = $state<FilePane>();
  let localPane = $state<FilePane>();
  let activePane = $state<"cloud" | "local">("cloud");
  let cloudPath = $state("/");
  let localPath = $state(stored(KEY_LOCAL) ?? "");
  /** Pane that files dragged from Explorer are currently over. */
  let externalOver = $state<string | null>(null);

  const selected = $derived(accounts.find((a) => a.id === selectedId) ?? null);

  function describe(items: Entry[]) {
    return items.length === 1 ? `«${items[0].name}»` : `выбранные объекты (${items.length})`;
  }

  function cloudOps(account: Account): PaneOps {
    return {
      canModify: () => true,
      mkdir: (parent, name) => api.cloudMkdir(account.id, parent, name),
      rename: (entry, name) => api.cloudRename(account.id, entry.path, name, entry.is_dir),
      remove: (items) => api.cloudDelete(account.id, items),
      deletePrompt: (items) =>
        `Удалить ${describe(items)} из облака «${account.name}»?

Файлы удаляются безвозвратно, минуя корзину облака.`,
      deleteLabel: "Удалить",
      ...(providerDef(account.kind).share && {
        share: async (items) => {
          const links = await api.cloudShare(account.id, items);
          showInfo(
            links.length === 1
              ? `Ссылка скопирована: ${links[0]}`
              : `Ссылки скопированы в буфер обмена (${links.length})`,
          );
        },
        unshare: async (items) => {
          await api.cloudUnshare(account.id, items);
          showInfo(`Доступ по ссылке закрыт: ${describe(items)}`);
        },
      }),
    };
  }

  const localOps: PaneOps = {
    canModify: (folder) => folder !== "",
    mkdir: (parent, name) => api.localMkdir(parent, name),
    rename: (entry, name) => api.localRename(entry.path, name),
    remove: (items) => api.localDelete(items.map((e) => e.path)),
    deletePrompt: (items) => `Переместить ${describe(items)} в корзину?`,
    deleteLabel: "Удалить",
    reveal: (path) => revealItemInDir(path),
  };

  async function cloudListing(accountId: string, path: string): Promise<Listing> {
    const entries = await api.listDir(accountId, path);
    return {
      path,
      parent: path === "/" ? null : parentPath(path),
      crumbs: splitPath(path),
      entries,
    };
  }

  function selectAccount(account: Account) {
    selectedId = account.id;
    cloudPath = "/";
    remember(KEY_ACCOUNT, account.id);
  }

  function added(account: Account) {
    accounts = [...accounts, account];
    selectAccount(account);
    adding = false;
  }

  async function remove(account: Account) {
    try {
      await api.removeAccount(account.id);
      accounts = accounts.filter((a) => a.id !== account.id);
      if (selectedId === account.id && accounts[0]) selectAccount(accounts[0]);
      else if (!accounts.length) selectedId = null;
    } catch (err) {
      notice = errorText(err);
    }
  }

  async function run(label: string, action: () => Promise<void>) {
    busy = label;
    notice = "";
    try {
      await action();
    } catch (err) {
      notice = errorText(err);
    } finally {
      busy = "";
    }
  }

  function confirmReplace(names: string[], where: string) {
    const list = names.slice(0, 5).map((n) => `• ${n}`).join("\n");
    const more = names.length > 5 ? `\n…и ещё ${names.length - 5}` : "";
    return ask(`${where} уже есть:\n${list}${more}\n\nЗаменить?`, {
      title: "Файлы уже существуют",
      kind: "warning",
      okLabel: "Заменить",
      cancelLabel: "Отмена",
    });
  }

  /** Downloads the cloud selection into the folder open on the right. */
  async function toLocal() {
    const account = selected;
    const items = cloudPane?.getSelection() ?? [];
    if (!account || !items.length) return;
    if (!localPath) {
      notice = "Справа откройте папку, в которую нужно скачать файлы.";
      return;
    }
    const dir = localPath;
    await run("Подготовка скачивания…", async () => {
      const conflicts = await api.localConflicts(dir, items.map((e) => e.name));
      if (conflicts.length && !(await confirmReplace(conflicts, "В папке на компьютере"))) return;
      await api.download(account.id, items, dir);
      transfers.kick();
    });
  }

  /** Uploads local files (the right-hand selection by default) into the open cloud folder. */
  async function toCloud(paths?: string[]) {
    const account = selected;
    if (!account || !cloudPane) return;
    let localPaths = paths;
    if (!localPaths) {
      const items: Entry[] = localPane?.getSelection() ?? [];
      if (items.some((e) => e.icon === "drive")) {
        notice = "Диск целиком загрузить нельзя: откройте его и выберите папки или файлы.";
        return;
      }
      localPaths = items.map((e) => e.path);
    }
    if (!localPaths.length) return;
    const dir = cloudPane.getPath();
    const existing = new Set(cloudPane.getEntries().map((e) => e.name.toLowerCase()));
    const names = localPaths.map((p) => p.split(/[\\/]/).filter(Boolean).at(-1) ?? p);
    const sources = localPaths;
    await run("Подготовка загрузки…", async () => {
      const conflicts = names.filter((n) => existing.has(n.toLowerCase()));
      if (conflicts.length && !(await confirmReplace(conflicts, "В этой папке облака"))) return;
      await api.upload(account.id, sources, dir);
      transfers.kick();
    });
  }

  function dragTo(from: "cloud" | "local", target: string) {
    if (from === "cloud" && target === "local") toLocal();
    else if (from === "local" && target === "cloud") toCloud();
  }

  function isInside(file: string, dir: string) {
    const norm = (p: string) => p.replace(/[\\/]+$/, "").toLowerCase();
    return norm(file).startsWith(norm(dir) + (dir.includes("\\") ? "\\" : "/"));
  }

  const reloadTimers: Record<string, ReturnType<typeof setTimeout>> = {};

  /** Refreshes a pane once a burst of finished transfers settles. */
  function reloadSoon(pane: "cloud" | "local") {
    clearTimeout(reloadTimers[pane]);
    reloadTimers[pane] = setTimeout(() => {
      (pane === "cloud" ? cloudPane : localPane)?.reload();
    }, 500);
  }

  onMount(() => {
    transfers.kick();
    api
      .listAccounts()
      .then((list) => {
        accounts = list;
        const last = stored(KEY_ACCOUNT);
        selectedId = list.find((a) => a.id === last)?.id ?? list[0]?.id ?? null;
      })
      .catch((err) => (notice = errorText(err)))
      .finally(async () => {
        loaded = true;
        // The window starts hidden so the user never sees a blank webview.
        await tick();
        getCurrentWindow().show();
      });

    const offFinished = transfers.onFinished((t) => {
      if (t.direction === "upload" && t.account_id === selectedId && t.remote_dir === cloudPath) {
        reloadSoon("cloud");
      } else if (t.direction === "download" && localPath && isInside(t.local_path, localPath)) {
        reloadSoon("local");
      }
    });

    // Files dragged in from Explorer can be dropped onto the cloud pane.
    const unlisten = getCurrentWebview().onDragDropEvent((event) => {
      const payload = event.payload;
      if (payload.type === "leave") {
        externalOver = null;
        return;
      }
      const scale = window.devicePixelRatio || 1;
      const pane = paneAt(payload.position.x / scale, payload.position.y / scale);
      if (payload.type === "drop") {
        externalOver = null;
        if (pane === "cloud" && payload.paths.length) toCloud(payload.paths);
      } else {
        externalOver = pane === "cloud" && selected ? "cloud" : null;
      }
    });

    return () => {
      offFinished();
      unlisten.then((fn) => fn());
    };
  });
</script>

<main>
  <div class="panes">
    <div class="slot">
      {#if selected}
        {#key selected.id}
          <FilePane
            bind:this={cloudPane}
            id="cloud"
            load={(path) => cloudListing(selected.id, path)}
            initialPath="/"
            rootPath="/"
            rootLabel={selected.name}
            rootIcon="cloud"
            active={activePane === "cloud"}
            externalDrop={externalOver === "cloud"}
            dropLabel="Загрузить в эту папку облака"
            onactivate={() => (activePane = "cloud")}
            oncopy={toLocal}
            ondragto={(target) => dragTo("cloud", target)}
            onswitch={() => localPane?.focus()}
            onnavigate={(path) => (cloudPath = path)}
            ops={cloudOps(selected)}
            copyLabel="Скачать на компьютер"
            canCopy={!busy && !!localPath}
          >
            {#snippet leading()}
              <AccountMenu
                {accounts}
                current={selected}
                onselect={selectAccount}
                onadd={() => (adding = true)}
                onremove={remove}
              />
            {/snippet}
            {#snippet actions()}
              <button
                class="btn primary"
                onclick={toLocal}
                disabled={!!busy || !localPath}
                title={localPath ? `Скачать выбранное в ${localPath} (F5)` : "Справа откройте папку на компьютере"}
              >
                Скачать <Icon name="arrowRight" size={14} />
              </button>
            {/snippet}
          </FilePane>
        {/key}
      {:else if loaded}
        <div class="welcome">
          {#if accounts.length}
            <AccountList
              {accounts}
              selectedId={null}
              onselect={selectAccount}
              onadd={() => (adding = true)}
              onremove={remove}
            />
          {:else}
            <h1>Belka</h1>
            <p>Подключите облако, чтобы работать с файлами.</p>
            <button class="btn primary" onclick={() => (adding = true)}>Добавить аккаунт</button>
          {/if}
        </div>
      {/if}
    </div>

    <div class="slot">
      <FilePane
        bind:this={localPane}
        id="local"
        load={api.listLocal}
        initialPath={localPath}
        rootPath=""
        rootLabel="Этот компьютер"
        rootIcon="computer"
        active={activePane === "local"}
        dropLabel={localPath ? "Скачать в эту папку" : undefined}
        onactivate={() => (activePane = "local")}
        oncopy={() => toCloud()}
        ondragto={(target) => dragTo("local", target)}
        onswitch={() => cloudPane?.focus()}
        onnavigate={(path) => {
          localPath = path;
          remember(KEY_LOCAL, path);
        }}
        ops={localOps}
        copyLabel="Загрузить в облако"
        canCopy={!busy && !!selected}
      >
        {#snippet actions()}
          <button
            class="btn primary"
            onclick={() => toCloud()}
            disabled={!!busy || !selected}
            title="Загрузить выбранное в открытую слева папку облака (F5)"
          >
            <Icon name="arrowLeft" size={14} /> Загрузить
          </button>
        {/snippet}
      </FilePane>
    </div>
  </div>

  {#if notice || busy || info}
    <div class="notice" class:error={!!notice}>
      <span>{notice || busy || info}</span>
      {#if notice || info}
        <button class="icon-btn" title="Скрыть" onclick={() => (notice = info = "")}>
          <Icon name="close" size={14} />
        </button>
      {/if}
    </div>
  {/if}

  <TransferPanel />
</main>

{#if dnd.from}
  <div class="ghost" style:left="{dnd.x + 14}px" style:top="{dnd.y + 14}px">
    <Icon name={dnd.from === "cloud" ? "download" : "upload"} size={14} />
    {dnd.count === 1 ? "1 объект" : `${dnd.count} объектов`}
  </div>
{/if}

<AddAccountDialog open={adding} onclose={() => (adding = false)} onadded={added} />

<style>
  main {
    display: flex;
    flex-direction: column;
    height: 100vh;
  }

  .panes {
    flex: 1;
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
    min-height: 0;
  }

  .slot {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
  }

  .slot + .slot {
    border-left: 1px solid var(--border);
  }

  .slot > :global(section) {
    flex: 1;
  }

  .notice {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 6px 12px;
    border-top: 1px solid var(--border);
    background: var(--surface);
    color: var(--muted);
  }

  .notice.error {
    border-radius: 0;
  }

  .welcome {
    display: flex;
    flex-direction: column;
    justify-content: center;
    align-items: center;
    flex: 1;
    padding: 24px;
    color: var(--muted);
    text-align: center;
  }

  .welcome h1 {
    margin: 0 0 6px;
    color: var(--text);
    font-size: 20px;
    font-weight: 600;
  }

  .welcome p {
    margin: 0 0 16px;
  }

  .welcome > :global(.accounts) {
    width: min(320px, 100%);
    text-align: left;
  }

  .ghost {
    position: fixed;
    z-index: 50;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 10px;
    border-radius: 6px;
    background: var(--accent);
    color: var(--accent-text);
    font-size: 12px;
    pointer-events: none;
    box-shadow: var(--shadow);
  }
</style>
