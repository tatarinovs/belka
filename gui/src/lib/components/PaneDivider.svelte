<script lang="ts">
  let { value, min = 25, max = 75, onchange }: { value: number; min?: number; max?: number; onchange: (value: number) => void } = $props();
  let handle: HTMLDivElement;
  let dragging = $state(false);
  function move(e: PointerEvent) {
    if (!dragging) return;
    const box = handle.parentElement!.getBoundingClientRect();
    onchange(Math.max(min, Math.min(max, ((e.clientX - box.left) / box.width) * 100)));
  }
</script>

<!-- A focusable separator implements the WAI-ARIA window splitter keyboard pattern. -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<div bind:this={handle} class="divider" class:dragging role="separator" tabindex="0"
  aria-label="Ширина файловых панелей" aria-orientation="vertical"
  aria-valuemin={Math.round(min)} aria-valuemax={Math.round(max)} aria-valuenow={Math.round(value)}
  title="Потяните для изменения ширины. Двойной щелчок — поровну."
  onpointerdown={(e) => { if (e.button === 0) { dragging = true; handle.setPointerCapture(e.pointerId); e.preventDefault(); } }}
  onpointermove={move} onpointerup={() => (dragging = false)} onpointercancel={() => (dragging = false)}
  ondblclick={() => onchange(50)}
  onkeydown={(e) => {
    if (e.key === "ArrowLeft") onchange(Math.max(min, value - 2));
    else if (e.key === "ArrowRight") onchange(Math.min(max, value + 2));
    else if (e.key === "Home") onchange(50);
    else return;
    e.preventDefault();
  }}
></div>

<style>
  .divider {
    width: 5px;
    cursor: col-resize;
    background: var(--border);
    touch-action: none;
    z-index: 1;
  }
  .divider:hover, .divider.dragging, .divider:focus-visible { background: var(--accent); }
</style>
