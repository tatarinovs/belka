<script lang="ts">
  import { onMount, tick } from "svelte";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { ask } from "@tauri-apps/plugin-dialog";
  import { revealItemInDir } from "@tauri-apps/plugin-opener";
  import { api, errorText, type Account, type Entry, type Listing } from "$lib/api";
  import { dnd, paneAt } from "$lib/dnd.svelte";
  import { baseName, parentPath, splitPath } from "$lib/format";
  import { providerDef } from "$lib/providers";
  import { transfers } from "$lib/transfers.svelte";
  import AccountList from "$lib/components/AccountList.svelte";
  import AccountMenu from "$lib/components/AccountMenu.svelte";
  import AccountDialog from "$lib/components/AccountDialog.svelte";
  import AccountsDialog from "$lib/components/AccountsDialog.svelte";
  import FilePane, { type PaneOps } from "$lib/components/FilePane.svelte";
  import PreviewDialog from "$lib/components/PreviewDialog.svelte";
  import ShareDialog from "$lib/components/ShareDialog.svelte";
  import Icon from "$lib/components/Icon.svelte";
  import TabBar, { type Tab } from "$lib/components/TabBar.svelte";
  import TransferPanel from "$lib/components/TransferPanel.svelte";

  const KEY_TABS = "tabs";
  // Used before tabs appeared; read once to keep the last place.
  const KEY_ACCOUNT = "lastAccount";
  const KEY_LOCAL = "lastLocalPath";

  type CloudTab = { id: string; account: string; path: string };
  type LocalTab = { id: string; path: string };
  type TabAction = "new" | "close" | "next" | "prev";

  function newId() {
    return crypto.randomUUID();
  }

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
  let cloudTabs = $state<CloudTab[]>([]);
  let cloudTabId = $state<string | null>(null);
  let localTabs = $state<LocalTab[]>([]);
  let localTabId = $state<string | null>(null);
  let adding = $state(false);
  /** The account whose settings dialog is open. */
  let editing = $state<Account | null>(null);
  let managing = $state(false);
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
  const cloudTab = $derived(cloudTabs.find((t) => t.id === cloudTabId) ?? null);
  const localTab = $derived(localTabs.find((t) => t.id === localTabId) ?? null);
  const selectedId = $derived(cloudTab?.account ?? null);
  const cloudPath = $derived(cloudTab?.path ?? "/");
  const localPath = $derived(localTab?.path ?? "");
  /** Files of the cloud folder being previewed and where the preview starts. */
  let previewing = $state<{ files: Entry[]; start: number } | null>(null);
  /** Cloud items the link settings dialog is open for. */
  let sharing = $state<Entry[] | null>(null);
  /** Pane that files dragged from Explorer are currently over. */
  let externalOver = $state<string | null>(null);

  const selected = $derived(accounts.find((a) => a.id === selectedId) ?? null);

  function describe(items: Entry[]) {
    return items.length === 1 ? `«${items[0].name}»` : `выбранные объекты (${items.length})`;
  }

  function showLinks(links: string[]) {
    showInfo(
      links.length === 1
        ? `Ссылка скопирована: ${links[0]}`
        : `Ссылки скопированы в буфер обмена (${links.length})`,
    );
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
        share: async (items) => showLinks(await api.cloudShare(account.id, items)),
        unshare: async (items) => {
          await api.cloudUnshare(account.id, items);
          showInfo(`Доступ по ссылке закрыт: ${describe(items)}`);
        },
      }),
      ...(providerDef(account.kind).shareOptions && {
        shareWithOptions: (items) => (sharing = items),
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

  async function cloudListing(account: Account, path: string): Promise<Listing> {
    const entries = markEncrypted(account, await api.listDir(account.id, path));
    return {
      path,
      parent: path === "/" ? null : parentPath(path),
      crumbs: splitPath(path),
      entries,
    };
  }

  /** Marks what is stored encrypted: everything in an encrypted folder, and
   * in a regular cloud the encrypted folders and what is inside them. */
  function markEncrypted(account: Account, entries: Entry[]): Entry[] {
    if (account.kind === "crypt") {
      return entries.map((e) => ({ ...e, locked: "Хранится в облаке зашифрованным" }));
    }
    const folders = accounts
      .filter((a) => a.crypt?.base === account.id)
      .map((a) => ({ account: a, root: "/" + a.crypt!.root.replace(/^\/+|\/+$/g, "") }));
    if (!folders.length) return entries;
    return entries.map((e) => {
      for (const { account: folder, root } of folders) {
        if (e.path === root) {
          return {
            ...e,
            locked: `Зашифрованная папка «${folder.name}»: откройте, чтобы увидеть файлы`,
            opensAccount: folder.id,
          };
        }
        if (root === "/" || e.path.startsWith(root + "/")) {
          return { ...e, locked: `Зашифрованные данные папки «${folder.name}»` };
        }
      }
      return e;
    });
  }

  /** Opens `account` in the active cloud tab, as choosing a drive does in Total Commander. */
  function selectAccount(account: Account) {
    if (cloudTab) {
      cloudTab.account = account.id;
      cloudTab.path = "/";
    } else {
      openCloudTab(account.id, "/");
    }
  }

  function openCloudTab(account: string, path: string) {
    const tab = { id: newId(), account, path };
    const at = cloudTabs.findIndex((t) => t.id === cloudTabId);
    cloudTabs.splice(at < 0 ? cloudTabs.length : at + 1, 0, tab);
    cloudTabId = tab.id;
  }

  function openLocalTab(path: string) {
    const tab = { id: newId(), path };
    const at = localTabs.findIndex((t) => t.id === localTabId);
    localTabs.splice(at < 0 ? localTabs.length : at + 1, 0, tab);
    localTabId = tab.id;
  }

  function closeTab(side: "cloud" | "local", id: string) {
    const tabs = side === "cloud" ? cloudTabs : localTabs;
    if (tabs.length < 2) return;
    const at = tabs.findIndex((t) => t.id === id);
    if (at < 0) return;
    tabs.splice(at, 1);
    const next = tabs[Math.min(at, tabs.length - 1)].id;
    if (side === "cloud" && cloudTabId === id) cloudTabId = next;
    if (side === "local" && localTabId === id) localTabId = next;
  }

  function reorderTabs<T extends { id: string }>(tabs: T[], ids: string[]): T[] {
    return ids.map((id) => tabs.find((t) => t.id === id)).filter((t) => t !== undefined);
  }

  /** Tab commands for the pane's tabs: Ctrl+T, Ctrl+W, Ctrl+Tab and the "+" button. */
  async function tabKey(side: "cloud" | "local", action: TabAction) {
    const tabs = side === "cloud" ? cloudTabs : localTabs;
    const current = side === "cloud" ? cloudTabId : localTabId;
    if (action === "new") {
      if (side === "cloud" && cloudTab) openCloudTab(cloudTab.account, cloudTab.path);
      else if (side === "local") openLocalTab(localPath);
    } else if (action === "close") {
      if (current) closeTab(side, current);
    } else {
      const at = tabs.findIndex((t) => t.id === current);
      const next = tabs[(at + (action === "next" ? 1 : tabs.length - 1)) % tabs.length]?.id ?? null;
      if (side === "cloud") cloudTabId = next;
      else localTabId = next;
    }
    // The pane is remounted for the new tab; keep the keyboard in it.
    activePane = side;
    await tick();
    (side === "cloud" ? cloudPane : localPane)?.focus();
  }

  function localLabel(path: string) {
    if (!path) return "Этот компьютер";
    return baseName(path);
  }

  const cloudTabItems = $derived(
    cloudTabs.map((t): Tab => {
      const account = accounts.find((a) => a.id === t.account);
      const name = account?.name ?? "?";
      return {
        id: t.id,
        label: t.path === "/" ? name : (t.path.split("/").filter(Boolean).at(-1) ?? t.path),
        title: `${name}: ${t.path}`,
        kind: account?.kind,
        badge: account ? providerDef(account.kind).short : undefined,
      };
    }),
  );

  const localTabItems = $derived(
    localTabs.map((t): Tab => ({ id: t.id, label: localLabel(t.path), title: t.path || "Этот компьютер" })),
  );

  /** Restores the tabs of the last session, dropping ones of removed accounts. */
  function restoreTabs(list: Account[]) {
    let saved: {
      cloud?: { tabs: CloudTab[]; active: string | null };
      local?: { tabs: LocalTab[]; active: string | null };
    } = {};
    try {
      saved = JSON.parse(stored(KEY_TABS) ?? "{}");
    } catch {
      // Start with fresh tabs.
    }
    const known = new Set(list.map((a) => a.id));
    cloudTabs = (saved.cloud?.tabs ?? []).filter((t) => known.has(t.account));
    if (!cloudTabs.length && list.length) {
      const last = stored(KEY_ACCOUNT) ?? "";
      cloudTabs = [{ id: newId(), account: known.has(last) ? last : list[0].id, path: "/" }];
    }
    cloudTabId = cloudTabs.find((t) => t.id === saved.cloud?.active)?.id ?? cloudTabs[0]?.id ?? null;
    localTabs = saved.local?.tabs?.length ? saved.local.tabs : [{ id: newId(), path: stored(KEY_LOCAL) ?? "" }];
    localTabId = localTabs.find((t) => t.id === saved.local?.active)?.id ?? localTabs[0].id;
  }

  $effect(() => {
    if (!loaded) return;
    remember(
      KEY_TABS,
      JSON.stringify({
        cloud: { tabs: cloudTabs, active: cloudTabId },
        local: { tabs: localTabs, active: localTabId },
      }),
    );
  });

  function added(account: Account) {
    accounts = [...accounts, account];
    openCloudTab(account.id, "/");
    adding = false;
  }

  function saved(account: Account) {
    if (!editing) {
      added(account);
      return;
    }
    accounts = accounts.map((a) => (a.id === account.id ? account : a));
    editing = null;
    // New credentials may show other files.
    if (selectedId === account.id) cloudPane?.navigate(cloudPath);
  }

  /** Shows the new order at once and goes back to the old one if saving fails. */
  async function reorderAccounts(ids: string[]) {
    const before = accounts;
    accounts = ids
      .map((id) => before.find((a) => a.id === id))
      .filter((a) => a !== undefined);
    try {
      await api.reorderAccounts(ids);
    } catch (err) {
      accounts = before;
      throw err;
    }
  }

  async function removeAccount(account: Account) {
    await api.removeAccount(account.id);
    accounts = accounts.filter((a) => a.id !== account.id);
    // Tabs of the removed account close; if none are left, another account opens.
    const left = cloudTabs.filter((t) => t.account !== account.id);
    if (!left.length && accounts[0]) {
      left.push({ id: newId(), account: accounts[0].id, path: "/" });
    }
    cloudTabs = left;
    if (!left.some((t) => t.id === cloudTabId)) cloudTabId = left[0]?.id ?? null;
  }

  /** Shows accounts added from an exported file; with none before, opens the first. */
  async function importedAccounts() {
    accounts = await api.listAccounts();
    if (!cloudTabs.length && accounts[0]) openCloudTab(accounts[0].id, "/");
  }

  async function remove(account: Account) {
    try {
      await removeAccount(account);
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
    const names = localPaths.map(baseName);
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
        restoreTabs(list);
      })
      .catch((err) => {
        notice = errorText(err);
        restoreTabs([]);
      })
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
      {#if cloudTabs.length}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div onpointerdown={() => (activePane = "cloud")}>
          <TabBar
            tabs={cloudTabItems}
            activeId={cloudTabId}
            onselect={(id) => (cloudTabId = id)}
            onclose={(id) => closeTab("cloud", id)}
            onnew={() => tabKey("cloud", "new")}
            onreorder={(ids) => (cloudTabs = reorderTabs(cloudTabs, ids))}
          />
        </div>
      {/if}
      {#if selected && cloudTab}
        {#key `${cloudTab.id}:${selected.id}`}
          <FilePane
            bind:this={cloudPane}
            id="cloud"
            load={(path) => cloudListing(selected, path)}
            initialPath={cloudTab.path}
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
            onnavigate={(path) => {
              if (cloudTab) cloudTab.path = path;
            }}
            onopentab={(entry) => {
              if (entry.opensAccount) openCloudTab(entry.opensAccount, "/");
              else openCloudTab(selected.id, entry.path);
            }}
            ontabkey={(action) => tabKey("cloud", action)}
            onpreview={(entry, files) => (previewing = { files, start: files.indexOf(entry) })}
            onopenaccount={(id) => {
              const folder = accounts.find((a) => a.id === id);
              if (folder) selectAccount(folder);
            }}
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
                onedit={(a) => (editing = a)}
                onmanage={() => (managing = true)}
                onremove={remove}
              />
            {/snippet}
            {#snippet actions()}
              <button
                class="btn primary arrow"
                onclick={toLocal}
                disabled={!!busy || !localPath}
                title={localPath ? `Скачать выбранное в ${localPath} (F5)` : "Справа откройте папку на компьютере"}
                aria-label="Скачать"
              >
                <Icon name="arrowRight" size={16} />
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
              onedit={(a) => (editing = a)}
              onmanage={() => (managing = true)}
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
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div onpointerdown={() => (activePane = "local")}>
        <TabBar
          tabs={localTabItems}
          activeId={localTabId}
          onselect={(id) => (localTabId = id)}
          onclose={(id) => closeTab("local", id)}
          onnew={() => tabKey("local", "new")}
          onreorder={(ids) => (localTabs = reorderTabs(localTabs, ids))}
        />
      </div>
      {#if localTab}
        {#key localTab.id}
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
          if (localTab) localTab.path = path;
        }}
        onopentab={(entry) => openLocalTab(entry.path)}
        ontabkey={(action) => tabKey("local", action)}
        ops={localOps}
        onopenfile={(entry) => api.localOpen(entry.path).catch((err) => (notice = errorText(err)))}
        copyLabel="Загрузить в облако"
        canCopy={!busy && !!selected}
      >
        {#snippet leading()}
          <button
            class="btn primary arrow"
            onclick={() => toCloud()}
            disabled={!!busy || !selected}
            title="Загрузить выбранное в открытую слева папку облака (F5)"
            aria-label="Загрузить"
          >
            <Icon name="arrowLeft" size={16} />
          </button>
        {/snippet}
      </FilePane>
        {/key}
      {/if}
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

{#if previewing && selected}
  <PreviewDialog
    account={selected}
    files={previewing.files}
    start={previewing.start}
    onclose={() => {
      previewing = null;
      cloudPane?.focus();
    }}
  />
{/if}

{#if selected}
  {@const account = selected}
  <ShareDialog
    entries={sharing}
    onclose={() => {
      sharing = null;
      cloudPane?.focus();
    }}
    onshare={async (options) => {
      if (!sharing) return;
      showLinks(await api.cloudShare(account.id, sharing, options));
      sharing = null;
    }}
  />
{/if}

<AccountsDialog
  open={managing}
  {accounts}
  onclose={() => (managing = false)}
  onadd={() => (adding = true)}
  onedit={(a) => (editing = a)}
  onremove={removeAccount}
  onreorder={reorderAccounts}
  onimported={importedAccounts}
/>

<AccountDialog
  open={adding || editing !== null}
  account={editing}
  {accounts}
  onclose={() => {
    adding = false;
    editing = null;
  }}
  onsaved={saved}
/>

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

  /* The transfer buttons face each other across the divider. */
  .slot :global(.btn.arrow) {
    justify-content: center;
    width: 36px;
    padding: 0;
  }

  .slot + .slot :global(.btn.arrow) {
    margin-right: 6px;
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
