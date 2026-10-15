<script lang="ts" module>
  import type { Entry } from "$lib/api";

  /** File operations a pane offers; the page binds them to the cloud or the disk. */
  export interface PaneOps {
    /** False where nothing can be created or changed (the virtual "This PC" root). */
    canModify(folder: string): boolean;
    /** Resolves to the new folder's path. */
    mkdir(parent: string, name: string): Promise<string>;
    /** Resolves to the item's new path. */
    rename(entry: Entry, newName: string): Promise<string>;
    remove(entries: Entry[]): Promise<void>;
    /** Confirmation text for deleting `entries`. */
    deletePrompt(entries: Entry[]): string;
    deleteLabel: string;
    /** Shows the item (or the open folder) in the system file manager. */
    reveal?(path: string): void;
    /** Opens access by link and copies the links; set where the cloud supports it. */
    share?(entries: Entry[]): Promise<void>;
    /** Asks for a password and expiry date, then shares; set where the cloud supports them. */
    shareWithOptions?(entries: Entry[]): void;
    /** Closes access by link. */
    unshare?(entries: Entry[]): Promise<void>;
  }
</script>

<script lang="ts">
  import { onMount, type Snippet } from "svelte";
  import { ask } from "@tauri-apps/plugin-dialog";
  import { SvelteSet } from "svelte/reactivity";
  import { errorText, type Listing } from "$lib/api";
  import { dnd, paneAt } from "$lib/dnd.svelte";
  import { formatBytes, formatDate } from "$lib/format";
  import ContextMenu, { type MenuItem } from "./ContextMenu.svelte";
  import Icon, { type IconName } from "./Icon.svelte";

  let {
    id,
    load: fetchListing,
    initialPath,
    rootPath,
    rootLabel,
    rootIcon,
    active,
    externalDrop = false,
    dropLabel,
    leading,
    actions,
    ops,
    copyLabel,
    canCopy = false,
    onactivate,
    oncopy,
    ondragto,
    onswitch,
    onnavigate,
    onopenaccount,
    onpreview,
    onopenfile,
  }: {
    /** Pane id used for drag and drop between panes. */
    id: string;
    load: (path: string) => Promise<Listing>;
    initialPath: string;
    rootPath: string;
    rootLabel: string;
    rootIcon: IconName;
    active: boolean;
    /** Files from Explorer are being dragged over this pane. */
    externalDrop?: boolean;
    /** Text shown while something can be dropped here. */
    dropLabel?: string;
    leading?: Snippet;
    actions?: Snippet;
    ops?: PaneOps;
    /** Context menu label for copying the selection to the other pane. */
    copyLabel?: string;
    canCopy?: boolean;
    onactivate: () => void;
    /** F5: copy the selection to the other pane. */
    oncopy: () => void;
    /** The selection was dragged and released over pane `target`. */
    ondragto: (target: string) => void;
    /** Tab: move focus to the other pane. */
    onswitch: () => void;
    onnavigate?: (path: string) => void;
    /** A row that stands for an encrypted folder was opened. */
    onopenaccount?: (accountId: string) => void;
    /** Space or double click on a file; `files` are the folder's files in display order. */
    onpreview?: (entry: Entry, files: Entry[]) => void;
    /** Enter or double click on a file, instead of the preview. */
    onopenfile?: (entry: Entry) => void;
  } = $props();

  type SortKey = "name" | "size" | "modified";

  let listing = $state<Listing | null>(null);
  let loading = $state(false);
  let error = $state("");
  const selected = new SvelteSet<string>();
  /** The row keyboard navigation moves from. */
  let cursor = $state<string | null>(null);
  /** Where a Shift-selection range starts. */
  let anchor: string | null = null;
  let sortKey = $state<SortKey>("name");
  let sortAsc = $state(true);
  let grid: HTMLDivElement;
  /** Path of the row being renamed inline. */
  let renaming = $state<string | null>(null);
  let renameValue = $state("");
  let menu = $state<{ x: number; y: number; items: MenuItem[] } | null>(null);
  let opBusy = $state("");

  /** Responses for a folder the user has already left are dropped. */
  let requestSeq = 0;

  const path = $derived(listing?.path ?? initialPath);
  const entries = $derived(listing?.entries ?? []);
  const collator = new Intl.Collator("ru", { numeric: true, sensitivity: "base" });

  const sorted = $derived.by(() => {
    const dir = sortAsc ? 1 : -1;
    const byName = (a: Entry, b: Entry) => collator.compare(a.name, b.name);
    const compare = (a: Entry, b: Entry) => {
      // The virtual root keeps its own order (places, then drives).
      if (a.icon || b.icon) return 0;
      if (a.is_dir !== b.is_dir) return a.is_dir ? -1 : 1;
      let result = 0;
      if (sortKey === "size") result = a.size - b.size;
      else if (sortKey === "modified")
        result = (a.modified ? Date.parse(a.modified) : 0) - (b.modified ? Date.parse(b.modified) : 0);
      return (result || byName(a, b)) * dir;
    };
    return [...entries].sort(compare);
  });

  /** Row keys: Google Drive allows several items with the same name in a
   * folder, so repeated paths get a counter to keep the keys unique. */
  const rowKeys = $derived.by(() => {
    const seen = new Map<string, number>();
    return sorted.map((e) => {
      const n = seen.get(e.path) ?? 0;
      seen.set(e.path, n + 1);
      return n ? `${e.path}\u0000${n}` : e.path;
    });
  });
  const selectedEntries = $derived(sorted.filter((e) => selected.has(e.path)));
  const dragTarget = $derived(dnd.from !== null && dnd.from !== id && dnd.over === id);

  const summary = $derived.by(() => {
    const picked = selectedEntries;
    if (picked.length > 1 || (picked.length === 1 && !picked[0].is_dir)) {
      const bytes = picked.reduce((sum, e) => sum + (e.is_dir ? 0 : e.size), 0);
      return `Выбрано: ${picked.length} · ${formatBytes(bytes)}`;
    }
    const files = entries.filter((e) => !e.is_dir);
    const dirs = entries.length - files.length;
    const bytes = files.reduce((sum, e) => sum + e.size, 0);
    return `Папок: ${dirs} · файлов: ${files.length} · ${formatBytes(bytes)}`;
  });

  /** Opens `target`; `focus` selects rows there (by default the selection is kept on reload). */
  export async function navigate(target: string, focus: string[] | null = null) {
    const seq = ++requestSeq;
    loading = true;
    error = "";
    try {
      const result = await fetchListing(target);
      if (seq !== requestSeq) return;
      const changedFolder = result.path !== listing?.path;
      listing = result;
      const keep = new Set(result.entries.map((e) => e.path));
      const next = (focus ?? (changedFolder ? [] : [...selected])).filter((p) => keep.has(p));
      selected.clear();
      for (const p of next) selected.add(p);
      if (focus || changedFolder || !cursor || !keep.has(cursor)) cursor = next[0] ?? null;
      anchor = cursor;
      if (changedFolder) onnavigate?.(result.path);
    } catch (err) {
      if (seq !== requestSeq) return;
      error = errorText(err);
      // Show the error instead of a stale folder after a failed first load.
      listing ??= { path: target, parent: target === rootPath ? null : rootPath, crumbs: [], entries: [] };
    } finally {
      if (seq === requestSeq) loading = false;
    }
    if (focus?.[0]) {
      const row = grid?.querySelector(`[data-path="${CSS.escape(focus[0])}"]`);
      row?.scrollIntoView({ block: "nearest" });
    }
  }

  export function reload() {
    return navigate(path);
  }

  export function getPath() {
    return path;
  }

  export function getEntries() {
    return entries;
  }

  export function getSelection() {
    return selectedEntries;
  }

  export function focus() {
    grid.focus();
  }

  // The page remounts a pane (via {#key}) whenever its source changes.
  onMount(() => {
    navigate(initialPath);
  });

  function open(entry: Entry) {
    if (entry.opensAccount && onopenaccount) onopenaccount(entry.opensAccount);
    else if (entry.is_dir) navigate(entry.path);
    else if (onopenfile) onopenfile(entry);
    else preview(entry);
  }

  function preview(entry: Entry) {
    if (onpreview && !entry.is_dir) onpreview(entry, sorted.filter((e) => !e.is_dir));
  }

  function goUp() {
    if (listing?.parent != null) navigate(listing.parent, [path]);
  }

  function sortBy(key: SortKey) {
    if (sortKey === key) sortAsc = !sortAsc;
    else {
      sortKey = key;
      sortAsc = true;
    }
  }

  function selectRange(from: string | null, to: string) {
    const a = sorted.findIndex((e) => e.path === from);
    const b = sorted.findIndex((e) => e.path === to);
    selected.clear();
    if (a < 0) {
      selected.add(to);
      return;
    }
    for (let i = Math.min(a, b); i <= Math.max(a, b); i++) selected.add(sorted[i].path);
  }

  function selectOnly(p: string) {
    selected.clear();
    selected.add(p);
    anchor = p;
    cursor = p;
  }

  function click(event: MouseEvent, entry: Entry) {
    if (suppressClick) {
      suppressClick = false;
      return;
    }
    if (event.shiftKey) {
      selectRange(anchor, entry.path);
    } else if (event.ctrlKey || event.metaKey) {
      if (selected.has(entry.path)) selected.delete(entry.path);
      else selected.add(entry.path);
      anchor = entry.path;
    } else {
      selectOnly(entry.path);
      return;
    }
    cursor = entry.path;
  }

  // ---- dragging rows to the other pane ----

  let pressed: { entry: Entry; x: number; y: number } | null = null;
  let suppressClick = false;

  function pointerdown(event: PointerEvent, entry: Entry) {
    if (event.button !== 0 || event.shiftKey || event.ctrlKey || event.metaKey) return;
    pressed = { entry, x: event.clientX, y: event.clientY };
  }

  function pointermove(event: PointerEvent) {
    if (!pressed) return;
    if (dnd.from === null) {
      if (Math.hypot(event.clientX - pressed.x, event.clientY - pressed.y) < 6) return;
      if (!selected.has(pressed.entry.path)) selectOnly(pressed.entry.path);
      dnd.from = id;
      dnd.count = selected.size;
    }
    dnd.x = event.clientX;
    dnd.y = event.clientY;
    dnd.over = paneAt(event.clientX, event.clientY);
  }

  function pointerup() {
    if (dnd.from === id) {
      const target = dnd.over;
      dnd.from = null;
      dnd.over = null;
      suppressClick = true;
      setTimeout(() => (suppressClick = false));
      if (target && target !== id) ondragto(target);
    }
    pressed = null;
  }

  function onkeydown(event: KeyboardEvent) {
    if (event.target instanceof HTMLInputElement) return;
    const index = sorted.findIndex((e) => e.path === cursor);
    const move = (to: number) => {
      const entry = sorted[Math.max(0, Math.min(sorted.length - 1, to))];
      if (!entry) return;
      cursor = entry.path;
      if (event.shiftKey) selectRange(anchor, entry.path);
      else selectOnly(entry.path);
      grid.querySelector(`[data-path="${CSS.escape(entry.path)}"]`)?.scrollIntoView({ block: "nearest" });
    };
    const ctrl = event.ctrlKey || event.metaKey;
    if (ctrl && event.key.toLowerCase() === "a") {
      selected.clear();
      for (const e of sorted) selected.add(e.path);
    } else if (ctrl && event.key.toLowerCase() === "r") {
      reload();
    } else if (ctrl && event.shiftKey && event.key.toLowerCase() === "n") {
      newFolder();
    } else {
      switch (event.key) {
        case "ArrowDown": move(index + 1); break;
        case "ArrowUp": move(index < 0 ? 0 : index - 1); break;
        case "Home": move(0); break;
        case "End": move(sorted.length - 1); break;
        case "Enter": if (index >= 0) open(sorted[index]); break;
        case "Backspace": goUp(); break;
        case "F5": oncopy(); break;
        case "Tab": onswitch(); break;
        case "Escape": selected.clear(); break;
        case " ": {
          const entry = sorted[index];
          if (!entry || entry.is_dir || !onpreview) return;
          preview(entry);
          break;
        }
        case "F2": if (selectedEntries.length === 1) startRename(selectedEntries[0]); break;
        case "Delete": removeSelected(); break;
        case "F7": newFolder(); break;
        case "ContextMenu": {
          const row = cursor ? grid.querySelector(`[data-path="${CSS.escape(cursor)}"]`) : null;
          const rect = (row ?? grid).getBoundingClientRect();
          const entry = sorted.find((e) => e.path === cursor) ?? null;
          openMenu(new MouseEvent("contextmenu", { clientX: rect.left + 40, clientY: rect.bottom }), entry);
          break;
        }
        default: return;
      }
    }
    event.preventDefault();
  }

  const modifiable = $derived(!!ops && listing !== null && ops.canModify(path));

  async function operation(label: string, action: () => Promise<void>) {
    opBusy = label;
    error = "";
    try {
      await action();
    } catch (err) {
      error = errorText(err);
    } finally {
      opBusy = "";
    }
  }

  function startRename(entry: Entry) {
    if (!modifiable || entry.icon) return;
    renaming = entry.path;
    renameValue = entry.name;
  }

  /** Selects the name without its extension, like Explorer does. */
  function focusRenameInput(input: HTMLInputElement) {
    input.focus();
    const entry = entries.find((e) => e.path === renaming);
    const dot = entry && !entry.is_dir ? input.value.lastIndexOf(".") : -1;
    input.setSelectionRange(0, dot > 0 ? dot : input.value.length);
  }

  function cancelRename() {
    renaming = null;
    grid.focus();
  }

  async function commitRename() {
    const entry = entries.find((e) => e.path === renaming);
    renaming = null;
    grid.focus();
    if (!entry || !ops || renameValue.trim() === entry.name) return;
    const newName = renameValue;
    await operation("Переименование…", async () => {
      const newPath = await ops.rename(entry, newName);
      await navigate(path, [newPath]);
    });
  }

  function uniqueName(base: string) {
    const taken = new Set(entries.map((e) => e.name.toLowerCase()));
    if (!taken.has(base.toLowerCase())) return base;
    for (let n = 2; ; n++) {
      const name = `${base} (${n})`;
      if (!taken.has(name.toLowerCase())) return name;
    }
  }

  /** Creates "Новая папка" right away and lets the user rename it, like Explorer. */
  async function newFolder() {
    if (!modifiable || !ops) return;
    const name = uniqueName("Новая папка");
    await operation("Создание папки…", async () => {
      const created = await ops.mkdir(path, name);
      await navigate(path, [created]);
      const entry = entries.find((e) => e.path === created);
      if (entry) startRename(entry);
    });
  }

  async function removeSelected() {
    const items = selectedEntries.filter((e) => !e.icon);
    if (!modifiable || !ops || !items.length) return;
    const confirmed = await ask(ops.deletePrompt(items), {
      title: "Удаление",
      kind: "warning",
      okLabel: ops.deleteLabel,
      cancelLabel: "Отмена",
    });
    if (!confirmed) return;
    await operation("Удаление…", async () => {
      // Keep the cursor near the deleted rows.
      const index = sorted.findIndex((e) => e.path === items[0].path);
      const removed = new Set(items.map((e) => e.path));
      const neighbour = sorted.slice(index).find((e) => !removed.has(e.path))
        ?? sorted.slice(0, index).reverse().find((e) => !removed.has(e.path));
      await ops.remove(items);
      await navigate(path, neighbour ? [neighbour.path] : []);
    });
    grid.focus();
  }

  function openMenu(event: MouseEvent, entry: Entry | null) {
    event.preventDefault();
    onactivate();
    if (entry && !selected.has(entry.path)) selectOnly(entry.path);
    const picked = entry ? selectedEntries : [];
    const single = picked.length === 1 ? picked[0] : null;
    const items: MenuItem[] = [];
    if (single?.is_dir) items.push({ label: "Открыть", icon: "folder", shortcut: "Enter", action: () => open(single) });
    if (single && !single.is_dir && onopenfile) {
      items.push({ label: "Открыть", icon: "file", shortcut: "Enter", action: () => open(single) });
    }
    if (single && !single.is_dir && onpreview) {
      items.push({ label: "Просмотр", icon: "eye", shortcut: "Пробел", action: () => preview(single) });
    }
    if (picked.length && copyLabel) {
      items.push({
        label: copyLabel,
        icon: id === "cloud" ? "arrowRight" : "arrowLeft",
        shortcut: "F5",
        disabled: !canCopy,
        action: oncopy,
      });
    }
    if (items.length) items.push("separator");
    const { share, shareWithOptions, unshare } = ops ?? {};
    if (picked.length && share && unshare) {
      items.push({
        label: picked.length === 1 ? "Поделиться ссылкой" : "Поделиться ссылками",
        icon: "link",
        action: () => operation("Создание ссылки…", () => share(picked)),
      });
      if (shareWithOptions) {
        items.push({ label: "Ссылка с паролем или сроком…", action: () => shareWithOptions(picked) });
      }
      items.push({
        label: "Закрыть доступ по ссылке",
        action: () => operation("Закрытие доступа…", () => unshare(picked)),
      });
      items.push("separator");
    }
    if (ops) {
      items.push({ label: "Новая папка", icon: "plus", shortcut: "F7", disabled: !modifiable, action: newFolder });
      if (picked.length) {
        items.push({
          label: "Переименовать",
          shortcut: "F2",
          disabled: !modifiable || !single || !!single.icon,
          action: () => single && startRename(single),
        });
        items.push({
          label: ops.deleteLabel,
          icon: "trash",
          shortcut: "Del",
          danger: true,
          disabled: !modifiable || picked.every((e) => e.icon),
          action: removeSelected,
        });
      }
      items.push("separator");
      const reveal = ops.reveal;
      if (reveal && (single || path)) {
        items.push({
          label: "Показать в Проводнике",
          icon: "reveal",
          action: () => reveal(single?.path ?? path),
        });
      }
    }
    items.push({ label: "Обновить", icon: "refresh", shortcut: "Ctrl+R", action: reload });
    menu = { x: event.clientX, y: event.clientY, items };
  }

  function sortMark(key: SortKey) {
    return sortKey === key ? (sortAsc ? " ▲" : " ▼") : "";
  }

  function iconFor(entry: Entry): IconName {
    if (entry.icon) return entry.icon;
    return entry.is_dir ? "folder" : "file";
  }
</script>

<svelte:window onpointermove={pointermove} onpointerup={pointerup} />

<!-- Any click or focus inside the pane makes it the active one. -->
<section data-pane={id} class:active onpointerdowncapture={onactivate} onfocusin={onactivate}>
  <div class="toolbar">
    {@render leading?.()}
    <button class="icon-btn" title="Вверх (Backspace)" onclick={goUp} disabled={listing?.parent == null}>
      <Icon name="up" />
    </button>
    <button class="icon-btn" title="Обновить (Ctrl+R)" onclick={reload}>
      <Icon name="refresh" />
    </button>

    <nav class="crumbs" aria-label="Путь">
      <button onclick={() => navigate(rootPath)} title={rootLabel}>
        <Icon name={rootIcon} size={14} />
        <span>{rootLabel}</span>
      </button>
      {#each listing?.crumbs ?? [] as crumb (crumb.path)}
        <Icon name="chevron" size={12} />
        <button onclick={() => navigate(crumb.path)}>{crumb.name}</button>
      {/each}
    </nav>

    {#if actions}
      <div class="actions">{@render actions()}</div>
    {/if}
  </div>

  {#if error}
    <div class="error banner">{error}</div>
  {/if}

  <div
    class="table"
    role="grid"
    aria-multiselectable="true"
    tabindex="0"
    aria-busy={loading}
    {onkeydown}
    bind:this={grid}
  >
    <div class="head" role="row">
      <button role="columnheader" tabindex="-1" onclick={() => sortBy("name")}>Имя{sortMark("name")}</button>
      <button role="columnheader" tabindex="-1" class="num" onclick={() => sortBy("size")}>Размер{sortMark("size")}</button>
      <button role="columnheader" tabindex="-1" onclick={() => sortBy("modified")}>Изменён{sortMark("modified")}</button>
    </div>

    <div class="body" class:dim={loading} role="presentation" oncontextmenu={(e) => openMenu(e, null)}>
      {#each sorted as entry, i (rowKeys[i])}
        <!-- Keyboard navigation is handled by the grid's onkeydown. -->
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <div
          class="row"
          role="row"
          tabindex="-1"
          aria-selected={selected.has(entry.path)}
          class:selected={selected.has(entry.path)}
          class:cursor={entry.path === cursor}
          data-path={entry.path}
          onpointerdown={(e) => pointerdown(e, entry)}
          onclick={(e) => click(e, entry)}
          ondblclick={() => open(entry)}
          oncontextmenu={(e) => {
            e.stopPropagation();
            openMenu(e, entry);
          }}
        >
          <span class="name" role="gridcell">
            <span class="ico" data-kind={entry.icon ?? (entry.is_dir ? "dir" : "file")} title={entry.locked}>
              <Icon name={iconFor(entry)} />
              {#if entry.locked}
                <span class="lock"><Icon name="lock" size={10} /></span>
              {/if}
            </span>
            {#if renaming === entry.path}
              <input
                class="rename"
                bind:value={renameValue}
                use:focusRenameInput
                spellcheck="false"
                onkeydown={(e) => {
                  e.stopPropagation();
                  if (e.key === "Enter") commitRename();
                  else if (e.key === "Escape") cancelRename();
                }}
                onblur={() => renaming === entry.path && commitRename()}
                onpointerdown={(e) => e.stopPropagation()}
                onclick={(e) => e.stopPropagation()}
                ondblclick={(e) => e.stopPropagation()}
              />
            {:else}
              <span class="label" title={entry.name}>{entry.name}</span>
            {/if}
          </span>
          <span class="num" role="gridcell">{entry.is_dir ? "" : formatBytes(entry.size)}</span>
          <span class="date" role="gridcell">{formatDate(entry.modified)}</span>
        </div>
      {:else}
        {#if !loading && !error}
          <div class="placeholder">Папка пуста</div>
        {/if}
      {/each}
    </div>

    {#if (dragTarget || externalDrop) && dropLabel}
      <div class="drop">
        <Icon name="download" size={28} />
        <span>{dropLabel}</span>
      </div>
    {/if}
  </div>

  <footer>
    <span>{opBusy || (loading ? "Загрузка списка…" : summary)}</span>
  </footer>
</section>

{#if menu}
  <ContextMenu x={menu.x} y={menu.y} items={menu.items} onclose={() => (menu = null)} />
{/if}

<style>
  section {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    background: var(--surface);
  }

  .toolbar {
    display: flex;
    align-items: center;
    gap: 2px;
    min-height: 41px;
    padding: 6px 8px;
    border-bottom: 1px solid var(--border);
    box-shadow: inset 0 2px 0 transparent;
  }

  section.active .toolbar {
    box-shadow: inset 0 2px 0 var(--accent);
  }

  .crumbs {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 2px;
    min-width: 0;
    margin-left: 4px;
    overflow: hidden;
    color: var(--muted);
  }

  .crumbs button {
    display: inline-flex;
    flex-shrink: 1;
    align-items: center;
    gap: 6px;
    min-width: 0;
    height: 26px;
    padding: 0 6px;
    border: 0;
    border-radius: 5px;
    background: none;
    color: var(--text);
    white-space: nowrap;
  }

  .crumbs button:last-child {
    flex-shrink: 0;
  }

  .crumbs button span {
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .crumbs button:hover {
    background: var(--hover);
  }

  .actions {
    display: flex;
    gap: 6px;
    margin-left: 8px;
  }

  .banner {
    margin: 8px;
  }

  .table {
    position: relative;
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
    outline: none;
  }

  .head,
  .row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 90px 130px;
    align-items: center;
  }

  .head {
    border-bottom: 1px solid var(--border);
  }

  .head button {
    height: 28px;
    padding: 0 12px;
    border: 0;
    background: none;
    color: var(--muted);
    font-size: 12px;
    text-align: left;
  }

  .head button:hover {
    color: var(--text);
  }

  .head .num,
  .row .num {
    text-align: right;
  }

  .body {
    flex: 1;
    overflow-y: auto;
    transition: opacity 0.15s;
  }

  .body.dim {
    opacity: 0.55;
  }

  .row {
    height: 28px;
    cursor: default;
    user-select: none;
  }

  .row:hover {
    background: var(--hover);
  }

  .row.selected {
    background: var(--selected);
  }

  section.active .table:focus .row.selected {
    background: var(--selected-strong);
  }

  section.active .table:focus .row.cursor {
    box-shadow: inset 0 0 0 1px var(--focus);
  }

  .row > span {
    padding: 0 12px;
    overflow: hidden;
    white-space: nowrap;
  }

  .name {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .label {
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .rename {
    flex: 1;
    min-width: 0;
    height: 22px;
    margin-left: -4px;
    padding: 0 4px;
    border: 1px solid var(--accent);
    border-radius: 4px;
    background: var(--surface);
    outline: none;
  }

  .ico {
    position: relative;
    display: inline-grid;
    color: var(--muted);
  }

  .lock {
    position: absolute;
    right: -4px;
    bottom: -3px;
    display: grid;
    padding: 1px;
    border-radius: 50%;
    background: var(--surface);
    color: #455a64;
  }

  .row.selected .lock {
    background: var(--selected);
  }

  .ico[data-kind="dir"] {
    color: #e0a43a;
  }

  .ico[data-kind="place"],
  .ico[data-kind="drive"] {
    color: var(--accent);
  }

  .num,
  .date {
    color: var(--muted);
    font-variant-numeric: tabular-nums;
  }

  .placeholder {
    padding: 40px;
    color: var(--muted);
    text-align: center;
  }

  .drop {
    position: absolute;
    inset: 6px;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 16px;
    border: 2px dashed var(--accent);
    border-radius: 10px;
    background: color-mix(in srgb, var(--surface) 85%, var(--accent));
    color: var(--accent);
    font-size: 14px;
    text-align: center;
    pointer-events: none;
  }

  footer {
    padding: 4px 12px;
    border-top: 1px solid var(--border);
    color: var(--muted);
    font-size: 12px;
  }
</style>
