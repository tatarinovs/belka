import { invoke } from "@tauri-apps/api/core";

let values: Record<string, string> = {};
let writes = Promise.resolve();

/** Load before mounting components, which read preferences synchronously. */
export async function initializeStorage() {
  values = await invoke<Record<string, string>>("load_ui_state");
}

export function stored(key: string): string | null {
  return values[key] ?? null;
}

export function remember(key: string, value: string) {
  if (values[key] === value) return;
  values[key] = value;
  // Keep updates ordered; a failed write must not prevent subsequent saves.
  writes = writes.then(() => invoke<void>("remember_ui_state", { key, value }))
    .catch((error) => console.error("Cannot save UI settings:", error));
}
