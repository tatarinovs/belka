/**
 * Dragging selected rows from one pane to the other. Native HTML5 drag and
 * drop is unavailable inside the webview on Windows while Tauri handles file
 * drops from Explorer, so panes track the pointer themselves.
 */
export const dnd = $state({
  /** Id of the pane the drag started in; null when nothing is dragged. */
  from: null as string | null,
  /** Id of the pane under the pointer. */
  over: null as string | null,
  count: 0,
  x: 0,
  y: 0,
});

export function paneAt(x: number, y: number): string | null {
  const el = document.elementFromPoint(x, y)?.closest<HTMLElement>("[data-pane]");
  return el?.dataset.pane ?? null;
}
