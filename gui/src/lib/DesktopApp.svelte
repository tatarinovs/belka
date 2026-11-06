<script lang="ts">
  import { onMount, tick } from "svelte";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { ask } from "@tauri-apps/plugin-dialog";
  import { revealItemInDir } from "@tauri-apps/plugin-opener";
  import { api, errorText, newBackupTask, type Account, type BackupTask, type Entry } from "$lib/api";
  import { accounts as accountStore } from "$lib/accounts.svelte";
  import { describe } from "$lib/cloud";
  import { explainError } from "$lib/errors";
  import { remember, stored } from "$lib/storage";
  import { dnd, paneAt } from "$lib/dnd.svelte";
  import { baseName, parentPath } from "$lib/format";
  import { providerDef } from "$lib/providers";
  import { transfers } from "$lib/transfers.svelte";
  import { cloudAddress, desktopListing, location, nativeEntry, restorePanes, tabItem, ROOT_LABEL,
    type Side, type PaneTabs, type Location } from "$lib/desktop-navigation";
  import AccountDialog from "$lib/components/AccountDialog.svelte";
  import AboutDialog from "$lib/components/AboutDialog.svelte";
  import SettingsDialog from "$lib/components/SettingsDialog.svelte";
  import AccountsDialog from "$lib/components/AccountsDialog.svelte";
  import BackupScreen from "$lib/components/BackupScreen.svelte";
  import FilePane, { type PaneOps } from "$lib/components/FilePane.svelte";
  import PreviewDialog from "$lib/components/PreviewDialog.svelte";
  import QuotaMeter from "$lib/components/QuotaMeter.svelte";
  import ShareDialog from "$lib/components/ShareDialog.svelte";
  import ShortcutsDialog from "$lib/components/ShortcutsDialog.svelte";
  import Icon from "$lib/components/Icon.svelte";
  import ContextMenu, { type MenuItem } from "$lib/components/ContextMenu.svelte";
  import TabBar from "$lib/components/TabBar.svelte";
  import TransferPanel from "$lib/components/TransferPanel.svelte";
  import PaneDivider from "$lib/components/PaneDivider.svelte";
  import SavedLocations from "$lib/components/SavedLocations.svelte";
  import type { SavedLocation } from "$lib/locations";

  const sides: Side[] = ["left", "right"];
  const other = (side: Side): Side => side === "left" ? "right" : "left";
  const accounts = $derived(accountStore.list);
  let panes = $state<Record<Side, PaneTabs> | null>(null);
  let paneRefs = $state<Partial<Record<Side, FilePane>>>({});
  const pane = (side: Side) => paneRefs[side];
  let activePane = $state<Side>("left");
  const currentTab = (side: Side) => panes?.[side].tabs.find((t) => t.id === panes?.[side].active);
  const currentLocation = (side: Side) => location(currentTab(side)?.path ?? "");
  function accountAt(side: Side) {
    const source = currentLocation(side);
    return source.kind === "cloud" ? accounts.find((a) => a.id === source.account) ?? null : null;
  }
  async function focusPane(side: Side) {
    activePane = side;
    await tick();
    if (activePane === side && !document.querySelector("dialog[open]")) pane(side)?.focus();
  }
  function navigate(side: Side, path: string) {
    void focusPane(side);
    void pane(side)?.navigate(path);
  }
  function newTab(side: Side, path = "") {
    if (!panes) return;
    const group = panes[side];
    const tab = { id: crypto.randomUUID(), path };
    const at = group.tabs.findIndex((t) => t.id === group.active);
    group.tabs.splice(at + 1, 0, tab);
    group.active = tab.id;
    void focusPane(side);
  }
  function selectTab(side: Side, id: string) {
    if (panes) panes[side].active = id;
    void focusPane(side);
  }
  function closeTab(side: Side, id: string) {
    if (!panes || panes[side].tabs.length < 2) return;
    const group = panes[side];
    const at = group.tabs.findIndex((t) => t.id === id);
    if (at < 0) return;
    group.tabs.splice(at, 1);
    if (group.active === id) group.active = group.tabs[Math.min(at, group.tabs.length - 1)].id;
    void focusPane(side);
  }
  function tabKey(side: Side, action: "new" | "close" | "next" | "prev") {
    if (!panes) return;
    const group = panes[side];
    if (action === "new") newTab(side);
    else if (action === "close") closeTab(side, group.active);
    else {
      const at = group.tabs.findIndex((t) => t.id === group.active);
      selectTab(side, group.tabs[(at + (action === "next" ? 1 : group.tabs.length - 1)) % group.tabs.length].id);
    }
  }
  function reorderTabs(side: Side, ids: string[]) {
    if (panes) panes[side].tabs = ids.map((id) => panes![side].tabs.find((t) => t.id === id)).filter((t) => t !== undefined);
  }
  $effect(() => {
    if (panes) remember("tabs", JSON.stringify({ version: 2, ...panes }));
  });

  let adding = $state(false);
  let editing = $state<Account | null>(null);
  let managing = $state(false);
  let backups = $state(false);
  let backupDraft = $state<BackupTask | null>(null);
  let about = $state(false);
  let settings = $state(false);
  let shortcuts = $state(false);
  let savedLocations = $state(false);
  let notice = $state("");
  const explainedNotice = $derived(explainError(notice));
  let busy = $state("");
  let info = $state("");
  let infoTimer: ReturnType<typeof setTimeout> | undefined;
  let quotaVersion = $state(0);
  let previewing = $state<{ account: Account; side: Side; files: Entry[]; start: number } | null>(null);
  let sharing = $state<{ account: Account; side: Side; items: Entry[] } | null>(null);
  let clipboard = $state<{ source: Exclude<Location, { kind: "root" }>; items: Entry[] } | null>(null);
  let externalOver = $state<Side | null>(null);
  let appMenu = $state<{ x: number; y: number } | null>(null);
  let menuButton: HTMLButtonElement;
  let driveMenu = $state<{ side: Side; x: number; y: number; items: MenuItem[] } | null>(null);
  let menuRequest = 0;

  function showInfo(text: string) {
    info = text;
    clearTimeout(infoTimer);
    infoTimer = setTimeout(() => (info = ""), 10_000);
  }
  function showLinks(links: string[]) {
    showInfo(links.length === 1 ? `Ссылка скопирована: ${links[0]}` : `Ссылки скопированы (${links.length})`);
  }
  function closeAppMenu(restore = false) {
    appMenu = null;
    if (restore) void focusPane(activePane);
  }
  function toggleAppMenu() {
    if (appMenu) { closeAppMenu(true); return; }
    closeDriveMenu();
    const rect = menuButton.getBoundingClientRect();
    appMenu = { x: rect.left, y: rect.bottom + 4 };
  }
  function closeDriveMenu(restore = false) {
    menuRequest++;
    const side = driveMenu?.side;
    driveMenu = null;
    if (restore && side) void focusPane(side);
  }
  async function showDriveMenu(side: Side) {
    const seq = ++menuRequest;
    appMenu = null;
    driveMenu = null;
    await focusPane(side);
    try {
      const root = await desktopListing(accounts, "");
      if (seq !== menuRequest || activePane !== side || document.querySelector("dialog[open]")) return;
      const rect = document.querySelector(`[data-pane="${side}"] .toolbar`)?.getBoundingClientRect();
      if (!rect) return;
      driveMenu = { side, x: rect.left + 4, y: rect.bottom + 4, items: [
        { label: ROOT_LABEL, icon: "computer", action: () => navigate(side, "") }, "separator",
        ...root.entries.map((e): MenuItem => ({ label: e.name, icon: e.icon ?? "folder", action: () => navigate(side, e.path) })),
      ] };
    } catch (err) { notice = errorText(err); }
  }
  function keyboard(event: KeyboardEvent) {
    if (document.querySelector("dialog[open]")) return;
    if (event.altKey && !event.ctrlKey && !event.metaKey && (event.key === "F1" || event.key === "F2")) {
      event.preventDefault();
      void showDriveMenu(event.key === "F1" ? "left" : "right");
    } else if (event.key === "F1" && !event.altKey && !event.ctrlKey && !event.metaKey) {
      event.preventDefault();
      closeDriveMenu();
      shortcuts = true;
    }
  }

  let panesWidth = $state(1280);
  const minPane = $derived(Math.max(25, Math.min(49, (320 / panesWidth) * 100)));
  let paneWidth = $state((() => {
    const saved = Number(stored("desktop.paneWidth") ?? 50);
    return Number.isFinite(saved) ? Math.max(25, Math.min(75, saved)) : 50;
  })());
  function resizePanes(value: number) {
    paneWidth = Math.max(minPane, Math.min(100 - minPane, value));
    remember("desktop.paneWidth", String(paneWidth));
  }
  const shownPaneWidth = $derived(Math.max(minPane, Math.min(100 - minPane, paneWidth)));

  function copyAllowed(source: Location, target: Location) {
    return source.kind !== "root" && target.kind !== "root" && (source.kind === "cloud" || target.kind === "cloud");
  }
  function canCopy(side: Side) {
    return !busy && !pane(side)?.isLoading() && !pane(other(side))?.isLoading() && copyAllowed(currentLocation(side), currentLocation(other(side)));
  }
  async function run(label: string, action: () => Promise<void>) {
    busy = label; notice = "";
    try { await action(); } catch (err) { notice = errorText(err); } finally { busy = ""; }
  }
  function confirmReplace(names: string[]) {
    const list = names.slice(0, 5).map((n) => `• ${n}`).join("\n");
    return ask(`В папке назначения уже есть:\n${list}${names.length > 5 ? `\n…и ещё ${names.length - 5}` : ""}\n\nЗаменить?`, {
      title: "Файлы уже существуют", kind: "warning", okLabel: "Заменить", cancelLabel: "Отмена",
    });
  }
  async function copyItems(source: Location, items: Entry[], targetSide: Side, targetAddress = currentTab(targetSide)?.path ?? "") {
    const target = location(targetAddress);
    if (busy || pane(targetSide)?.isLoading() || !items.length || source.kind === "root" || target.kind === "root") return;
    if (!copyAllowed(source, target)) { notice = "Копирование между локальными папками пока недоступно."; return; }
    if (source.kind === "cloud" && target.kind === "cloud" && source.account === target.account &&
      items.some((e) => parentPath(e.path) === target.path)) {
      notice = "Выбранные файлы уже находятся в папке назначения."; return;
    }
    const destination = target.path;
    const targetEntries = pane(targetSide)?.getEntries() ?? [];
    await run("Подготовка копирования…", async () => {
      const existing = new Set(targetEntries.map((e) => e.name.toLowerCase()));
      const conflicts = target.kind === "local" ? await api.localConflicts(destination, items.map((e) => e.name)) :
        items.map((e) => e.name).filter((name) => existing.has(name.toLowerCase()));
      if (conflicts.length && !(await confirmReplace(conflicts))) return;
      if (source.kind === "cloud" && target.kind === "cloud") await api.cloudCopy(source.account, items, target.account, destination);
      else if (source.kind === "cloud") await api.download(source.account, items, destination);
      else if (target.kind === "cloud") await api.upload(target.account, items.map((e) => e.path), destination);
      transfers.kick();
    });
  }
  function copySelection(side: Side) {
    if (!canCopy(side)) return;
    const items = pane(side)?.getSelection().filter((e) => !e.icon).map(nativeEntry) ?? [];
    void copyItems(currentLocation(side), items, other(side));
  }
  function dragTo(side: Side, target: string) {
    if (target === other(side)) copySelection(side);
  }
  function backup(entry: Entry) {
    const cloudSide = accountAt(activePane) ? activePane : other(activePane);
    const target = currentLocation(cloudSide);
    backupDraft = newBackupTask(entry.path, target.kind === "cloud" ? target.account : accounts[0]?.id ?? "", target.kind === "cloud" ? target.path : "");
    backups = true;
  }
  function opsFor(side: Side): PaneOps | undefined {
    const source = currentLocation(side);
    if (source.kind === "root") return undefined;
    const common = {
      canModify: (folder: string) => location(folder).kind !== "root",
      copyItems: (items: Entry[]) => {
        clipboard = { source, items: items.filter((e) => !e.icon).map(nativeEntry) };
        showInfo(`Скопировано: ${describe(items)}. Откройте папку назначения и нажмите Ctrl+V.`);
      },
      pasteLabel: () => clipboard && copyAllowed(clipboard.source, source) ? `Вставить (${clipboard.items.length})` : null,
      paste: (folder: string) => { if (clipboard) void copyItems(clipboard.source, clipboard.items, side, folder); },
    };
    if (source.kind === "local") return { ...common,
      mkdir: api.localMkdir, rename: (entry, name) => api.localRename(entry.path, name),
      remove: (items) => api.localDelete(items.map((e) => e.path)),
      deletePrompt: (items) => `Переместить ${describe(items)} в корзину?`, deleteLabel: "Удалить", confirmLabel: "В корзину",
      reveal: revealItemInDir, backup,
    };
    const account = accounts.find((a) => a.id === source.account);
    return { ...common,
      mkdir: async (parent, name) => cloudAddress(source.account, await api.cloudMkdir(source.account, (location(parent) as typeof source).path, name)),
      rename: async (entry, name) => cloudAddress(source.account, await api.cloudRename(source.account, nativeEntry(entry).path, name, entry.is_dir)),
      remove: (items) => api.cloudDelete(source.account, items.map(nativeEntry)),
      deletePrompt: (items) => `Удалить ${describe(items)} из облака «${account?.name}»?\n\nФайлы удаляются безвозвратно, минуя корзину облака. Отменить это нельзя.`,
      deleteLabel: "Удалить", confirmLabel: "Удалить навсегда",
      ...(account && providerDef(account.kind).share && {
        share: async (items: Entry[]) => showLinks(await api.cloudShare(source.account, items.map(nativeEntry))),
        unshare: async (items: Entry[]) => { await api.cloudUnshare(source.account, items.map(nativeEntry)); showInfo("Доступ по ссылке закрыт"); },
      }),
      ...(account && providerDef(account.kind).shareOptions && {
        shareWithOptions: (items: Entry[]) => { sharing = { account, side, items: items.map(nativeEntry) }; },
      }),
    };
  }
  function openFile(side: Side, entry: Entry, files: Entry[]) {
    const account = accountAt(side);
    if (account) previewing = { account, side, files: files.map(nativeEntry), start: files.findIndex((e) => e.path === entry.path) };
    else api.localOpen(entry.path).catch((err) => (notice = errorText(err)));
  }
  function refreshRoots() {
    for (const side of sides) if (currentLocation(side).kind === "root") void pane(side)?.reload();
  }
  function savedAccount(account: Account) {
    if (editing) accountStore.saved(account);
    else accountStore.added(account);
    adding = false; editing = null;
    refreshRoots();
    for (const side of sides) if (accountAt(side)?.id === account.id) void pane(side)?.reload();
    void focusPane(activePane);
  }
  async function removeAccount(account: Account) {
    await accountStore.remove(account);
    if (!panes) return;
    for (const side of sides) {
      const affected = accountAt(side) === null && currentLocation(side).kind === "cloud";
      for (const tab of panes[side].tabs) {
        const source = location(tab.path);
        if (source.kind === "cloud" && source.account === account.id) tab.path = "";
      }
      if (affected) void pane(side)?.navigate("");
    }
    refreshRoots();
  }
  async function importedAccounts() { await accountStore.load(); refreshRoots(); }
  const savedCloudSide = $derived(accountAt(activePane) ? activePane : other(activePane));
  const savedCloud = $derived.by(() => {
    const source = currentLocation(savedCloudSide);
    return source.kind === "cloud" ? { account: source.account, path: source.path } : null;
  });
  const savedLocal = $derived.by(() => {
    for (const side of [activePane, other(activePane)]) {
      const source = currentLocation(side);
      if (source.kind === "local") return source.path;
    }
    return "";
  });
  function openSaved(saved: SavedLocation) {
    if (saved.kind !== "local" && !accounts.some((a) => a.id === saved.account)) { notice = "Облако этой закладки больше не подключено."; return; }
    const side = activePane;
    if (saved.kind === "pair") {
      navigate(other(side), saved.local);
      navigate(side, cloudAddress(saved.account, saved.remote));
    } else navigate(side, saved.kind === "cloud" ? cloudAddress(saved.account, saved.remote) : saved.local);
  }

  const reloadTimers: Partial<Record<Side, ReturnType<typeof setTimeout>>> = {};
  function reloadSoon(side: Side) {
    clearTimeout(reloadTimers[side]);
    reloadTimers[side] = setTimeout(() => { void pane(side)?.reload(); quotaVersion++; }, 500);
  }
  function isInside(file: string, dir: string) {
    const norm = (p: string) => p.replace(/[\\/]+$/, "").toLowerCase();
    return norm(file).startsWith(norm(dir) + (dir.includes("\\") ? "\\" : "/"));
  }
  async function externalUpload(side: Side, paths: string[]) {
    const source: Location = { kind: "local", path: "" };
    await copyItems(source, paths.map((path) => ({ name: baseName(path), path, is_dir: false, size: 0, modified: null })), side);
  }
  onMount(() => {
    let mounted = true;
    transfers.kick();
    accountStore.load().catch((err) => { notice = errorText(err); return []; }).then(async (list) => {
      if (!mounted) return;
      panes = restorePanes(stored("tabs"), list);
      await focusPane("left");
      api.appReady().catch((err) => (notice = errorText(err)));
    });
    const offFinished = transfers.onFinished((transfer) => {
      for (const side of sides) {
        const target = currentLocation(side);
        if ((target.kind === "cloud" && transfer.direction !== "download" && target.account === transfer.account_id && target.path === transfer.remote_dir) ||
          (target.kind === "local" && transfer.direction === "download" && isInside(transfer.local_path, target.path))) reloadSoon(side);
      }
    });
    const unlisten = getCurrentWebview().onDragDropEvent(({ payload }) => {
      if (payload.type === "leave") { externalOver = null; return; }
      const scale = window.devicePixelRatio || 1;
      const hit = paneAt(payload.position.x / scale, payload.position.y / scale);
      const target = (hit === "left" || hit === "right") && currentLocation(hit).kind === "cloud" ? hit : null;
      if (payload.type === "drop") {
        externalOver = null;
        if (target && payload.paths.length) void externalUpload(target, payload.paths);
      } else externalOver = target;
    });
    return () => {
      mounted = false;
      clearTimeout(infoTimer);
      for (const side of sides) clearTimeout(reloadTimers[side]);
      offFinished();
      void unlisten.then((fn) => fn());
    };
  });
</script>

<svelte:window onkeydown={keyboard} />

{#snippet transferButton(side: Side)}
  {@const count = pane(side)?.getSelection().filter((e) => !e.icon).length ?? 0}
  <button class="btn primary arrow" onclick={() => copySelection(side)} disabled={!count || !canCopy(side)}
    title={currentLocation(other(side)).kind === "root" ? "Откройте папку назначения в другой панели" : "Копировать в другую панель (F5)"}
    aria-label="Копировать в другую панель">
    <Icon name={side === "left" ? "arrowRight" : "arrowLeft"} size={16} />
    <span>Копировать{count ? ` (${count})` : ""}</span>
  </button>
{/snippet}

<main>
  <header class="appbar">
    <button class="tool" bind:this={menuButton} title="Меню" aria-label="Меню" aria-haspopup="menu" aria-expanded={appMenu !== null}
      onpointerdown={(event) => event.stopPropagation()} onclick={toggleAppMenu}
      onkeydown={(event) => { if (event.key === "ArrowDown") { event.preventDefault(); if (!appMenu) toggleAppMenu(); } }}>
      <Icon name="menu" />
    </button>
    <button class="tool" title="Подключённые облака" onclick={() => (managing = true)}><Icon name="cloud" /> Аккаунты</button>
    <button class="tool" title="Избранные папки и сохранённые пары панелей" onclick={() => (savedLocations = true)}><Icon name="star" /> Избранное</button>
  </header>
  <div class="panes" bind:clientWidth={panesWidth} style:grid-template-columns="minmax(0, {shownPaneWidth}fr) 5px minmax(0, {100 - shownPaneWidth}fr)">
    {#each sides as side}
      {#if side === "right"}<PaneDivider value={shownPaneWidth} min={minPane} max={100 - minPane} onchange={resizePanes} />{/if}
      <div class="slot">
        {#if panes}
          {@const tab = currentTab(side)!}
          {@const source = currentLocation(side)}
          {@const account = accountAt(side)}
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <div onpointerdown={() => (activePane = side)}>
            <TabBar tabs={panes[side].tabs.map((t) => tabItem(t, accounts))} activeId={panes[side].active}
              onselect={(id) => selectTab(side, id)} onclose={(id) => closeTab(side, id)} onnew={() => newTab(side)}
              onreorder={(ids) => reorderTabs(side, ids)} />
          </div>
          {#key tab.id}
            <FilePane bind:this={paneRefs[side]} id={side}
              load={(path) => desktopListing(accounts, path)} initialPath={tab.path}
              rootPath="" rootLabel={ROOT_LABEL} rootIcon="computer" active={activePane === side}
              externalDrop={externalOver === side} dropLabel={canCopy(other(side)) || externalOver === side ? "Копировать в эту папку" : undefined}
              onactivate={() => (activePane = side)} oncopy={() => copySelection(side)} ondragto={(target) => dragTo(side, target)}
              onswitch={() => focusPane(other(side))} onnavigate={(path) => (tab.path = path)}
              onopenaccount={(id) => navigate(side, cloudAddress(id))}
              onopentab={(entry) => newTab(side, entry.opensAccount ? cloudAddress(entry.opensAccount) : entry.path)}
              ontabkey={(action) => tabKey(side, action)} ops={opsFor(side)} copyLabel="Копировать в другую панель" canCopy={canCopy(side)}
              onpreview={account ? (entry, files) => openFile(side, entry, files) : undefined}
              onopenfile={account ? undefined : (entry) => openFile(side, entry, [])}
              displayPath={(path) => { const target = location(path); return target.kind === "cloud" ? target.path : path; }}
              resolvePath={(value, current) => {
                const target = location(current);
                return target.kind === "cloud" && !value.startsWith("cloud://") && !/^(?:[A-Za-z]:|\\\\)/.test(value) ? cloudAddress(target.account, value) : value;
              }}>
              {#snippet leading()}{#if side === "right"}{@render transferButton(side)}{/if}{/snippet}
              {#snippet actions()}{#if side === "left"}{@render transferButton(side)}{/if}{/snippet}
              {#snippet trailing()}{#if account && source.kind === "cloud"}<QuotaMeter {account} path={source.path} version={quotaVersion} />{/if}{/snippet}
            </FilePane>
          {/key}
        {/if}
      </div>
    {/each}
  </div>
  <div class="hints" aria-hidden="true">
    <span><kbd>Alt+F1</kbd> Левый диск</span><span><kbd>Alt+F2</kbd> Правый диск</span>
    <span><kbd>F2</kbd> Переименовать</span><span><kbd>F5</kbd> Копировать</span><span><kbd>F7</kbd> Новая папка</span>
    <span><kbd>Del</kbd> Удалить</span><span><kbd>Tab</kbd> Другая панель</span><span><kbd>F1</kbd> Все клавиши</span>
  </div>
  {#if notice || busy || info}
    <div class="notice" class:error={!!notice} role={notice ? "alert" : "status"}>
      <span>{notice ? explainedNotice.summary : busy || info}
        {#if notice && explainedNotice.detail}<details><summary>Подробнее</summary>{notice}</details>{/if}
      </span>
      {#if notice || info}<button class="icon-btn" title="Скрыть" aria-label="Скрыть сообщение" onclick={() => (notice = info = "")}><Icon name="close" size={14} /></button>{/if}
    </div>
  {/if}
  <TransferPanel />
</main>

{#if appMenu}
  <ContextMenu x={appMenu.x} y={appMenu.y} onclose={closeAppMenu} items={[
    { label: "Настройки", icon: "settings", action: () => (settings = true) },
    { label: "Автобэкап", icon: "clock", action: () => (backups = true) },
    { label: "О приложении", icon: "info", action: () => (about = true) },
  ]} />
{/if}
{#if driveMenu}<ContextMenu x={driveMenu.x} y={driveMenu.y} items={driveMenu.items} onclose={closeDriveMenu} />{/if}
{#if dnd.from}
  <div class="ghost" style:left="{dnd.x + 14}px" style:top="{dnd.y + 14}px"><Icon name="copy" size={14} />{dnd.count === 1 ? "1 объект" : `${dnd.count} объектов`}</div>
{/if}
{#if previewing}
  <PreviewDialog account={previewing.account} files={previewing.files} start={previewing.start}
    onclose={() => { const side = previewing!.side; previewing = null; void focusPane(side); }} />
{/if}
<ShareDialog entries={sharing?.items ?? null} onclose={() => { const side = sharing?.side ?? activePane; sharing = null; void focusPane(side); }}
  onshare={async (options) => { if (!sharing) return; showLinks(await api.cloudShare(sharing.account.id, sharing.items, options)); const side = sharing.side; sharing = null; void focusPane(side); }} />
{#if backups}
  <BackupScreen {accounts} storageOk={true} bind:draft={backupDraft} desktop
    currentCloud={savedCloud ? { accountId: savedCloud.account, path: savedCloud.path } : null}
    onclose={() => { backups = false; backupDraft = null; void focusPane(activePane); }} onstorage={() => {}}
    ontoast={(text, error) => (error ? (notice = text) : showInfo(text))} />
{/if}
<AccountsDialog open={managing} {accounts} onclose={() => { managing = false; void focusPane(activePane); }}
  onadd={() => (adding = true)} onedit={(a) => (editing = a)} onremove={removeAccount}
  onreorder={async (ids) => { await accountStore.reorder(ids); refreshRoots(); }} onimported={importedAccounts} />
{#if settings}<SettingsDialog onclose={() => { settings = false; void focusPane(activePane); }} />{/if}
<AboutDialog open={about} onclose={() => { about = false; void focusPane(activePane); }} />
<SavedLocations open={savedLocations} {accounts} cloud={savedCloud} local={savedLocal}
  onclose={() => { savedLocations = false; void focusPane(activePane); }} onopen={openSaved} />
<ShortcutsDialog open={shortcuts} onclose={() => { shortcuts = false; void focusPane(activePane); }} />
<AccountDialog open={adding || editing !== null} account={editing} {accounts}
  onclose={() => { adding = false; editing = null; void focusPane(activePane); }} onsaved={savedAccount} />

<style>
  main {
    display: flex;
    flex-direction: column;
    height: 100vh;
  }

  /* The app's own tools, above both panes. */
  .appbar {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 4px 8px;
    border-bottom: 1px solid var(--border);
    background: var(--sidebar);
  }

  .tool {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 28px;
    padding: 0 10px;
    border: 0;
    border-radius: 6px;
    background: none;
  }

  .tool:hover {
    background: var(--hover);
  }

  .tool :global(svg) {
    color: var(--muted);
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
    min-width: 36px;
    padding: 0 10px;
    white-space: nowrap;
  }

  .slot:last-child :global(.btn.arrow) {
    margin-right: 6px;
  }

  @media (max-width: 1100px) {
    .slot :global(.btn.arrow span) { display: none; }
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

  .notice details {
    margin-top: 2px;
    font-size: 12px;
    white-space: pre-wrap;
    word-break: break-word;
  }

  .notice summary {
    cursor: pointer;
  }

  /* Keys that work in the panes, always in sight. */
  .hints {
    display: flex;
    flex-wrap: wrap;
    gap: 2px 16px;
    padding: 3px 12px;
    border-top: 1px solid var(--border);
    background: var(--sidebar);
    color: var(--muted);
    font-size: 12px;
  }

  .hints kbd {
    margin-right: 4px;
    padding: 0 5px;
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--surface);
    color: var(--text);
    font: inherit;
    font-size: 12px;
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
