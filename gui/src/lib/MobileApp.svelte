<!--
  The phone layout: one pane that shows either a cloud or the phone's storage.
  Files are selected with a long press; "Скачать" and "Загрузить" then switch
  to the other side, where the user opens the target folder and confirms.
-->
<script lang="ts">
  import { onMount, tick } from "svelte";
  import { SvelteSet } from "svelte/reactivity";
  import { getVersion } from "@tauri-apps/api/app";
  import { ask } from "@tauri-apps/plugin-dialog";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { api, errorText, type Account, type Entry, type Listing, type Update } from "$lib/api";
  import { baseName, formatBytes, formatDate, parentPath, plural, splitPath } from "$lib/format";
  import { ANDROID_DOWNLOADS } from "$lib/platform";
  import { providerDef } from "$lib/providers";
  import { transfers } from "$lib/transfers.svelte";
  import AccountDialog from "$lib/components/AccountDialog.svelte";
  import AccountsDialog from "$lib/components/AccountsDialog.svelte";
  import Icon, { type IconName } from "$lib/components/Icon.svelte";
  import PreviewDialog from "$lib/components/PreviewDialog.svelte";
  import ShareDialog from "$lib/components/ShareDialog.svelte";
  import TransferPanel from "$lib/components/TransferPanel.svelte";

  type Mode = "cloud" | "local";
  type SortKey = "name" | "modified" | "size";
  /** Choosing the folder that the selected items go to. */
  type Pick =
    | { kind: "download"; account: Account; items: Entry[] }
    | { kind: "upload"; paths: string[] };
  type SheetItem =
    | {
        label: string;
        icon?: IconName;
        badge?: Account;
        checked?: boolean;
        danger?: boolean;
        disabled?: boolean;
        action: () => void;
      }
    | "separator";

  const UPDATE_CHECK_MS = 24 * 60 * 60 * 1000;

  function stored(key: string): string | null {
    try {
      return localStorage.getItem(`m.${key}`);
    } catch {
      return null;
    }
  }

  function remember(key: string, value: string) {
    try {
      localStorage.setItem(`m.${key}`, value);
    } catch {
      // Remembering is a convenience only.
    }
  }

  let accounts = $state<Account[]>([]);
  let loaded = $state(false);
  let mode = $state<Mode>(stored("mode") === "local" ? "local" : "cloud");
  let accountId = $state<string | null>(stored("account"));
  /** The last folder of every account. */
  const cloudPaths: Record<string, string> = (() => {
    try {
      return JSON.parse(stored("cloudPaths") ?? "{}");
    } catch {
      return {};
    }
  })();
  let localPath = stored("localPath") ?? ANDROID_DOWNLOADS;
  let sortKey = $state<SortKey>((stored("sort") as SortKey | null) ?? "name");

  let listing = $state<Listing | null>(null);
  let loading = $state(false);
  let error = $state("");
  const selected = new SvelteSet<string>();
  let pick = $state<Pick | null>(null);
  let busy = $state("");

  let sheet = $state<{ title?: string; items: SheetItem[] } | null>(null);
  let prompt = $state<{ title: string; value: string; okLabel: string; resolve: (v: string | null) => void } | null>(
    null,
  );
  let adding = $state(false);
  let editing = $state<Account | null>(null);
  let managing = $state(false);
  let previewing = $state<{ files: Entry[]; start: number } | null>(null);
  let sharing = $state<Entry[] | null>(null);

  let toast = $state<{ text: string; error: boolean } | null>(null);
  let toastTimer: ReturnType<typeof setTimeout> | undefined;
  let storageOk = $state(true);
  let update = $state<Update | null>(null);
  let version = $state("");

  const account = $derived(accounts.find((a) => a.id === accountId) ?? null);
  const path = $derived(listing?.path ?? "");
  const atRoot = $derived(listing?.parent == null);
  const entries = $derived(listing?.entries ?? []);
  const collator = new Intl.Collator("ru", { numeric: true, sensitivity: "base" });

  const sorted = $derived.by(() => {
    const time = (e: Entry) => (e.modified ? Date.parse(e.modified) : 0);
    return [...entries].sort((a, b) => {
      // The phone's root keeps its own order (folders, then drives).
      if (a.icon || b.icon) return 0;
      if (a.is_dir !== b.is_dir) return a.is_dir ? -1 : 1;
      // Newest and largest first: that is what one looks for on a phone.
      const result =
        sortKey === "modified" ? time(b) - time(a) : sortKey === "size" ? b.size - a.size : 0;
      return result || collator.compare(a.name, b.name);
    });
  });
  const selectedEntries = $derived(sorted.filter((e) => selected.has(e.path)));
  const selecting = $derived(selected.size > 0);

  const title = $derived.by(() => {
    if (mode === "cloud") return atRoot ? (account?.name ?? "Облако") : baseName(path);
    return path ? baseName(path) : "Телефон";
  });
  const subtitle = $derived.by(() => {
    if (mode === "cloud") return account && !atRoot ? `${account.name}: ${path}` : "";
    return path;
  });

  function showToast(text: string, isError = false) {
    toast = { text, error: isError };
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => (toast = null), isError ? 8000 : 4000);
  }

  function fail(err: unknown) {
    showToast(errorText(err), true);
  }

  function describe(items: Entry[]) {
    return items.length === 1 ? `«${items[0].name}»` : `выбранные объекты (${items.length})`;
  }

  // ---- listings ----

  async function cloudListing(acc: Account, target: string): Promise<Listing> {
    const list = markEncrypted(acc, await api.listDir(acc.id, target));
    return { path: target, parent: target === "/" ? null : parentPath(target), crumbs: splitPath(target), entries: list };
  }

  /** Marks what is stored encrypted, as the desktop layout does. */
  function markEncrypted(acc: Account, list: Entry[]): Entry[] {
    if (acc.kind === "crypt") return list.map((e) => ({ ...e, locked: "Хранится зашифрованным" }));
    const folders = accounts
      .filter((a) => a.crypt?.base === acc.id)
      .map((a) => ({ account: a, root: "/" + a.crypt!.root.replace(/^\/+|\/+$/g, "") }));
    if (!folders.length) return list;
    return list.map((e) => {
      for (const { account: folder, root } of folders) {
        if (e.path === root) return { ...e, locked: `Зашифрованная папка «${folder.name}»`, opensAccount: folder.id };
        if (root === "/" || e.path.startsWith(root + "/")) return { ...e, locked: "Зашифрованные данные" };
      }
      return e;
    });
  }

  let requestSeq = 0;

  async function navigate(target: string) {
    const seq = ++requestSeq;
    const forMode = mode;
    const acc = account;
    loading = true;
    error = "";
    try {
      if (forMode === "cloud" && !acc) return;
      const result = forMode === "cloud" ? await cloudListing(acc!, target) : await api.listLocal(target);
      if (seq !== requestSeq) return;
      if (result.path !== listing?.path) selected.clear();
      listing = result;
      if (forMode === "cloud") {
        cloudPaths[acc!.id] = result.path;
        remember("cloudPaths", JSON.stringify(cloudPaths));
      } else {
        localPath = result.path;
        remember("localPath", localPath);
      }
    } catch (err) {
      if (seq !== requestSeq) return;
      error = errorText(err);
      listing ??= { path: target, parent: null, crumbs: [], entries: [] };
    } finally {
      if (seq === requestSeq) loading = false;
    }
  }

  function reload() {
    return navigate(path);
  }

  /** Switches the pane to the cloud or the phone, at the last folder there. */
  function show(next: Mode, target?: string) {
    mode = next;
    remember("mode", next);
    listing = null;
    error = "";
    selected.clear();
    if (next === "cloud") {
      if (account) navigate(target ?? cloudPaths[account.id] ?? "/");
      else loading = false;
    } else if (storageOk) {
      navigate(target ?? localPath);
    }
  }

  function selectAccount(acc: Account, target?: string) {
    accountId = acc.id;
    remember("account", acc.id);
    show("cloud", target);
  }

  function goUp() {
    if (listing?.parent != null) navigate(listing.parent);
  }

  // ---- taps ----

  /** Android WebView fires `contextmenu` on a long press; the tap after it is dropped. */
  let longPressAt = 0;

  function longPress(event: Event, entry: Entry) {
    event.preventDefault();
    if (pick || entry.icon) return;
    longPressAt = Date.now();
    navigator.vibrate?.(15);
    toggle(entry);
  }

  function toggle(entry: Entry) {
    if (selected.has(entry.path)) selected.delete(entry.path);
    else selected.add(entry.path);
  }

  function tap(entry: Entry) {
    if (Date.now() - longPressAt < 600) return;
    if (selecting && !pick) {
      if (!entry.icon) toggle(entry);
      return;
    }
    if (entry.opensAccount && !pick) {
      const folder = accounts.find((a) => a.id === entry.opensAccount);
      if (folder) selectAccount(folder);
    } else if (entry.is_dir) {
      navigate(entry.path);
    } else if (pick) {
      return;
    } else if (mode === "cloud") {
      const files = sorted.filter((e) => !e.is_dir);
      previewing = { files, start: files.indexOf(entry) };
    } else {
      api.localOpen(entry.path).catch(fail);
    }
  }

  // ---- transfers ----

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

  function startDownload() {
    if (!account || !selectedEntries.length) return;
    pick = { kind: "download", account, items: selectedEntries };
    show("local", localPath || ANDROID_DOWNLOADS);
  }

  function startUpload() {
    const items = selectedEntries.filter((e) => !e.icon);
    if (!items.length) return;
    if (!accounts.length) {
      showToast("Сначала подключите облако");
      adding = true;
      return;
    }
    pick = { kind: "upload", paths: items.map((e) => e.path) };
    if (!account) accountId = accounts[0].id;
    show("cloud");
  }

  function cancelPick() {
    if (!pick) return;
    const back: Mode = pick.kind === "download" ? "cloud" : "local";
    if (pick.kind === "download") accountId = pick.account.id;
    pick = null;
    show(back);
  }

  async function confirmPick() {
    if (!pick || !path || busy) return;
    const current = pick;
    const dir = path;
    busy = current.kind === "download" ? "Подготовка скачивания…" : "Подготовка загрузки…";
    try {
      if (current.kind === "download") {
        const conflicts = await api.localConflicts(dir, current.items.map((e) => e.name));
        if (conflicts.length && !(await confirmReplace(conflicts, "В этой папке"))) return;
        const count = await api.download(current.account.id, current.items, dir);
        showToast(`Скачивание: ${plural(count, "файл", "файла", "файлов")}`);
      } else {
        if (!account) return;
        const existing = new Set(entries.map((e) => e.name.toLowerCase()));
        const conflicts = current.paths.map(baseName).filter((n) => existing.has(n.toLowerCase()));
        if (conflicts.length && !(await confirmReplace(conflicts, "В этой папке облака"))) return;
        const count = await api.upload(account.id, current.paths, dir);
        showToast(`Загрузка в облако: ${plural(count, "файл", "файла", "файлов")}`);
      }
      transfers.kick();
      cancelPick();
    } catch (err) {
      fail(err);
    } finally {
      busy = "";
    }
  }

  // ---- file operations ----

  const canModify = $derived(mode === "cloud" ? !!account : !!path);

  function askName(title: string, value: string, okLabel: string): Promise<string | null> {
    return new Promise((resolve) => (prompt = { title, value, okLabel, resolve }));
  }

  function closePrompt(value: string | null) {
    prompt?.resolve(value);
    prompt = null;
  }

  async function operation(label: string, action: () => Promise<void>) {
    busy = label;
    try {
      await action();
    } catch (err) {
      fail(err);
    } finally {
      busy = "";
    }
  }

  async function newFolder() {
    if (!canModify) return;
    const name = await askName("Новая папка", "Новая папка", "Создать");
    if (!name?.trim()) return;
    const parent = path;
    await operation("Создание папки…", async () => {
      if (mode === "cloud") await api.cloudMkdir(account!.id, parent, name);
      else await api.localMkdir(parent, name);
      await reload();
    });
  }

  async function rename(entry: Entry) {
    const name = await askName("Переименовать", entry.name, "Сохранить");
    if (!name?.trim() || name.trim() === entry.name) return;
    await operation("Переименование…", async () => {
      if (mode === "cloud") await api.cloudRename(account!.id, entry.path, name, entry.is_dir);
      else await api.localRename(entry.path, name);
      selected.clear();
      await reload();
    });
  }

  async function remove(items: Entry[]) {
    const where = mode === "cloud" ? `из облака «${account?.name}»` : "с телефона";
    const confirmed = await ask(`Удалить ${describe(items)} ${where}?\n\nФайлы удаляются безвозвратно.`, {
      title: "Удаление",
      kind: "warning",
      okLabel: "Удалить",
      cancelLabel: "Отмена",
    });
    if (!confirmed) return;
    await operation("Удаление…", async () => {
      if (mode === "cloud") await api.cloudDelete(account!.id, items);
      else await api.localDelete(items.map((e) => e.path));
      selected.clear();
      await reload();
    });
  }

  async function share(items: Entry[]) {
    if (!account) return;
    const acc = account;
    await operation("Создание ссылки…", async () => {
      const links = await api.cloudShare(acc.id, items);
      selected.clear();
      showToast(links.length === 1 ? `Ссылка скопирована: ${links[0]}` : `Ссылки скопированы (${links.length})`);
    });
  }

  // ---- menus ----

  function openMenu() {
    const items: SheetItem[] = [
      { label: "Новая папка", icon: "plus", disabled: !canModify, action: newFolder },
      { label: "Обновить", icon: "refresh", action: reload },
      "separator",
      { label: "По имени", icon: "sort", checked: sortKey === "name", action: () => setSort("name") },
      { label: "Сначала новые", checked: sortKey === "modified", action: () => setSort("modified") },
      { label: "Сначала большие", checked: sortKey === "size", action: () => setSort("size") },
      "separator",
      { label: "Аккаунты", icon: "settings", action: () => (managing = true) },
    ];
    if (!storageOk) {
      items.push({ label: "Разрешить доступ к файлам", icon: "lock", action: requestStorage });
    }
    items.push({ label: "Проверить обновления", icon: "download", action: () => checkUpdate(true) });
    sheet = { title: version ? `Belka ${version}` : "Belka", items };
  }

  function setSort(key: SortKey) {
    sortKey = key;
    remember("sort", key);
  }

  function openAccounts() {
    const items: SheetItem[] = accounts.map((a) => ({
      label: a.name,
      badge: a,
      checked: a.id === accountId,
      action: () => selectAccount(a),
    }));
    items.push("separator");
    items.push({ label: "Добавить аккаунт", icon: "plus", action: () => (adding = true) });
    items.push({ label: "Управление аккаунтами", icon: "settings", action: () => (managing = true) });
    sheet = { title: "Облака", items };
  }

  function openSelectionMenu() {
    const items = selectedEntries.filter((e) => !e.icon);
    const single = items.length === 1 ? items[0] : null;
    const def = account ? providerDef(account.kind) : null;
    const list: SheetItem[] = [];
    if (single && !single.is_dir && mode === "cloud") {
      list.push({ label: "Просмотр", icon: "eye", action: () => tap(single) });
    }
    if (single && !single.is_dir && mode === "local") {
      list.push({ label: "Открыть", icon: "file", action: () => api.localOpen(single.path).catch(fail) });
    }
    list.push({ label: "Переименовать", disabled: !single, action: () => single && rename(single) });
    if (mode === "cloud" && def?.share) {
      if (def.shareOptions) list.push({ label: "Ссылка с паролем или сроком", icon: "link", action: () => (sharing = items) });
      list.push({
        label: "Закрыть доступ по ссылке",
        action: () =>
          operation("Закрытие доступа…", async () => {
            await api.cloudUnshare(account!.id, items);
            selected.clear();
            showToast(`Доступ по ссылке закрыт: ${describe(items)}`);
          }),
      });
    }
    list.push("separator");
    list.push({ label: "Удалить", icon: "trash", danger: true, action: () => remove(items) });
    sheet = { items: list };
  }

  function choose(item: SheetItem) {
    if (item === "separator" || item.disabled) return;
    sheet = null;
    item.action();
  }

  function selectAll() {
    const all = sorted.filter((e) => !e.icon);
    if (selected.size === all.length) selected.clear();
    else for (const e of all) selected.add(e.path);
  }

  // ---- storage access and updates ----

  async function checkStorage() {
    const was = storageOk;
    storageOk = await api.storageAccess().catch(() => false);
    if (storageOk && !was && mode === "local") show("local");
  }

  async function requestStorage() {
    await api.requestStorageAccess().catch(fail);
  }

  async function checkUpdate(manual = false) {
    try {
      remember("updateChecked", String(Date.now()));
      update = await api.checkUpdate();
      if (manual && !update) showToast("Установлена последняя версия");
    } catch (err) {
      if (manual) fail(err);
    }
  }

  // ---- account changes ----

  function added(acc: Account) {
    accounts = [...accounts, acc];
    adding = false;
    editing = null;
    if (pick?.kind === "upload") {
      accountId = acc.id;
      show("cloud", "/");
    } else {
      selectAccount(acc, "/");
    }
  }

  function saved(acc: Account) {
    if (!editing) {
      added(acc);
      return;
    }
    accounts = accounts.map((a) => (a.id === acc.id ? acc : a));
    editing = null;
    if (mode === "cloud" && accountId === acc.id) reload();
  }

  async function removeAccount(acc: Account) {
    await api.removeAccount(acc.id);
    accounts = accounts.filter((a) => a.id !== acc.id);
    delete cloudPaths[acc.id];
    if (accountId === acc.id) {
      accountId = accounts[0]?.id ?? null;
      if (mode === "cloud") show("cloud");
    }
  }

  async function reorderAccounts(ids: string[]) {
    const before = accounts;
    accounts = ids.map((id) => before.find((a) => a.id === id)).filter((a) => a !== undefined);
    try {
      await api.reorderAccounts(ids);
    } catch (err) {
      accounts = before;
      throw err;
    }
  }

  async function importedAccounts() {
    accounts = await api.listAccounts();
    if (!account && accounts[0]) selectAccount(accounts[0]);
  }

  // ---- the system Back button ----

  /** Undoes the innermost thing: a menu, a dialog, the selection, a folder. */
  function back(): boolean {
    if (prompt) closePrompt(null);
    else if (sheet) sheet = null;
    else if (previewing) previewing = null;
    else if (sharing) sharing = null;
    else if (adding || editing) {
      adding = false;
      editing = null;
    } else if (managing) managing = false;
    else if (selecting) selected.clear();
    else if (pick) cancelPick();
    else if (!atRoot) goUp();
    else return false;
    return true;
  }

  let exitArmed = false;

  function onpopstate() {
    if (back()) {
      history.pushState({ belka: true }, "");
    } else if (!exitArmed) {
      // The next Back leaves the app: there is no history left to go back to.
      exitArmed = true;
      showToast("Нажмите «Назад» ещё раз, чтобы выйти");
      setTimeout(() => {
        if (exitArmed) {
          exitArmed = false;
          history.pushState({ belka: true }, "");
        }
      }, 2000);
    }
  }

  onMount(() => {
    history.pushState({ belka: true }, "");
    transfers.kick();
    getVersion().then((v) => (version = v));
    checkStorage();
    const last = Number(stored("updateChecked") ?? 0);
    if (Date.now() - last > UPDATE_CHECK_MS) checkUpdate();

    api
      .listAccounts()
      .then((list) => {
        accounts = list;
        if (!list.some((a) => a.id === accountId)) accountId = list[0]?.id ?? null;
      })
      .catch(fail)
      .finally(async () => {
        loaded = true;
        await tick();
        show(mode);
      });

    const offFinished = transfers.onFinished((t) => {
      if (mode === "cloud" && t.direction === "upload" && t.account_id === accountId && t.remote_dir === path) {
        reloadSoon();
      } else if (mode === "local" && t.direction === "download" && path && t.local_path.startsWith(path + "/")) {
        reloadSoon();
      }
    });

    // Back from the system settings, where storage access may have been granted.
    const onvisible = () => {
      if (document.visibilityState === "visible") {
        checkStorage();
        transfers.kick();
      }
    };
    document.addEventListener("visibilitychange", onvisible);
    return () => {
      offFinished();
      document.removeEventListener("visibilitychange", onvisible);
    };
  });

  let reloadTimer: ReturnType<typeof setTimeout> | undefined;

  function reloadSoon() {
    clearTimeout(reloadTimer);
    reloadTimer = setTimeout(() => !busy && !selecting && reload(), 600);
  }

  function iconFor(entry: Entry): IconName {
    if (entry.icon) return entry.icon;
    return entry.is_dir ? "folder" : "file";
  }

  function meta(entry: Entry) {
    if (entry.icon) return entry.path;
    const date = formatDate(entry.modified);
    if (entry.is_dir) return date;
    return date ? `${formatBytes(entry.size)} · ${date}` : formatBytes(entry.size);
  }

  function focusPrompt(input: HTMLInputElement) {
    input.focus();
    const dot = input.value.lastIndexOf(".");
    input.setSelectionRange(0, dot > 0 ? dot : input.value.length);
  }
</script>

<svelte:window {onpopstate} />

<main>
  <header class="bar">
    {#if selecting && !pick}
      <button class="tool" aria-label="Снять выделение" onclick={() => selected.clear()}>
        <Icon name="close" size={22} />
      </button>
      <div class="title"><strong>Выбрано: {selected.size}</strong></div>
      <button class="tool" aria-label="Выбрать все" onclick={selectAll}>
        <Icon name="checkAll" size={22} />
      </button>
    {:else}
      {#if !atRoot}
        <button class="tool" aria-label="Вверх" onclick={goUp}>
          <Icon name="arrowLeft" size={22} />
        </button>
      {/if}
      <div class="title" class:padded={atRoot}>
        <strong>{title}</strong>
        {#if subtitle}<small dir="rtl"><bdi>{subtitle}</bdi></small>{/if}
      </div>
      <button class="tool" aria-label="Обновить" onclick={reload} disabled={mode === "cloud" && !account}>
        <Icon name="refresh" size={20} />
      </button>
      <button class="tool" aria-label="Меню" onclick={openMenu}>
        <Icon name="more" size={22} />
      </button>
    {/if}
  </header>

  {#if !pick || pick.kind === "upload"}
    <nav class="tabs">
      <button
        class:active={mode === "cloud"}
        onclick={() => (mode === "cloud" && accounts.length ? openAccounts() : show("cloud"))}
      >
        {#if account}
          <span class="badge" data-kind={account.kind}>{providerDef(account.kind).short}</span>
          <span class="label">{account.name}</span>
          <Icon name="chevronDown" size={14} />
        {:else}
          <Icon name="cloud" size={18} />
          <span class="label">Облако</span>
        {/if}
      </button>
      <button class:active={mode === "local"} disabled={!!pick} onclick={() => show("local")}>
        <Icon name="phone" size={18} />
        <span class="label">Телефон</span>
      </button>
    </nav>
  {/if}

  {#if pick}
    <div class="pick">
      <Icon name={pick.kind === "download" ? "download" : "upload"} size={18} />
      <span>
        {#if pick.kind === "download"}
          Откройте папку на телефоне, куда скачать {pick.items.length === 1 ? `«${pick.items[0].name}»` : `объекты (${pick.items.length})`}
        {:else}
          Откройте папку в облаке, куда загрузить {pick.paths.length === 1 ? `«${baseName(pick.paths[0])}»` : `объекты (${pick.paths.length})`}
        {/if}
      </span>
    </div>
  {/if}

  {#if update}
    <div class="update">
      <span>Доступна версия {update.version}</span>
      <button class="btn primary small" onclick={() => openUrl(update!.apk ?? update!.page).catch(fail)}>Скачать</button>
      <button class="tool small" aria-label="Скрыть" onclick={() => (update = null)}>
        <Icon name="close" size={16} />
      </button>
    </div>
  {/if}

  <div class="progress" class:on={loading || !!busy}></div>

  <div class="list" class:dim={loading}>
    {#if mode === "cloud" && !account}
      {#if loaded}
        <div class="empty">
          <Icon name="cloud" size={48} />
          <h1>Belka</h1>
          <p>Подключите облако, чтобы скачивать и загружать файлы.</p>
          <button class="btn primary big" onclick={() => (adding = true)}>Добавить аккаунт</button>
          <button class="btn big" onclick={() => (managing = true)}>Импорт аккаунтов из файла</button>
        </div>
      {/if}
    {:else if mode === "local" && !storageOk}
      <div class="empty">
        <Icon name="lock" size={48} />
        <h1>Нужен доступ к файлам</h1>
        <p>Чтобы показывать папки телефона и сохранять в них файлы, разрешите Belka доступ ко всем файлам.</p>
        <button class="btn primary big" onclick={requestStorage}>Разрешить</button>
      </div>
    {:else}
      {#if error}
        <div class="error banner">{error}</div>
      {/if}
      {#each sorted as entry, i (`${i}:${entry.path}`)}
        {@const isSelected = selected.has(entry.path)}
        <button
          class="row"
          class:selected={isSelected}
          class:muted={!!pick && !entry.is_dir}
          onclick={() => tap(entry)}
          oncontextmenu={(e) => longPress(e, entry)}
        >
          <span class="ico" data-kind={entry.icon ?? (entry.is_dir ? "dir" : "file")}>
            {#if isSelected}
              <span class="check"><Icon name="check" size={18} /></span>
            {:else}
              <Icon name={iconFor(entry)} size={26} />
              {#if entry.locked}<span class="lock"><Icon name="lock" size={11} /></span>{/if}
            {/if}
          </span>
          <span class="text">
            <span class="name">{entry.name}</span>
            {#if meta(entry)}<span class="meta">{meta(entry)}</span>{/if}
          </span>
        </button>
      {:else}
        {#if !loading && !error && listing}
          <div class="placeholder">Папка пуста</div>
        {/if}
      {/each}
    {/if}
  </div>

  {#if toast}
    <div class="toast" class:error={toast.error} role="status">
      <span>{toast.text}</span>
      <button class="tool small" aria-label="Скрыть" onclick={() => (toast = null)}>
        <Icon name="close" size={16} />
      </button>
    </div>
  {/if}

  {#if pick}
    <footer class="actions">
      <button class="btn big" onclick={cancelPick}>Отмена</button>
      <button
        class="btn primary big grow"
        onclick={confirmPick}
        disabled={!!busy || loading || !path || (mode === "local" && !storageOk) || (mode === "cloud" && !account)}
      >
        <Icon name={pick.kind === "download" ? "download" : "upload"} size={18} />
        {pick.kind === "download" ? "Скачать сюда" : "Загрузить сюда"}
      </button>
    </footer>
  {:else if selecting}
    <footer class="actions">
      {#if mode === "cloud"}
        <button class="btn primary big grow" onclick={startDownload} disabled={!!busy}>
          <Icon name="download" size={18} />
          Скачать
        </button>
        {#if account && providerDef(account.kind).share}
          <button class="btn big" aria-label="Поделиться ссылкой" onclick={() => share(selectedEntries)} disabled={!!busy}>
            <Icon name="link" size={18} />
          </button>
        {/if}
      {:else}
        <button
          class="btn primary big grow"
          onclick={startUpload}
          disabled={!!busy || selectedEntries.every((e) => e.icon)}
        >
          <Icon name="upload" size={18} />
          Загрузить в облако
        </button>
      {/if}
      <button class="btn big" aria-label="Ещё" onclick={openSelectionMenu} disabled={!!busy}>
        <Icon name="more" size={18} />
      </button>
    </footer>
  {/if}

  <TransferPanel />
</main>

{#if sheet}
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="scrim" onclick={() => (sheet = null)}>
    <div class="sheet" role="menu" tabindex="-1" onclick={(e) => e.stopPropagation()}>
      {#if sheet.title}<div class="sheet-title">{sheet.title}</div>{/if}
      {#each sheet.items as item, i (i)}
        {#if item === "separator"}
          <hr />
        {:else}
          <button
            class="sheet-item"
            class:danger={item.danger}
            role="menuitem"
            disabled={item.disabled}
            onclick={() => choose(item)}
          >
            <span class="sheet-ico">
              {#if item.badge}
                <span class="badge" data-kind={item.badge.kind}>{providerDef(item.badge.kind).short}</span>
              {:else if item.icon}
                <Icon name={item.icon} size={20} />
              {/if}
            </span>
            <span class="grow">{item.label}</span>
            {#if item.checked}<Icon name="check" size={20} />{/if}
          </button>
        {/if}
      {/each}
    </div>
  </div>
{/if}

{#if prompt}
  {@const p = prompt}
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="scrim center" onclick={() => closePrompt(null)}>
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <form
      class="prompt"
      onclick={(e) => e.stopPropagation()}
      onsubmit={(e) => {
        e.preventDefault();
        closePrompt(p.value);
      }}
    >
      <h2>{p.title}</h2>
      <input bind:value={p.value} use:focusPrompt spellcheck="false" autocomplete="off" />
      <div class="prompt-actions">
        <button type="button" class="btn big" onclick={() => closePrompt(null)}>Отмена</button>
        <button type="submit" class="btn primary big" disabled={!p.value.trim()}>{p.okLabel}</button>
      </div>
    </form>
  </div>
{/if}

{#if previewing && account}
  <PreviewDialog account={account} files={previewing.files} start={previewing.start} onclose={() => (previewing = null)} />
{/if}

{#if account}
  {@const acc = account}
  <ShareDialog
    entries={sharing}
    onclose={() => (sharing = null)}
    onshare={async (options) => {
      if (!sharing) return;
      const links = await api.cloudShare(acc.id, sharing, options);
      sharing = null;
      selected.clear();
      showToast(links.length === 1 ? `Ссылка скопирована: ${links[0]}` : `Ссылки скопированы (${links.length})`);
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
    height: 100dvh;
    background: var(--surface);
    font-size: 15px;
  }

  .bar {
    display: flex;
    align-items: center;
    gap: 4px;
    min-height: 56px;
    padding: 4px 4px;
  }

  .title {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .title.padded {
    padding-left: 12px;
  }

  .title strong {
    overflow: hidden;
    font-size: 19px;
    font-weight: 600;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* Long paths are cut at the start, so the end of the path stays visible. */
  .title small {
    overflow: hidden;
    color: var(--muted);
    font-size: 12px;
    text-align: left;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .tool {
    flex: none;
    display: grid;
    place-items: center;
    width: 44px;
    height: 44px;
    padding: 0;
    border: 0;
    border-radius: 50%;
    background: none;
    color: var(--text);
  }

  .tool.small {
    width: 32px;
    height: 32px;
  }

  .tool:active:not(:disabled) {
    background: var(--hover);
  }

  .tool:disabled {
    opacity: 0.35;
  }

  .tabs {
    display: flex;
    gap: 8px;
    padding: 0 12px 8px;
  }

  .tabs button {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    min-width: 0;
    height: 40px;
    padding: 0 12px;
    border: 1px solid var(--border);
    border-radius: 20px;
    background: var(--surface);
    color: var(--muted);
  }

  .tabs button.active {
    border-color: transparent;
    background: var(--selected);
    color: var(--text);
    font-weight: 600;
  }

  .tabs button:disabled {
    opacity: 0.4;
  }

  .tabs .label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .badge {
    width: 22px;
    height: 22px;
    border-radius: 6px;
    font-size: 11px;
  }

  .pick,
  .update {
    display: flex;
    align-items: center;
    gap: 10px;
    margin: 0 12px 8px;
    padding: 10px 12px;
    border-radius: 10px;
    background: var(--selected);
  }

  .pick span {
    flex: 1;
    min-width: 0;
    overflow-wrap: anywhere;
  }

  .update span {
    flex: 1;
  }

  .progress {
    height: 2px;
    background: transparent;
  }

  .progress.on {
    background: linear-gradient(90deg, transparent, var(--accent), transparent) 0 0 / 40% 100% no-repeat;
    animation: slide 1s linear infinite;
  }

  @keyframes slide {
    from {
      background-position: -40% 0;
    }
    to {
      background-position: 140% 0;
    }
  }

  .list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    border-top: 1px solid var(--border);
    transition: opacity 0.15s;
    overscroll-behavior: contain;
  }

  .list.dim {
    opacity: 0.6;
  }

  .row {
    display: flex;
    align-items: center;
    gap: 14px;
    width: 100%;
    min-height: 60px;
    padding: 8px 16px;
    border: 0;
    background: none;
    text-align: left;
    user-select: none;
    -webkit-user-select: none;
    -webkit-touch-callout: none;
    -webkit-tap-highlight-color: transparent;
  }

  .row:active {
    background: var(--hover);
  }

  .row.selected {
    background: var(--selected);
  }

  .row.muted {
    opacity: 0.45;
  }

  .ico {
    position: relative;
    flex: none;
    display: grid;
    place-items: center;
    width: 36px;
    height: 36px;
    color: var(--muted);
  }

  .ico[data-kind="dir"] {
    color: #e0a43a;
  }

  .ico[data-kind="place"],
  .ico[data-kind="drive"] {
    color: var(--accent);
  }

  .check {
    display: grid;
    place-items: center;
    width: 32px;
    height: 32px;
    border-radius: 50%;
    background: var(--accent);
    color: var(--accent-text);
  }

  .lock {
    position: absolute;
    right: 0;
    bottom: 2px;
    display: grid;
    padding: 1px;
    border-radius: 50%;
    background: var(--surface);
    color: #455a64;
  }

  .text {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .name {
    display: -webkit-box;
    overflow: hidden;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow-wrap: anywhere;
  }

  .meta {
    overflow: hidden;
    color: var(--muted);
    font-size: 12.5px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .placeholder {
    padding: 48px 16px;
    color: var(--muted);
    text-align: center;
  }

  .banner {
    margin: 12px;
  }

  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 12px;
    padding: 48px 24px;
    color: var(--muted);
    text-align: center;
  }

  .empty h1 {
    margin: 0;
    color: var(--text);
    font-size: 20px;
    font-weight: 600;
  }

  .empty p {
    margin: 0 0 8px;
  }

  .btn.big {
    justify-content: center;
    height: 44px;
    padding: 0 18px;
    border-radius: 22px;
    font-size: 15px;
  }

  .btn.small {
    height: 32px;
    border-radius: 16px;
  }

  .empty .btn.big {
    width: min(300px, 100%);
  }

  .grow {
    flex: 1;
  }

  .actions {
    display: flex;
    gap: 8px;
    padding: 10px 12px;
    border-top: 1px solid var(--border);
    background: var(--surface);
  }

  .toast {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 8px 12px;
    padding: 8px 8px 8px 14px;
    border-radius: 10px;
    background: var(--text);
    color: var(--surface);
  }

  .toast span {
    flex: 1;
    min-width: 0;
    overflow-wrap: anywhere;
  }

  .toast .tool {
    color: inherit;
  }

  .toast.error {
    background: var(--danger);
    color: #fff;
  }

  .scrim {
    position: fixed;
    inset: 0;
    z-index: 40;
    display: flex;
    flex-direction: column;
    justify-content: flex-end;
    background: rgb(0 0 0 / 0.4);
  }

  .scrim.center {
    justify-content: center;
    align-items: center;
    padding: 16px;
  }

  .sheet {
    max-height: 80dvh;
    overflow-y: auto;
    padding: 8px 0 12px;
    border-radius: 16px 16px 0 0;
    background: var(--surface);
  }

  .sheet-title {
    padding: 10px 20px 8px;
    color: var(--muted);
    font-size: 13px;
  }

  .sheet hr {
    margin: 6px 0;
    border: 0;
    border-top: 1px solid var(--border);
  }

  .sheet-item {
    display: flex;
    align-items: center;
    gap: 16px;
    width: 100%;
    min-height: 52px;
    padding: 0 20px;
    border: 0;
    background: none;
    text-align: left;
  }

  .sheet-item:active:not(:disabled) {
    background: var(--hover);
  }

  .sheet-item:disabled {
    opacity: 0.4;
  }

  .sheet-item.danger {
    color: var(--danger);
  }

  .sheet-ico {
    display: grid;
    place-items: center;
    width: 24px;
    color: var(--muted);
  }

  .sheet-item.danger .sheet-ico {
    color: inherit;
  }

  .prompt {
    display: flex;
    flex-direction: column;
    gap: 16px;
    width: min(400px, 100%);
    padding: 20px;
    border-radius: 16px;
    background: var(--surface);
  }

  .prompt h2 {
    margin: 0;
    font-size: 18px;
    font-weight: 600;
  }

  .prompt input {
    height: 44px;
    padding: 0 12px;
    border: 1px solid var(--accent);
    border-radius: 8px;
    background: var(--surface);
    outline: none;
    font-size: 16px;
  }

  .prompt-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }

  /* The transfer panel is the desktop one; on a phone it gets bigger rows. */
  main > :global(section:last-child) {
    max-height: 40dvh;
  }
</style>
