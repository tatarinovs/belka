<script lang="ts" module>
  export interface Tab {
    id: string;
    label: string;
    /** Full location, shown on hover. */
    title: string;
    /** Provider kind for a small badge (cloud tabs). */
    kind?: string;
    badge?: string;
  }
</script>

<script lang="ts">
  import Icon from "./Icon.svelte";

  let { tabs, activeId, onselect, onclose, onnew, onreorder }: {
    tabs: Tab[];
    activeId: string | null;
    onselect: (id: string) => void;
    onclose: (id: string) => void;
    /** Opens a copy of the active tab. */
    onnew: () => void;
    onreorder: (ids: string[]) => void;
  } = $props();

  let strip = $state<HTMLDivElement>();
  /** The tab being dragged and the order shown meanwhile. */
  let drag = $state<{ id: string; ids: string[] } | null>(null);
  let pressed: { id: string; x: number } | null = null;

  const shown = $derived(
    drag ? drag.ids.map((id) => tabs.find((t) => t.id === id)).filter((t) => t !== undefined) : tabs,
  );

  function pointerdown(event: PointerEvent, tab: Tab) {
    if (event.button === 1) {
      // Middle click closes, as in browsers and file managers.
      event.preventDefault();
      if (tabs.length > 1) onclose(tab.id);
      return;
    }
    if (event.button !== 0) return;
    onselect(tab.id);
    pressed = { id: tab.id, x: event.clientX };
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
  }

  function pointermove(event: PointerEvent) {
    if (!pressed || !strip) return;
    if (!drag) {
      if (Math.abs(event.clientX - pressed.x) < 6 || tabs.length < 2) return;
      drag = { id: pressed.id, ids: tabs.map((t) => t.id) };
    }
    const others = [...strip.querySelectorAll<HTMLElement>("[data-tab]")].filter(
      (el) => el.dataset.tab !== drag?.id,
    );
    const index = others.filter((el) => {
      const box = el.getBoundingClientRect();
      return box.left + box.width / 2 < event.clientX;
    }).length;
    const ids = others.map((el) => el.dataset.tab!);
    ids.splice(index, 0, drag.id);
    if (ids.join() !== drag.ids.join()) drag.ids = ids;
  }

  function pointerup() {
    pressed = null;
    if (!drag) return;
    const ids = drag.ids;
    drag = null;
    if (ids.join() !== tabs.map((t) => t.id).join()) onreorder(ids);
  }
</script>

<div class="tabs" class:dragging={drag !== null} role="tablist">
  <div class="strip" bind:this={strip} role="presentation" ondblclick={(e) => e.target === e.currentTarget && onnew()}>
    {#each shown as tab (tab.id)}
      <div
        class="tab"
        class:active={tab.id === activeId}
        class:dragged={drag?.id === tab.id}
        role="tab"
        tabindex="-1"
        aria-selected={tab.id === activeId}
        title={tab.title}
        data-tab={tab.id}
        onpointerdown={(e) => pointerdown(e, tab)}
        onpointermove={pointermove}
        onpointerup={pointerup}
        onpointercancel={pointerup}
      >
        {#if tab.badge}
          <span class="badge" data-kind={tab.kind}>{tab.badge}</span>
        {/if}
        <span class="label">{tab.label}</span>
        {#if tabs.length > 1}
          <button
            class="close"
            title="Закрыть вкладку (Ctrl+W)"
            tabindex="-1"
            onpointerdown={(e) => e.stopPropagation()}
            onclick={() => onclose(tab.id)}
          >
            <Icon name="close" size={12} />
          </button>
        {/if}
      </div>
    {/each}
  </div>
  <button class="icon-btn new" title="Новая вкладка (Ctrl+T)" onclick={onnew}>
    <Icon name="plus" size={14} />
  </button>
</div>

<style>
  .tabs {
    display: flex;
    align-items: flex-end;
    gap: 2px;
    min-height: 31px;
    padding: 4px 6px 0;
    background: var(--sidebar);
    border-bottom: 1px solid var(--border);
    user-select: none;
  }

  .strip {
    flex: 1;
    display: flex;
    gap: 2px;
    min-width: 0;
    align-self: stretch;
    align-items: flex-end;
    overflow-x: auto;
    scrollbar-width: none;
  }

  .tab {
    flex: 0 1 180px;
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 70px;
    height: 27px;
    margin-bottom: -1px;
    padding: 0 4px 0 10px;
    border: 1px solid transparent;
    border-bottom: none;
    border-radius: 6px 6px 0 0;
    color: var(--muted);
    cursor: default;
  }

  .tab:hover {
    background: var(--hover);
  }

  .tab.active {
    background: var(--surface);
    border-color: var(--border);
    color: var(--text);
  }

  .dragging,
  .dragging .tab {
    cursor: grabbing;
  }

  .tab.dragged {
    opacity: 0.7;
  }

  .label {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .close {
    flex: none;
    display: grid;
    place-items: center;
    width: 18px;
    height: 18px;
    padding: 0;
    border: none;
    border-radius: 4px;
    background: none;
    color: var(--muted);
    visibility: hidden;
  }

  .tab:hover .close,
  .tab.active .close {
    visibility: visible;
  }

  .close:hover {
    background: var(--hover);
    color: var(--text);
  }

  .new {
    flex: none;
    align-self: center;
    margin-bottom: 2px;
  }

  .badge {
    width: 18px;
    height: 16px;
    border-radius: 4px;
    font-size: 9px;
  }

</style>
