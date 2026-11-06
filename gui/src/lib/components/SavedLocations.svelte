<script lang="ts">
  import { tick } from "svelte";
  import type { Account } from "$lib/api";
  import { baseName } from "$lib/format";
  import { LOCATIONS_KEY, loadLocations, locationKey, type SavedLocation } from "$lib/locations";
  import { remember } from "$lib/storage";
  import Icon from "./Icon.svelte";

  let { open, accounts, cloud, local, onopen, onclose }: {
    open: boolean;
    accounts: Account[];
    cloud: { account: string; path: string } | null;
    local: string;
    onopen: (location: SavedLocation) => void;
    onclose: () => void;
  } = $props();
  let dialog: HTMLDialogElement;
  let locations = $state<SavedLocation[]>(loadLocations());
  let category = $state<"folders" | "pairs">("folders");
  let editing = $state<string | null>(null);
  let name = $state("");
  let message = $state("");
  const shown = $derived(locations.filter((l) => (l.kind === "pair") === (category === "pairs")));
  const accountName = (id: string) => accounts.find((a) => a.id === id)?.name ?? "Аккаунт удалён";
  const available = (l: SavedLocation) => l.kind === "local" || accounts.some((a) => a.id === l.account);
  const details = (l: SavedLocation) => l.kind === "local" ? l.local || "Этот компьютер" :
    `${accountName(l.account)}: ${l.remote}${l.kind === "pair" ? ` ↔ ${l.local}` : ""}`;

  $effect(() => remember(LOCATIONS_KEY, JSON.stringify(locations)));
  $effect(() => {
    if (open && !dialog.open) { message = ""; editing = null; dialog.showModal(); }
    else if (!open && dialog.open) dialog.close();
  });

  function add(kind: SavedLocation["kind"]) {
    const id = crypto.randomUUID();
    const acc = cloud && accounts.find((a) => a.id === cloud.account);
    let entry: SavedLocation;
    if (kind === "local") entry = { id, name: baseName(local) || "Этот компьютер", kind, local };
    else if (!cloud || !acc) return;
    else if (kind === "cloud") entry = { id, name: cloud.path === "/" ? acc.name : baseName(cloud.path), kind, account: acc.id, remote: cloud.path };
    else {
      if (!local) return;
      entry = { id, name: `${baseName(local)} ↔ ${acc.name}`, kind, local, account: acc.id, remote: cloud.path };
    }
    category = kind === "pair" ? "pairs" : "folders";
    const existing = locations.find((l) => locationKey(l) === locationKey(entry));
    if (existing) { message = "Это расположение уже сохранено"; return; }
    locations = [...locations, entry];
    message = "Сохранено";
    editing = entry.id;
    name = entry.name;
  }

  function rename() {
    if (!editing || !name.trim()) return;
    locations = locations.map((l) => l.id === editing ? { ...l, name: name.trim() } : l);
    editing = null;
  }

  function focusName(input: HTMLInputElement) { input.focus(); input.select(); }

  async function openLocation(location: SavedLocation) {
    dialog.close();
    await tick();
    onopen(location);
  }
</script>

<dialog class="modal saved" bind:this={dialog} onclose={onclose} aria-labelledby="saved-title">
  <header><h2 id="saved-title">Избранное</h2>
    <button class="icon-btn" aria-label="Закрыть избранное" onclick={() => dialog.close()}><Icon name="close" /></button>
  </header>
  <div class="save-actions">
    <span>Сохранить текущее:</span>
    <button class="btn" onclick={() => add("cloud")} disabled={!cloud}><Icon name="cloud" /> Папку облака</button>
    <button class="btn" onclick={() => add("local")}><Icon name="computer" /> Папку компьютера</button>
    <button class="btn" onclick={() => add("pair")} disabled={!cloud || !local}><Icon name="link" /> Пару папок</button>
  </div>
  <nav aria-label="Раздел избранного">
    <button class:chosen={category === "folders"} aria-pressed={category === "folders"} onclick={() => { category = "folders"; editing = null; }}>Папки ({locations.filter((l) => l.kind !== "pair").length})</button>
    <button class:chosen={category === "pairs"} aria-pressed={category === "pairs"} onclick={() => { category = "pairs"; editing = null; }}>Пары папок ({locations.filter((l) => l.kind === "pair").length})</button>
  </nav>
  {#if message}<div class="message" role="status">{message}</div>{/if}
  <div class="list">
    {#each shown as location (location.id)}
      <div class="item">
        <Icon name={location.kind === "pair" ? "link" : location.kind === "cloud" ? "cloud" : "folder"} />
        {#if editing === location.id}
          <form onsubmit={(e) => { e.preventDefault(); rename(); }}>
            <input bind:value={name} use:focusName aria-label="Название избранного" maxlength="120" onkeydown={(e) => { if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); editing = null; } }} />
            <button class="btn" disabled={!name.trim()}>Сохранить название</button>
          </form>
        {:else}
          <button class="location" disabled={!available(location)} onclick={() => openLocation(location)} title={details(location)}>
            <strong>{location.name}</strong><small>{details(location)}</small>
          </button>
          <button class="icon-btn" aria-label={`Переименовать ${location.name}`} onclick={() => { editing = location.id; name = location.name; }}><Icon name="edit" /></button>
          <button class="icon-btn" aria-label={`Убрать ${location.name} из избранного`} onclick={() => { locations = locations.filter((l) => l.id !== location.id); message = ""; }}><Icon name="trash" /></button>
        {/if}
      </div>
    {:else}
      <p class="empty">{category === "pairs" ? "Сохраните открытую пару папок, чтобы быстро возвращаться к ней." : "Сохраните часто используемые папки компьютера и облаков."}</p>
    {/each}
  </div>
  <footer>{category === "pairs" ? "Открытие пары меняет папки в обеих панелях." : "Нажмите на сохранённую папку, чтобы открыть её."}</footer>
</dialog>

<style>
  .saved { width: min(720px, calc(100vw - 32px)); padding: 0; }
  header { display: flex; align-items: center; justify-content: space-between; padding: 12px 16px; border-bottom: 1px solid var(--border); }
  .save-actions { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; padding: 12px 16px; }
  .save-actions > span { color: var(--muted); font-size: 12px; }
  nav { display: flex; gap: 4px; padding: 0 16px 8px; border-bottom: 1px solid var(--border); }
  nav button { border: 0; background: none; border-radius: 4px; padding: 6px 10px; color: var(--muted); }
  nav button.chosen { background: var(--selected); color: var(--text); }
  .list { min-height: 140px; max-height: 45vh; overflow-y: auto; padding: 4px 8px; }
  .item { display: flex; align-items: center; gap: 8px; padding: 4px 8px; border-bottom: 1px solid var(--border); }
  .item > :global(svg) { flex: none; color: var(--muted); }
  .location { display: flex; flex-direction: column; gap: 3px; flex: 1; min-width: 0; text-align: left; border: 0; background: none; padding: 8px 0; }
  .location:hover strong { color: var(--accent); }
  strong { font-weight: 500; }
  small { color: var(--muted); overflow-wrap: anywhere; }
  .location:disabled { opacity: .5; }
  form { display: flex; gap: 8px; flex: 1; min-width: 0; padding: 6px 0; }
  input { flex: 1; min-width: 0; height: 28px; border: 1px solid var(--border); border-radius: 4px; background: var(--surface); padding: 0 8px; }
  .message, footer { color: var(--muted); padding: 8px 16px; font-size: 12px; }
  footer { border-top: 1px solid var(--border); }
  .empty { color: var(--muted); text-align: center; padding: 24px; }
</style>
