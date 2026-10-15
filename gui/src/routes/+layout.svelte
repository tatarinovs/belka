<script lang="ts">
  import "../app.css";

  let { children } = $props();

  // The webview's own menu (Reload, Inspect…) is meaningless in the app;
  // text fields keep it for copy and paste.
  function oncontextmenu(event: MouseEvent) {
    const target = event.target as HTMLElement;
    if (!target.closest("input, textarea")) event.preventDefault();
  }
</script>

<svelte:document {oncontextmenu} />

{@render children()}
