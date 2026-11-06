<script lang="ts">
  import DesktopApp from "$lib/DesktopApp.svelte";
  import MobileApp from "$lib/MobileApp.svelte";
  import { isMobile } from "$lib/platform";
  import { onMount } from "svelte";
  import { initializeStorage } from "$lib/storage";

  let ready = $state(false);
  let error = $state("");
  async function initialize() {
    error = "";
    try {
      await initializeStorage();
      ready = true;
    } catch (err) {
      error = String(err);
    }
  }
  onMount(() => { void initialize(); });
</script>

{#if error}
  <p role="alert">Не удалось загрузить настройки: {error}</p>
  <button onclick={initialize}>Повторить</button>
{:else if !ready}
  <p>Загрузка…</p>
{:else if isMobile}
  <MobileApp />
{:else}
  <DesktopApp />
{/if}
