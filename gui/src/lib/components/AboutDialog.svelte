<script lang="ts">
  import { getVersion } from "@tauri-apps/api/app";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { api, errorText, type Update } from "$lib/api";
  import logo from "../../../src-tauri/icons/128x128@2x.png";
  import Icon from "./Icon.svelte";

  let { open, onclose }: { open: boolean; onclose: () => void } = $props();

  const SITE = "https://tatarinovs.github.io/belka/";

  let dialog: HTMLDialogElement;
  let version = $state("");
  let checking = $state(false);
  /** The result of the last check: `null` when this version is the latest. */
  let update = $state<Update | null | undefined>(undefined);
  let error = $state("");

  $effect(() => {
    if (open && !dialog.open) {
      error = "";
      dialog.showModal();
    } else if (!open && dialog.open) {
      dialog.close();
    }
  });

  getVersion().then((v) => (version = v));

  async function checkUpdate() {
    checking = true;
    error = "";
    try {
      update = await api.checkUpdate();
    } catch (err) {
      error = errorText(err);
    } finally {
      checking = false;
    }
  }

  function visit(url: string) {
    openUrl(url).catch((err) => (error = errorText(err)));
  }

  /** The webview would open the link inside the app; the system browser is wanted. */
  function openSite(event: MouseEvent) {
    event.preventDefault();
    visit(SITE);
  }
</script>

<dialog class="modal" bind:this={dialog} onclose={onclose}>
  <div class="body">
    <button class="icon-btn close" title="Закрыть" onclick={() => dialog.close()}>
      <Icon name="close" />
    </button>

    <img src={logo} alt="" width="112" height="112" />
    <h2>Belka</h2>
    {#if version}<p class="version">Версия {version}</p>{/if}
    <p class="help">Бэкапы в облачные хранилища и файловый менеджер для облаков.</p>
    <a href={SITE} onclick={openSite}>tatarinovs.github.io/belka</a>

    {#if error}
      <div class="error">{error}</div>
    {:else if update === null}
      <div class="info">Установлена последняя версия</div>
    {/if}

    {#if update}
      <button class="btn primary" onclick={() => visit(update!.page)}>
        <Icon name="download" size={14} /> Скачать версию {update.version}
      </button>
    {:else}
      <button class="btn" onclick={checkUpdate} disabled={checking}>
        {checking ? "Проверка…" : "Проверить обновления"}
      </button>
    {/if}
  </div>
</dialog>

<style>
  dialog {
    width: min(360px, calc(100vw - 32px));
  }

  .body {
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    padding: 28px 24px 22px;
    text-align: center;
  }

  .close {
    position: absolute;
    top: 8px;
    right: 8px;
  }

  img {
    margin-bottom: 4px;
    border-radius: 24px;
  }

  h2 {
    font-size: 20px;
  }

  p {
    margin: 0;
  }

  .version,
  .help {
    color: var(--muted);
  }

  .help {
    font-size: 12px;
  }

  a {
    color: var(--accent);
    text-decoration: none;
  }

  a:hover {
    text-decoration: underline;
  }

  .info {
    padding: 8px 10px;
    border-radius: 6px;
    background: var(--hover);
  }

  .error,
  .info {
    align-self: stretch;
  }

  .btn {
    margin-top: 8px;
  }
</style>
