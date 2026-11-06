<script lang="ts">
  import { onMount } from "svelte";
  import { api, errorText, type Account, type Entry } from "$lib/api";
  import { cloudListing } from "$lib/cloud";
  import { parentPath, splitPath } from "$lib/format";
  import Icon from "./Icon.svelte";

  let { accounts, initialAccount, initialPath, onpick, onclose }: {
    accounts: Account[];
    initialAccount: string;
    initialPath: string;
    onpick: (accountId: string, path: string) => void;
    onclose: () => void;
  } = $props();

  let dialog: HTMLDialogElement;
  let accountId = $state("");
  let path = $state("/");
  let folders = $state<Entry[]>([]);
  let selected = $state<Entry | null>(null);
  let loading = $state(false);
  let error = $state("");
  let creating = $state(false);
  let name = $state<string | null>(null);
  let createError = $state("");
  let seq = 0;
  const account = $derived(accounts.find((a) => a.id === accountId));
  const crumbs = $derived(splitPath(path));
  const collator = new Intl.Collator("ru", { numeric: true, sensitivity: "base" });

  async function navigate(target: string) {
    const request = ++seq;
    const acc = account;
    if (!acc) return;
    loading = true;
    selected = null;
    name = null;
    error = createError = "";
    try {
      const result = await cloudListing(accounts, acc, target);
      if (request !== seq) return;
      path = result.path;
      folders = result.entries.filter((e) => e.is_dir).sort((a, b) => collator.compare(a.name, b.name));
    } catch (err) {
      if (request === seq) { error = errorText(err); folders = []; }
    } finally {
      if (request === seq) loading = false;
    }
  }

  function open(entry: Entry) {
    if (entry.opensAccount) { accountId = entry.opensAccount; navigate("/"); }
    else navigate(entry.path);
  }

  async function createFolder() {
    if (!account || !name?.trim() || creating) return;
    creating = true;
    createError = "";
    try {
      const created = await api.cloudMkdir(account.id, path, name.trim());
      await navigate(created);
    } catch (err) { createError = errorText(err); }
    finally { creating = false; }
  }

  function choose() {
    if (loading || creating || error || !account) return;
    if (selected?.opensAccount) onpick(selected.opensAccount, "/");
    else onpick(account.id, selected?.path ?? path);
  }

  function focusInput(input: HTMLInputElement) { input.focus(); }

  onMount(() => {
    accountId = accounts.find((a) => a.id === initialAccount)?.id ?? accounts[0]?.id ?? "";
    path = accountId === initialAccount ? "/" + initialPath.trim().replace(/^\/+|\/+$/g, "") : "/";
    dialog.showModal();
    navigate(path);
    return () => { seq++; };
  });
</script>

<dialog bind:this={dialog} class="modal picker" aria-labelledby="folder-picker-title"
  oncancel={(e) => { e.preventDefault(); onclose(); }} onclose={onclose}>
  <header><h2 id="folder-picker-title">Папка в облаке</h2>
    <button class="icon-btn" aria-label="Закрыть выбор папки" onclick={onclose}><Icon name="close" /></button>
  </header>
  <label class="account"><span>Облако</span>
    <select bind:value={accountId} disabled={creating} onchange={() => { path = "/"; folders = []; navigate("/"); }}>
      {#each accounts as a (a.id)}<option value={a.id}>{a.name}</option>{/each}
    </select>
  </label>
  <div class="navigation">
    <button class="icon-btn" aria-label="На уровень выше" onclick={() => navigate(parentPath(path))} disabled={path === "/" || loading || creating}><Icon name="up" /></button>
    <nav aria-label="Путь выбора папки">
      <button onclick={() => navigate("/")} disabled={creating}><Icon name="cloud" /> Корень</button>
      {#each crumbs as crumb (crumb.path)}
        <Icon name="chevron" size={12} /><button onclick={() => navigate(crumb.path)} disabled={creating}>{crumb.name}</button>
      {/each}
    </nav>
    <button class="icon-btn" aria-label="Обновить папки" onclick={() => navigate(path)} disabled={loading || creating}><Icon name="refresh" /></button>
  </div>
  {#if error}<div class="error" role="alert">{error}</div>{/if}
  <div class="folders" aria-busy={loading}>
    {#if loading}<p class="empty" role="status">Загрузка папок…</p>
    {:else}
      {#each folders as folder, i (`${folder.path}:${i}`)}
        <div class="folder" class:selected={selected === folder}>
          <button class="select-folder" aria-pressed={selected === folder} disabled={creating}
            onclick={() => (selected = folder)} ondblclick={() => open(folder)}>
            <Icon name={folder.locked ? "lock" : "folder"} /><span>{folder.name}</span>
          </button>
          <button class="icon-btn" aria-label={`Открыть папку ${folder.name}`} disabled={creating} onclick={() => open(folder)}><Icon name="chevron" /></button>
        </div>
      {:else}{#if !error}<p class="empty">Вложенных папок нет. Можно выбрать текущую или создать новую.</p>{/if}
      {/each}
    {/if}
  </div>
  {#if name !== null}
    <form class="create" onsubmit={(e) => { e.preventDefault(); createFolder(); }}>
      <input bind:value={name} use:focusInput aria-label="Имя новой папки" placeholder="Имя новой папки" disabled={creating} />
      <button class="btn" type="submit" disabled={creating || !name.trim()}>Создать</button>
      <button class="icon-btn" type="button" aria-label="Отменить создание папки" disabled={creating} onclick={() => { name = null; createError = ""; }}><Icon name="close" /></button>
    </form>
  {/if}
  {#if createError}<div class="error" role="alert">{createError}</div>{/if}
  <div class="destination">Выбрать: <strong>{account?.name}: {selected?.path ?? path}</strong></div>
  <footer>
    <button class="btn" disabled={loading || creating || !!error || !account} onclick={() => (name = "")}><Icon name="plus" /> Новая папка</button>
    <span></span><button class="btn" onclick={onclose} disabled={creating}>Отмена</button>
    <button class="btn primary" onclick={choose} disabled={loading || creating || !!error || !account}>Выбрать папку</button>
  </footer>
</dialog>

<style>
  .picker { width: min(620px, calc(100vw - 32px)); padding: 0; }
  header, footer, .navigation, .create, .account { display: flex; align-items: center; gap: 8px; padding: 12px 16px; }
  header { justify-content: space-between; border-bottom: 1px solid var(--border); }
  h2 { margin: 0; font-size: 16px; }
  .account span { color: var(--muted); }
  label.account { flex-direction: row; }
  .account select { width: 0; }
  select, input { flex: 1; min-width: 0; height: 32px; background: var(--surface); border: 1px solid var(--border); border-radius: 4px; padding: 0 8px; }
  .navigation { padding-top: 0; border-bottom: 1px solid var(--border); }
  nav { display: flex; align-items: center; flex: 1; min-width: 0; overflow-x: auto; }
  nav button { display: inline-flex; align-items: center; gap: 5px; border: 0; background: none; white-space: nowrap; padding: 6px; }
  .folders { height: min(280px, 35vh); overflow: auto; padding: 4px 8px; }
  .folder { display: flex; align-items: center; border-radius: 4px; }
  .folder:hover { background: var(--hover); }
  .folder.selected { background: var(--selected); }
  .select-folder { flex: 1; display: flex; align-items: center; gap: 8px; min-width: 0; text-align: left; border: 0; background: none; padding: 9px 8px; }
  .select-folder span { overflow-wrap: anywhere; }
  .select-folder :global(svg) { flex: none; color: var(--folder); }
  .empty { color: var(--muted); padding: 20px 8px; text-align: center; }
  .destination { padding: 8px 16px; overflow-wrap: anywhere; color: var(--muted); font-size: 12px; }
  .destination strong { color: var(--text); font-weight: 400; }
  footer { border-top: 1px solid var(--border); flex-wrap: wrap; }
  footer > span { flex: 1; }
  .error { margin: 8px 16px; }
</style>
