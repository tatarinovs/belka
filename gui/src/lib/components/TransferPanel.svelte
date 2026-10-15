<script lang="ts">
  import { revealItemInDir } from "@tauri-apps/plugin-opener";
  import type { Transfer } from "$lib/api";
  import { formatBytes } from "$lib/format";
  import { transfers } from "$lib/transfers.svelte";
  import Icon from "./Icon.svelte";

  let collapsed = $state(false);

  const queued = $derived(transfers.counts.queued);
  const running = $derived(transfers.counts.running);
  const finished = $derived(transfers.total - transfers.activeCount);
  /** Transfers the backend only counted. */
  const hidden = $derived(transfers.total - transfers.items.length);

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
        return `Готово · ${formatBytes(t.total || t.done)}`;
      case "cancelled":
        return "Отменено";
      case "failed":
        return t.error ?? "Ошибка";
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
          <button class="btn small" onclick={() => transfers.cancelAll()}>Отменить все</button>
        {/if}
        {#if finished}
          <button class="btn small" onclick={() => transfers.clear()}>Очистить завершённые</button>
        {/if}
      </div>
    </header>

    {#if !collapsed}
      <ul>
        {#each transfers.items as t (t.id)}
          <li class={t.state}>
            <span class="dir" title={t.direction === "upload" ? "Загрузка в облако" : "Скачивание"}>
              <Icon name={t.direction === "upload" ? "upload" : "download"} size={14} />
            </span>
            <div class="main">
              <div class="line">
                <span class="name" title={`${t.account_name}: ${t.remote_path}\n${t.local_path}`}>{t.name}</span>
                <span class="account">{t.account_name}</span>
              </div>
              <div class="bar"><div style:width="{percent(t)}%"></div></div>
              <div class="detail" title={t.state === "failed" ? t.error : undefined}>{detail(t)}</div>
            </div>
            <div class="row-actions">
              {#if t.state === "queued" || t.state === "running"}
                <button class="icon-btn" title="Отменить" onclick={() => transfers.cancel(t.id)}>
                  <Icon name="close" size={14} />
                </button>
              {:else if t.state === "failed" || t.state === "cancelled"}
                <button class="icon-btn" title="Повторить" onclick={() => transfers.retry(t.id)}>
                  <Icon name="retry" size={14} />
                </button>
              {:else if t.direction === "download"}
                <button class="icon-btn" title="Показать в папке" onclick={() => revealItemInDir(t.local_path)}>
                  <Icon name="reveal" size={14} />
                </button>
              {/if}
            </div>
          </li>
        {/each}
        {#if hidden > 0}
          <li class="more">…и ещё {hidden}</li>
        {/if}
      </ul>
    {/if}
  </section>
{/if}

<style>
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
    font-size: 11px;
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
    font-size: 11px;
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
