<script lang="ts">
  let { open, onclose }: { open: boolean; onclose: () => void } = $props();

  let dialog: HTMLDialogElement;

  $effect(() => {
    if (open && !dialog.open) dialog.showModal();
    else if (!open && dialog.open) dialog.close();
  });

  const groups: { title: string; keys: [string, string][] }[] = [
    {
      title: "Файлы",
      keys: [
        ["Enter", "Открыть папку или файл"],
        ["Пробел", "Просмотр файла"],
        ["F2", "Переименовать"],
        ["F5", "Скачать или загрузить выбранное"],
        ["Ctrl+C · Ctrl+V", "Копировать в другую папку или облако"],
        ["F7 · Ctrl+Shift+N", "Новая папка"],
        ["Delete", "Удалить"],
        ["Ctrl+A", "Выделить всё"],
        ["Shift+↑↓", "Выделить диапазон"],
      ],
    },
    {
      title: "Навигация",
      keys: [
        ["Tab", "Перейти в другую панель"],
        ["Backspace", "На уровень вверх"],
        ["Ctrl+R", "Обновить"],
        ["Ctrl+F", "Найти в текущей папке"],
        ["Ctrl+L", "Ввести путь к папке"],
        ["Alt+← · Alt+→", "Назад и вперёд"],
        ["Alt+↑", "На уровень вверх"],
        ["Ctrl+Enter", "Открыть папку в новой вкладке"],
        ["Ctrl+T · Ctrl+W", "Новая и закрыть вкладку"],
        ["Ctrl+Tab", "Следующая вкладка"],
        ["Esc", "Снять выделение"],
        ["F1", "Это окно"],
      ],
    },
  ];
</script>

<dialog class="modal" bind:this={dialog} onclose={onclose} aria-labelledby="shortcuts-title">
  <div class="body">
    <h2 id="shortcuts-title">Горячие клавиши</h2>
    {#each groups as group (group.title)}
      <section>
        <h3>{group.title}</h3>
        <dl>
          {#each group.keys as [keys, action] (keys)}
            <dt><kbd>{keys}</kbd></dt>
            <dd>{action}</dd>
          {/each}
        </dl>
      </section>
    {/each}
    <footer><button class="btn primary" onclick={() => dialog.close()}>Закрыть</button></footer>
  </div>
</dialog>

<style>
  dialog {
    width: min(460px, calc(100vw - 32px));
  }

  .body {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 18px 20px;
  }

  h3 {
    margin: 0 0 6px;
    color: var(--muted);
    font-size: 12px;
    font-weight: 600;
  }

  dl {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 4px 16px;
    margin: 0;
  }

  dt,
  dd {
    margin: 0;
  }

  kbd {
    padding: 1px 6px;
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--bg);
    font: inherit;
    font-size: 12px;
    white-space: nowrap;
  }

  footer {
    display: flex;
    justify-content: flex-end;
  }
</style>
