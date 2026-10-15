<script lang="ts">
  import DesktopApp from "$lib/DesktopApp.svelte";
  import MobileApp from "$lib/MobileApp.svelte";
  import { isMobile } from "$lib/platform";
</script>

{#if isMobile}
  <MobileApp />
{:else}
  <DesktopApp />
{/if}
