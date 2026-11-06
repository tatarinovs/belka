<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { errorText } from "$lib/api";

  let { onclose }: {
    onclose: () => void;
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
  footer { display: flex; justify-content: flex-end; }
</style>
