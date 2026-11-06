<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { errorText } from "$lib/api";
  import Icon from "./Icon.svelte";

  let { onclose, onbackup, onabout }: {
    onclose: () => void;
    onbackup: () => void;
    onabout: () => void;
  } = $props();

  let dialog: HTMLDialogElement;
  let settings = $state<{ windows: boolean; close_to_tray: boolean; autostart: boolean } | null>(null);
  let busy = $state(false);
  let error = $state("");

  onMount(() => {
    dialog.showModal();
    invoke<NonNullable<typeof settings>>("get_desktop_settings")
      .then((value) => (settings = value))
      .catch((err) => (error = errorText(err)));
  });

  async function change(key: "close_to_tray" | "autostart", event: Event) {
    const input = event.currentTarget as HTMLInputElement;
    const enabled = input.checked;
    busy = true;
    error = "";
    try {
      await invoke(key === "autostart" ? "set_autostart" : "set_close_to_tray", { enabled });
      if (settings) settings[key] = enabled;
    } catch (err) {
      input.checked = settings?.[key] ?? false;
      error = errorText(err);
    } finally {
      busy = false;
    }
  }

  function navigate(action: () => void) {
    dialog.close();
    action();
  }
</script>

<dialog class="modal" bind:this={dialog} onclose={onclose} aria-labelledby="settings-title">
  <div class="body">
    <h2 id="settings-title">Настройки</h2>
    {#if settings?.windows}
      <section class="preferences" aria-label="Поведение приложения">
        <label class="preference">
          <input type="checkbox" checked={settings.close_to_tray} disabled={busy} aria-describedby="tray-help" onchange={(event) => change("close_to_tray", event)} />
          <strong>Сворачивать в трей</strong>
          <small id="tray-help">При закрытии окна передачи и автобэкап продолжаются. Выход — через меню значка в трее.</small>
        </label>
        <label class="preference">
          <input type="checkbox" checked={settings.autostart} disabled={busy} aria-describedby="startup-help" onchange={(event) => change("autostart", event)} />
          <strong>Запуск при загрузке Windows</strong>
          <small id="startup-help">Открывать Belka при входе в Windows.</small>
        </label>
      </section>
    {:else if !settings && !error}
      <p>Загрузка настроек…</p>
    {/if}
    {#if error}<div class="error" role="alert">{error}</div>{/if}
    <section class="links" aria-label="Дополнительно">
      <button class="navigation" onclick={() => navigate(onbackup)}>
        <Icon name="clock" size={18} />
        <span class="navigation-text"><strong>Автобэкап</strong><small>Расписание копирования папок в облако</small></span>
        <Icon name="chevron" size={16} />
      </button>
      <button class="navigation" onclick={() => navigate(onabout)}>
        <Icon name="info" size={18} />
        <span class="navigation-text"><strong>О приложении</strong><small>Версия и проверка обновлений</small></span>
        <Icon name="chevron" size={16} />
      </button>
    </section>
    <footer><button class="btn primary" onclick={() => dialog.close()}>Закрыть</button></footer>
  </div>
</dialog>

<style>
  dialog { width: min(460px, calc(100vw - 32px)); }
  .body { display: flex; flex-direction: column; gap: 20px; padding: 22px 24px 18px; }
  .preferences { display: flex; flex-direction: column; gap: 20px; }
  .modal .preference {
    display: grid;
    grid-template-columns: 16px minmax(0, 1fr);
    align-items: start;
    gap: 5px 10px;
    cursor: pointer;
  }
  .preference input { width: 15px; height: 15px; margin: 2px 0 0; accent-color: var(--accent); }
  strong { font-size: 13px; font-weight: 500; line-height: 19px; }
  .preference small { grid-column: 2; }
  small, p { color: var(--muted); font-size: 12px; line-height: 1.5; }
  p { margin: 0; }
  .links { border-top: 1px solid var(--border); padding-top: 10px; display: flex; flex-direction: column; gap: 2px; }
  .navigation {
    display: grid;
    grid-template-columns: 18px minmax(0, 1fr) 16px;
    align-items: center;
    gap: 12px;
    width: 100%;
    padding: 10px 8px;
    border: 0;
    border-radius: 6px;
    background: transparent;
    text-align: left;
  }
  .navigation:hover { background: var(--hover); }
  .navigation > :global(svg) { color: var(--muted); }
  .navigation-text { display: flex; flex-direction: column; gap: 2px; }
  footer { display: flex; justify-content: flex-end; }
</style>
