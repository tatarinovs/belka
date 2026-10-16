<!--
  Backup tasks: each copies a local folder into a cloud folder on a schedule,
  uploading only new and changed files. On a phone the screen covers the app
  and folders are chosen in the main pane, so the task being edited (`draft`)
  belongs to the page and survives this screen being hidden meanwhile. On a
  desktop it is a dialog: the local folder comes from the system dialog, the
  cloud folder is typed or taken from the open cloud tab.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { ask, open as openDialog } from "@tauri-apps/plugin-dialog";
  import { api, errorText, newBackupTask, type Account, type BackupRun, type BackupTask } from "$lib/api";
  import { formatBytes, formatDate, plural } from "$lib/format";
  import { providerDef } from "$lib/providers";
  import Icon from "./Icon.svelte";

  let {
    accounts,
    storageOk,
    draft = $bindable(),
    desktop = false,
    currentCloud = null,
    onclose,
    onpick,
    onstorage,
    ontoast,
  }: {
    accounts: Account[];
    storageOk: boolean;
    /** The task being edited; null while the list is shown. */
    draft: BackupTask | null;
    /** The dialog layout of the desktop app. */
    desktop?: boolean;
    /** Desktop: the folder open in the active cloud tab. */
    currentCloud?: { accountId: string; path: string } | null;
    onclose: () => void;
    /** Phone: choose a folder of the draft in the main pane. */
    onpick?: (side: "local" | "remote") => void;
    onstorage: () => void;
    ontoast: (text: string, error?: boolean) => void;
  } = $props();

  const INTERVALS: [number, string][] = [
    [1, "Каждый час"],
    [3, "Каждые 3 часа"],
    [6, "Каждые 6 часов"],
    [12, "Каждые 12 часов"],
    [24, "Раз в сутки"],
    [72, "Раз в 3 дня"],
    [168, "Раз в неделю"],
  ];
  /** Days, the choice, and "older than …" for the confirmation. */
  const FREE_AFTER: [number, string, string][] = [
    [7, "через неделю", "недели"],
    [30, "через месяц", "месяца"],
    [90, "через 3 месяца", "3 месяцев"],
    [365, "через год", "года"],
  ];
  const PHONE_ROOT = /^\/storage\/emulated\/0(\/|$)/;

  let tasks = $state<BackupTask[]>([]);
  let loaded = $state(false);
  let saving = $state(false);
  let error = $state("");
  /** Filters as the user types them: patterns separated by commas. */
  let includeText = $state("");
  let excludeText = $state("");
  /** A run whose failed files are unfolded in the history. */
  let openRun = $state<string | null>(null);

  const editingSaved = $derived(!!draft?.id);

  async function refresh() {
    try {
      tasks = await api.backupTasks();
      if (!draft) error = "";
    } catch (err) {
      error = errorText(err);
    } finally {
      loaded = true;
    }
  }

  onMount(() => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    let alive = true;
    // Faster while something runs; a run started by the schedule shows up too.
    const tick = async () => {
      await refresh();
      if (alive) timer = setTimeout(tick, tasks.some((t) => t.running) ? 1000 : 4000);
    };
    tick();
    return () => {
      alive = false;
      clearTimeout(timer);
    };
  });

  // The filter fields follow the draft that is opened.
  let filtersOf: BackupTask | null = null;
  $effect(() => {
    if (draft && draft !== filtersOf) {
      filtersOf = draft;
      includeText = draft.include.join(", ");
      excludeText = draft.exclude.join(", ");
    }
  });

  function patterns(text: string): string[] {
    return text
      .split(/[,\n]/)
      .map((p) => p.trim())
      .filter(Boolean);
  }

  function newTask() {
    const cloud = desktop && currentCloud ? currentCloud : null;
    draft = newBackupTask("", cloud?.accountId ?? accounts[0]?.id ?? "", cloud?.path ?? "");
    error = "";
  }

  function edit(task: BackupTask) {
    draft = { ...task };
    error = "";
  }

  function localLabel(path: string) {
    return desktop ? path : path.replace(PHONE_ROOT, "/") || "/";
  }

  function accountOf(id: string) {
    return accounts.find((a) => a.id === id) ?? null;
  }

  function intervalLabel(hours: number) {
    return INTERVALS.find(([h]) => h === hours)?.[1] ?? `Каждые ${hours} ч`;
  }

  function conditions(task: BackupTask) {
    const parts = [intervalLabel(task.interval_hours).toLowerCase()];
    if (!desktop && task.wifi_only) parts.push("по Wi‑Fi");
    if (!desktop && task.charging_only) parts.push("на зарядке");
    if (task.include.length || task.exclude.length) parts.push("с фильтром");
    if (task.free_after_days) parts.push("освобождает место");
    return parts.join(", ");
  }

  function runText(run: BackupRun): { text: string; error?: boolean } {
    const when = formatDate(run.finished);
    if (run.cancelled) return { text: `${when} · остановлен` };
    if (run.error) return { text: `${when} · ${run.error}`, error: true };
    const parts = [
      run.uploaded
        ? `загружено ${plural(run.uploaded, "файл", "файла", "файлов")} (${formatBytes(run.bytes)})`
        : "новых файлов нет",
    ];
    if (run.freed) parts.push(`освобождено ${formatBytes(run.freed_bytes)}`);
    if (run.failed) parts.push(`не удалось: ${run.failed}`);
    return { text: `${when} · ${parts.join(", ")}`, error: run.failed > 0 };
  }

  function status(task: BackupTask): { text: string; error?: boolean } {
    const last = task.history[0];
    if (!last) return { text: task.enabled ? "Ещё не запускался" : "Выключен" };
    return runText(last);
  }

  async function run(task: BackupTask) {
    try {
      await api.backupRun(task.id);
      ontoast(`Автобэкап «${task.name}» запущен`);
      setTimeout(refresh, 1500);
    } catch (err) {
      ontoast(errorText(err), true);
    }
  }

  async function stop(task: BackupTask) {
    await api.backupCancel(task.id).catch((err) => ontoast(errorText(err), true));
    refresh();
  }

  async function toggle(task: BackupTask) {
    try {
      await api.backupSave({ ...task, enabled: !task.enabled });
      await refresh();
    } catch (err) {
      ontoast(errorText(err), true);
    }
  }

  async function pickLocal() {
    if (!desktop) {
      onpick?.("local");
      return;
    }
    const path = await openDialog({ directory: true, title: "Папка для автобэкапа", defaultPath: draft?.local || undefined });
    if (typeof path === "string" && draft) draft.local = path;
  }

  function takeCurrentCloud() {
    if (draft && currentCloud) {
      draft.account_id = currentCloud.accountId;
      draft.remote = currentCloud.path;
    }
  }

  async function save(event: SubmitEvent) {
    event.preventDefault();
    if (!draft || saving) return;
    if (!draft.local) {
      error = desktop ? "Выберите папку на компьютере" : "Выберите папку на телефоне";
      return;
    }
    if (!draft.account_id || !draft.remote.trim()) {
      error = "Выберите папку в облаке";
      return;
    }
    if (draft.free_after_days) {
      const confirmed = await ask(
        `Файлы старше ${FREE_AFTER.find(([d]) => d === draft!.free_after_days)?.[2] ?? plural(draft.free_after_days, "дня", "дней", "дней")} будут удаляться с телефона, если они уже есть в облаке с тем же размером.\n\nВосстановить их можно только из облака. Продолжить?`,
        { title: "Освобождение места", kind: "warning", okLabel: "Да, удалять", cancelLabel: "Отмена" },
      );
      if (!confirmed) return;
    }
    saving = true;
    try {
      const isNew = !draft.id;
      const saved = await api.backupSave({ ...draft, include: patterns(includeText), exclude: patterns(excludeText) });
      draft = null;
      error = "";
      await refresh();
      if (isNew && saved.enabled) {
        ontoast(
          desktop
            ? "Задача сохранена. Бэкап запускается, пока Belka открыта, или из планировщика: belka task run --due"
            : saved.wifi_only
              ? "Задача сохранена. Первый бэкап начнётся при подключении к Wi‑Fi"
              : "Задача сохранена. Первый бэкап начнётся в ближайшее время",
        );
      }
    } catch (err) {
      error = errorText(err);
    } finally {
      saving = false;
    }
  }

  async function remove() {
    if (!draft?.id) return;
    const task = draft;
    const confirmed = await ask(`Удалить задачу «${task.name}»?\n\nУже загруженные файлы останутся в облаке.`, {
      title: "Автобэкап",
      kind: "warning",
      okLabel: "Удалить",
      cancelLabel: "Отмена",
    });
    if (!confirmed) return;
    try {
      await api.backupRemove(task.id);
      draft = null;
      await refresh();
    } catch (err) {
      error = errorText(err);
    }
  }

  function cancelEdit() {
    draft = null;
    error = "";
  }

  /** The saved version of the task being edited, for its history. */
  const savedDraft = $derived(draft?.id ? (tasks.find((t) => t.id === draft!.id) ?? null) : null);
</script>

<svelte:window
  onkeydown={(e) => {
    if (desktop && e.key === "Escape") {
      e.preventDefault();
      if (draft) cancelEdit();
      else onclose();
    }
  }}
/>

{#if desktop}
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="backdrop" onclick={onclose}></div>
{/if}

<div class="screen" class:desktop>
  <header class="bar">
    {#if !desktop || draft}
      <button class="tool" aria-label="Назад" onclick={draft ? cancelEdit : onclose}>
        <Icon name="arrowLeft" size={22} />
      </button>
    {/if}
    <div class="title" class:padded={desktop && !draft}>
      <strong>{draft ? (editingSaved ? "Задача автобэкапа" : "Новая задача") : "Автобэкап"}</strong>
    </div>
    {#if !draft}
      <button class="tool" aria-label="Новая задача" title="Новая задача" onclick={newTask} disabled={!accounts.length}>
        <Icon name="plus" size={22} />
      </button>
      {#if desktop}
        <button class="tool" aria-label="Закрыть" onclick={onclose}>
          <Icon name="close" size={20} />
        </button>
      {/if}
    {/if}
  </header>

  {#if !storageOk}
    <div class="notice">
      <span>Чтобы читать папки телефона в фоне, нужен доступ ко всем файлам.</span>
      <button class="btn primary small" onclick={onstorage}>Разрешить</button>
    </div>
  {/if}

  {#if draft}
    {@const acc = accountOf(draft.account_id)}
    <form class="editor" onsubmit={save}>
      <label>
        <span>Название</span>
        <input bind:value={draft.name} placeholder="По имени папки" spellcheck="false" autocomplete="off" />
      </label>

      <div class="field">
        <span>Что копировать</span>
        <button type="button" class="choose" onclick={pickLocal}>
          <Icon name={desktop ? "folder" : "phone"} size={20} />
          <span class="grow">
            {draft.local ? localLabel(draft.local) : desktop ? "Выбрать папку на компьютере" : "Выбрать папку на телефоне"}
          </span>
          <Icon name="chevron" size={18} />
        </button>
      </div>

      <div class="field">
        <span>Куда</span>
        {#if desktop}
          <div class="row-fields">
            <select bind:value={draft.account_id}>
              {#each accounts as a (a.id)}
                <option value={a.id}>{a.name}</option>
              {/each}
            </select>
            <input bind:value={draft.remote} placeholder="/Backups/Документы" spellcheck="false" autocomplete="off" />
          </div>
          {#if currentCloud}
            <button type="button" class="btn link" onclick={takeCurrentCloud}>
              Взять папку из открытой вкладки облака
            </button>
          {/if}
        {:else}
          <button type="button" class="choose" onclick={() => onpick?.("remote")}>
            {#if acc && draft.remote}
              <span class="badge" data-kind={acc.kind}>{providerDef(acc.kind).short}</span>
              <span class="grow">{acc.name}: {draft.remote}</span>
            {:else}
              <Icon name="cloud" size={20} />
              <span class="grow">Выбрать папку в облаке</span>
            {/if}
            <Icon name="chevron" size={18} />
          </button>
        {/if}
      </div>

      <label>
        <span>Как часто</span>
        <select bind:value={draft.interval_hours}>
          {#each INTERVALS as [hours, label] (hours)}
            <option value={hours}>{label}</option>
          {/each}
        </select>
      </label>

      <label>
        <span>Только эти файлы (маски через запятую, пусто — все)</span>
        <input bind:value={includeText} placeholder="*.jpg, *.mp4" spellcheck="false" autocomplete="off" />
      </label>
      <label>
        <span>Пропускать (файлы и папки)</span>
        <input bind:value={excludeText} placeholder="*.tmp, cache" spellcheck="false" autocomplete="off" />
      </label>

      {#if !desktop}
        <label class="check">
          <input type="checkbox" bind:checked={draft.wifi_only} />
          <span>Только по Wi‑Fi</span>
        </label>
        <label class="check">
          <input type="checkbox" bind:checked={draft.charging_only} />
          <span>Только во время зарядки</span>
        </label>
        <label class="check">
          <input
            type="checkbox"
            checked={!!draft.free_after_days}
            onchange={(e) => (draft!.free_after_days = e.currentTarget.checked ? 30 : null)}
          />
          <span>Освобождать место на телефоне</span>
        </label>
        {#if draft.free_after_days}
          <label class="sub">
            <span>Удалять с телефона файлы, которые уже в облаке,</span>
            <select bind:value={draft.free_after_days}>
              {#each FREE_AFTER as [days, label] (days)}
                <option value={days}>{label}</option>
              {/each}
            </select>
          </label>
        {/if}
      {/if}
      <label class="check">
        <input type="checkbox" bind:checked={draft.enabled} />
        <span>Включена</span>
      </label>

      <p class="help">
        Загружаются только новые и изменённые файлы, подпапки сохраняются. В облаке ничего не удаляется,
        даже если удалить файл {desktop ? "на компьютере" : "на телефоне"}. Скрытые файлы пропускаются{desktop
          ? "."
          : ", в том числе корзина галереи."}
        {#if desktop}
          Задачи выполняются, пока Belka открыта; чтобы бэкап шёл и без неё, добавьте в Планировщик заданий
          команду <code>belka task run --due</code> раз в час.
        {/if}
      </p>

      {#if error}<div class="error">{error}</div>{/if}

      <footer>
        {#if editingSaved}
          <button type="button" class="btn big danger-text" onclick={remove} disabled={saving}>Удалить</button>
        {/if}
        <span class="grow"></span>
        <button type="button" class="btn big" onclick={cancelEdit} disabled={saving}>Отмена</button>
        <button type="submit" class="btn primary big" disabled={saving}>Сохранить</button>
      </footer>

      {#if savedDraft?.history.length}
        <section class="history">
          <h3>История</h3>
          {#each savedDraft.history as h, i (i)}
            {@const t = runText(h)}
            {@const key = `${h.started}`}
            <div class="run">
              <button
                type="button"
                class="run-line"
                class:error-text={t.error}
                disabled={!h.failures.length}
                onclick={() => (openRun = openRun === key ? null : key)}
              >
                {t.text}
                {#if h.failures.length}<Icon name={openRun === key ? "chevronDown" : "chevron"} size={14} />{/if}
              </button>
              {#if openRun === key}
                <ul class="failures">
                  {#each h.failures as [file, reason], j (j)}
                    <li><strong>{file}</strong>: {reason}</li>
                  {/each}
                  {#if h.failed > h.failures.length}
                    <li>…и ещё {h.failed - h.failures.length}</li>
                  {/if}
                </ul>
              {/if}
            </div>
          {/each}
        </section>
      {/if}
    </form>
  {:else}
    <div class="list">
      {#if error}<div class="error banner">{error}</div>{/if}
      {#each tasks as task (task.id)}
        {@const acc = accountOf(task.account_id)}
        {@const st = status(task)}
        <div class="task" class:off={!task.enabled}>
          <button class="task-main" onclick={() => edit(task)}>
            <span class="task-name">{task.name}</span>
            <span class="meta">{localLabel(task.local)} → {acc ? `${acc.name}: ${task.remote}` : "аккаунт удалён"}</span>
            <span class="meta">{task.enabled ? conditions(task) : "Выключена"}</span>
            {#if task.running}
              {@const r = task.running}
              <span class="meta">
                {#if r.scanning}
                  Поиск новых файлов…
                {:else}
                  {r.files_done} из {r.files_total} · {formatBytes(r.bytes_done)} из {formatBytes(r.bytes_total)}
                  {#if r.current}· {r.current}{/if}
                {/if}
              </span>
              <span class="bar-track">
                <span
                  class="bar-fill"
                  class:indeterminate={r.scanning || !r.bytes_total}
                  style:width={r.bytes_total ? `${Math.min(100, (r.bytes_done / r.bytes_total) * 100)}%` : undefined}
                ></span>
              </span>
            {:else}
              <span class="meta status" class:error-text={st.error}>{st.text}</span>
            {/if}
          </button>
          <div class="task-actions">
            {#if task.running}
              <button class="tool" aria-label="Остановить" title="Остановить" onclick={() => stop(task)}>
                <Icon name="stop" size={20} />
              </button>
            {:else}
              <button class="tool" aria-label="Запустить сейчас" title="Запустить сейчас" onclick={() => run(task)} disabled={!acc}>
                <Icon name="play" size={20} />
              </button>
            {/if}
            <label class="switch" title={task.enabled ? "Выключить" : "Включить"}>
              <input type="checkbox" checked={task.enabled} onchange={() => toggle(task)} aria-label="Включена" />
              <span></span>
            </label>
          </div>
        </div>
      {:else}
        {#if loaded}
          <div class="empty">
            <Icon name="clock" size={48} />
            <h1>Автобэкап</h1>
            <p>
              {#if desktop}
                Выберите папку на компьютере и папку в облаке. Belka будет регулярно загружать туда новые и изменённые
                файлы.
              {:else}
                Выберите папку на телефоне, например с фотографиями, и папку в облаке. Belka будет регулярно загружать
                туда новые файлы, даже когда приложение закрыто.
              {/if}
            </p>
            {#if accounts.length}
              <button class="btn primary big" onclick={newTask}>Новая задача</button>
            {:else}
              <p>Сначала подключите облако.</p>
            {/if}
          </div>
        {/if}
      {/each}
    </div>
  {/if}
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 29;
    background: rgb(0 0 0 / 0.35);
  }

  .screen {
    position: fixed;
    inset: 0;
    z-index: 30;
    display: flex;
    flex-direction: column;
    background: var(--surface);
    font-size: 15px;
  }

  .screen.desktop {
    inset: 6vh 0 auto;
    width: min(680px, calc(100vw - 32px));
    max-height: 88vh;
    margin: 0 auto;
    border: 1px solid var(--border);
    border-radius: 10px;
    box-shadow: var(--shadow);
    font-size: 14px;
  }

  .bar {
    display: flex;
    align-items: center;
    gap: 4px;
    min-height: 56px;
    padding: 4px;
    border-bottom: 1px solid var(--border);
  }

  .desktop .bar {
    min-height: 48px;
  }

  .title {
    flex: 1;
    min-width: 0;
  }

  .title.padded {
    padding-left: 14px;
  }

  .title strong {
    font-size: 19px;
    font-weight: 600;
  }

  .desktop .title strong {
    font-size: 16px;
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

  .desktop .tool {
    width: 36px;
    height: 36px;
  }

  .tool:hover:not(:disabled),
  .tool:active:not(:disabled) {
    background: var(--hover);
  }

  .tool:disabled {
    opacity: 0.35;
  }

  .notice {
    display: flex;
    align-items: center;
    gap: 10px;
    margin: 8px 12px 0;
    padding: 10px 12px;
    border-radius: 10px;
    background: var(--selected);
  }

  .notice span {
    flex: 1;
  }

  .list,
  .editor {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    overscroll-behavior: contain;
  }

  .task {
    display: flex;
    align-items: center;
    gap: 4px;
    padding-right: 8px;
    border-bottom: 1px solid var(--border);
  }

  .task.off .task-main {
    opacity: 0.55;
  }

  .task-main {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
    padding: 12px 8px 12px 16px;
    border: 0;
    background: none;
    color: var(--text);
    text-align: left;
  }

  .task-main:hover,
  .task-main:active {
    background: var(--hover);
  }

  .task-name {
    font-weight: 600;
  }

  .meta {
    color: var(--muted);
    font-size: 12.5px;
    overflow-wrap: anywhere;
  }

  .error-text,
  .status.error-text {
    color: var(--danger);
  }

  .task-actions {
    flex: none;
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .bar-track {
    display: block;
    height: 4px;
    margin-top: 4px;
    overflow: hidden;
    border-radius: 2px;
    background: var(--border);
  }

  .bar-fill {
    display: block;
    height: 100%;
    background: var(--accent);
    transition: width 0.4s;
  }

  .bar-fill.indeterminate {
    width: 40%;
    animation: slide 1.2s linear infinite;
  }

  @keyframes slide {
    from {
      transform: translateX(-100%);
    }
    to {
      transform: translateX(250%);
    }
  }

  /* A toggle switch over a real checkbox. */
  .switch {
    position: relative;
    display: inline-block;
    width: 44px;
    height: 26px;
  }

  .switch input {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    margin: 0;
    opacity: 0;
  }

  .switch span {
    position: absolute;
    inset: 0;
    border-radius: 13px;
    background: var(--border);
    transition: background 0.15s;
    pointer-events: none;
  }

  .switch span::after {
    content: "";
    position: absolute;
    top: 3px;
    left: 3px;
    width: 20px;
    height: 20px;
    border-radius: 50%;
    background: #fff;
    box-shadow: 0 1px 2px rgb(0 0 0 / 0.3);
    transition: transform 0.15s;
  }

  .switch input:checked + span {
    background: var(--accent);
  }

  .switch input:checked + span::after {
    transform: translateX(18px);
  }

  .editor {
    display: flex;
    flex-direction: column;
    gap: 14px;
    padding: 16px;
  }

  .editor label,
  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .editor label > span,
  .field > span {
    color: var(--muted);
    font-size: 13px;
  }

  .editor input:not([type="checkbox"]),
  .editor select {
    height: 44px;
    padding: 0 12px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--surface);
    color: var(--text);
    font-size: 16px;
  }

  .desktop .editor input:not([type="checkbox"]),
  .desktop .editor select {
    height: 34px;
    font-size: 14px;
  }

  .row-fields {
    display: flex;
    gap: 8px;
  }

  .row-fields select {
    flex: 0 0 40%;
  }

  .row-fields input {
    flex: 1;
    min-width: 0;
  }

  .editor label.check {
    flex-direction: row;
    align-items: center;
    gap: 12px;
    min-height: 36px;
  }

  .editor label.check > span {
    color: var(--text);
    font-size: 15px;
  }

  .editor label.sub {
    margin-top: -6px;
    padding-left: 32px;
  }

  .check input {
    width: 20px;
    height: 20px;
    margin: 0;
  }

  .choose {
    display: flex;
    align-items: center;
    gap: 12px;
    min-height: 52px;
    padding: 8px 12px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--surface);
    color: var(--text);
    text-align: left;
    overflow-wrap: anywhere;
  }

  .desktop .choose {
    min-height: 40px;
  }

  .choose:hover,
  .choose:active {
    background: var(--hover);
  }

  .btn.link {
    align-self: flex-start;
    height: auto;
    padding: 0;
    border: 0;
    background: none;
    color: var(--accent);
  }

  .badge {
    width: 22px;
    height: 22px;
    border-radius: 6px;
    font-size: 11px;
  }

  .help {
    margin: 0;
    color: var(--muted);
    font-size: 13px;
  }

  .help code {
    font-size: 12px;
  }

  .error {
    color: var(--danger);
  }

  .banner {
    margin: 12px;
  }

  footer {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    padding-top: 4px;
  }

  .grow {
    flex: 1;
    min-width: 0;
  }

  .btn.big {
    justify-content: center;
    height: 44px;
    padding: 0 18px;
    border-radius: 22px;
    font-size: 15px;
  }

  .desktop .btn.big {
    height: 34px;
    border-radius: 6px;
    font-size: 14px;
  }

  .btn.small {
    height: 32px;
    border-radius: 16px;
  }

  .danger-text {
    color: var(--danger);
  }

  .history h3 {
    margin: 8px 0 4px;
    font-size: 14px;
    font-weight: 600;
  }

  .run-line {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    padding: 6px 0;
    border: 0;
    border-top: 1px solid var(--border);
    background: none;
    color: var(--muted);
    font-size: 13px;
    text-align: left;
  }

  .run-line:disabled {
    cursor: default;
  }

  .failures {
    margin: 0 0 8px;
    padding-left: 18px;
    font-size: 12.5px;
    overflow-wrap: anywhere;
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

  .empty .btn.big {
    width: min(300px, 100%);
  }
</style>
