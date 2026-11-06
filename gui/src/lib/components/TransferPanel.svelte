<script lang="ts">
  import { revealItemInDir } from "@tauri-apps/plugin-opener";
  import { api, errorText, type Transfer, type TransferFilter } from "$lib/api";
  import { explainError } from "$lib/errors";
  import { isMobile } from "$lib/platform";
  import { formatBytes } from "$lib/format";
  import { transfers } from "$lib/transfers.svelte";
  import Icon, { type IconName } from "./Icon.svelte";

  const DIRECTIONS: Record<Transfer["direction"], { icon: IconName; title: string }> = {
    upload: { icon: "upload", title: "Загрузка в облако" },
    download: { icon: "download", title: "Скачивание" },
    copy: { icon: "copy", title: "Копирование в облаке или между облаками" },
  };

  let collapsed = $state(false);
  let working = $state(false);
  let message = $state("");
  let actionError = $state("");
  const filters: [TransferFilter, string][] = [["all", "Все"], ["active", "Активные"], ["failed", "Ошибки"], ["done", "Готово"], ["cancelled", "Отменённые"]];
  const shownCount = $derived(transfers.filter === "all" ? transfers.total : transfers.filter === "active" ? transfers.activeCount : transfers.counts[transfers.filter]);

  async function act(action: () => Promise<unknown>) {
    if (working) return;
    working = true;
    actionError = message = "";
    try { await action(); }
    catch (err) { actionError = errorText(err); }
    finally { working = false; }
  }

  async function retryErrors() {
    const result = await api.retryFailedTransfers();
    message = `Поставлено на повтор: ${result.retried}`;
    if (result.errors.length) actionError = `Не удалось повторить: ${result.errors.length}. ${result.errors[0]}`;
    await transfers.refresh();
    transfers.kick();
  }

  const queued = $derived(transfers.counts.queued);
  const running = $derived(transfers.counts.running);
  const finished = $derived(transfers.total - transfers.activeCount);
  /** Transfers the backend only counted. */
  const hidden = $derived(Math.max(0, shownCount - transfers.viewItems.length));

  const summary = $derived.by(() => {
    const parts: string[] = [];
    if (running) parts.push(`идёт ${running}`);
    if (queued) parts.push(`в очереди ${queued}`);
    if (transfers.failedCount) parts.push(`ошибок ${transfers.failedCount}`);
    if (!parts.length) parts.push("всё завершено");
    return parts.join(" · ");
  });

  function percent(t: Transfer) {
    if (t.state === "done") return 100;
    return t.total > 0 ? Math.min(100, (t.done / t.total) * 100) : 0;
  }

  function detail(t: Transfer) {
    switch (t.state) {
      case "queued":
        return t.total ? `В очереди · ${formatBytes(t.total)}` : "В очереди";
      case "running": {
        const retry = t.attempt > 1 ? `попытка ${t.attempt} из ${t.max_attempts}` : "";
        if (!t.done && retry) return `Связь прервалась · ${retry}`;
        if (!t.done && transfers.waiting[t.id]) return "Ждём ответа сервера…";
        const speed = transfers.speeds[t.id];
        const parts = [t.total ? `${formatBytes(t.done)} из ${formatBytes(t.total)}` : formatBytes(t.done)];
        if (speed) parts.push(`${formatBytes(speed)}/с`);
        if (retry) parts.push(retry);
        return parts.join(" · ");
      }
      case "done":
        return `${t.unchanged ? "Без изменений" : "Готово"} · ${formatBytes(t.total || t.done)}`;
      case "cancelled":
        return "Отменено";
      case "failed":
        return explainError(t.error ?? "").summary || "Ошибка";
    }
  }
</script>

{#if transfers.total}
  <section class:collapsed>
    <header>
      <button class="toggle" onclick={() => (collapsed = !collapsed)} aria-expanded={!collapsed}>
        <span class="chev" class:up={collapsed}><Icon name="chevronDown" size={14} /></span>
        <strong>Передачи</strong>
        <span class="summary">{summary}</span>
      </button>
      <div class="actions">
        {#if transfers.activeCount}
          <button class="btn small" disabled={working} onclick={() => act(() => transfers.cancelAll())}>Отменить все</button>
        {/if}
        {#if transfers.failedCount}
          <button class="btn small" disabled={working} onclick={() => act(retryErrors)}>Повторить ошибки ({transfers.failedCount})</button>
        {/if}
        {#if finished}
          <button class="btn small" disabled={working} title="Убрать из списка завершённые, ошибочные и отменённые передачи" onclick={() => act(() => transfers.clear())}>{isMobile ? "Очистить" : "Очистить завершённые"}</button>
        {/if}
      </div>
    </header>

    {#if actionError || transfers.error}<div class="panel-error" role="alert">{actionError || errorText(transfers.error)}</div>{/if}
    {#if message}<div class="panel-message" role="status">{message}</div>{/if}
    {#if !collapsed}
      <nav class="filters" aria-label="Фильтр передач">
        {#each filters as [filter, label]}
          {@const count = filter === "all" ? transfers.total : filter === "active" ? transfers.activeCount : transfers.counts[filter]}
          <button class:chosen={transfers.filter === filter} aria-pressed={transfers.filter === filter}
            onclick={() => transfers.setFilter(filter)}>{label} <span>{count}</span></button>
        {/each}
      </nav>
      <ul>
        {#each transfers.viewItems as t (t.id)}
          <li class={t.state}>
            <span class="dir" title={DIRECTIONS[t.direction].title}>
              <Icon name={DIRECTIONS[t.direction].icon} size={14} />
            </span>
            <div class="main">
              <div class="line">
                <span class="name" title={`${t.account_name}: ${t.remote_path}\n${t.local_path}`}>{t.name}</span>
                <span class="account">{t.account_name}</span>
              </div>
              <div class="bar" role="progressbar" aria-label={t.name} aria-valuemin="0" aria-valuemax="100" aria-valuenow={percent(t)}><div style:width="{percent(t)}%"></div></div>
              <div class="detail" title={t.state === "failed" ? t.error : undefined}>{detail(t)}</div>
              {#if !isMobile}
                <details class="transfer-info">
                  <summary>Подробности</summary>
                  <div><strong>{t.direction === "download" ? "Куда" : "Откуда"}:</strong> {t.local_path}</div>
                  <div><strong>{t.direction === "download" ? "Откуда" : "Куда"}:</strong> {t.account_name}: {t.remote_path}</div>
                  {#if t.error}<div class="panel-error">{t.error}</div>{/if}
                </details>
              {/if}
            </div>
            <div class="row-actions">
              {#if t.state === "queued" || t.state === "running"}
                <button class="icon-btn" title="Отменить" aria-label="Отменить передачу" disabled={working} onclick={() => act(() => transfers.cancel(t.id))}>
                  <Icon name="close" size={14} />
                </button>
              {:else if t.state === "failed" || t.state === "cancelled"}
                <button class="icon-btn" title="Повторить" aria-label="Повторить передачу" disabled={working} onclick={() => act(() => transfers.retry(t.id))}>
                  <Icon name="retry" size={14} />
                </button>
              {:else if t.direction === "download" && isMobile}
                <button class="icon-btn" title="Открыть" aria-label="Открыть файл" onclick={() => act(() => api.localOpen(t.local_path))}>
                  <Icon name="file" size={14} />
                </button>
              {:else if t.direction === "download"}
                <button class="icon-btn" title="Показать в папке" aria-label="Показать в папке" onclick={() => act(() => revealItemInDir(t.local_path))}>
                  <Icon name="reveal" size={14} />
                </button>
              {/if}
            </div>
          </li>
        {:else}<li class="more">{transfers.error ? "Не удалось получить список передач" : "В этой категории передач нет"}</li>{/each}
        {#if hidden > 0}
          <li class="more">…и ещё {hidden}</li>
        {/if}
      </ul>
    {/if}
  </section>
{/if}

<style>
  .filters { display: flex; gap: 4px; padding: 5px 8px; border-bottom: 1px solid var(--border); overflow-x: auto; flex-shrink: 0; }
  .filters button { display: flex; gap: 5px; align-items: center; white-space: nowrap; background: none; border: 0; border-radius: 4px; padding: 4px 8px; color: var(--muted); }
  .filters button.chosen { background: var(--selected); color: var(--text); }
  .filters button:hover { background: var(--hover); }
  .filters span { font-size: 11px; }
  .panel-error { color: var(--danger); padding: 4px 12px; overflow-wrap: anywhere; }
  .panel-message { color: var(--muted); padding: 4px 12px; }
  .transfer-info { margin-top: 3px; color: var(--muted); font-size: 12px; overflow-wrap: anywhere; }
  .transfer-info summary { cursor: pointer; }
  .transfer-info div { padding-top: 4px; white-space: pre-wrap; }
  .transfer-info strong { font-weight: 500; }
  section {
    display: flex;
    flex-direction: column;
    max-height: 260px;
    border-top: 1px solid var(--border);
    background: var(--surface);
  }

  section.collapsed {
    max-height: none;
  }

  header {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 8px;
    border-bottom: 1px solid var(--border);
    flex-wrap: wrap;
  }

  section.collapsed header {
    border-bottom: 0;
  }

  .toggle {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    height: 28px;
    padding: 0 4px;
    border: 0;
    background: none;
    text-align: left;
  }

  .chev {
    display: inline-grid;
    color: var(--muted);
    transition: transform 0.15s;
  }

  .chev.up {
    transform: rotate(180deg);
  }

  .summary {
    overflow: hidden;
    color: var(--muted);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .actions {
    display: flex;
    gap: 6px;
  }

  :global(.btn.small) {
    height: 24px;
    padding: 0 8px;
    font-size: 12px;
  }

  ul {
    flex: 1;
    margin: 0;
    padding: 0;
    overflow-y: auto;
    list-style: none;
  }

  li {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 12px;
    border-bottom: 1px solid var(--border);
  }

  li.more {
    color: var(--muted);
    font-size: 12px;
  }

  li:last-child {
    border-bottom: 0;
  }

  .dir {
    display: inline-grid;
    color: var(--muted);
  }

  .main {
    flex: 1;
    min-width: 0;
  }

  .line {
    display: flex;
    gap: 8px;
    align-items: baseline;
  }

  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .account {
    flex: none;
    color: var(--muted);
    font-size: 12px;
  }

  .bar {
    height: 4px;
    margin: 4px 0 3px;
    border-radius: 2px;
    background: var(--hover);
    overflow: hidden;
  }

  .bar div {
    height: 100%;
    border-radius: 2px;
    background: var(--accent);
    transition: width 0.3s linear;
  }

  .done .bar div {
    background: var(--success);
  }

  .failed .bar div,
  .cancelled .bar div {
    background: var(--muted);
  }

  .detail {
    overflow: hidden;
    color: var(--muted);
    font-size: 12px;
    font-variant-numeric: tabular-nums;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .failed .detail {
    color: var(--danger);
  }

  .row-actions {
    flex: none;
    width: 28px;
  }
</style>
