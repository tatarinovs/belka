import { stored } from "./storage";

export type SavedLocation = { id: string; name: string } & (
  | { kind: "local"; local: string }
  | { kind: "cloud"; account: string; remote: string }
  | { kind: "pair"; local: string; account: string; remote: string }
);

export const LOCATIONS_KEY = "desktop.locations";

export function loadLocations(): SavedLocation[] {
  try {
    const value: unknown = JSON.parse(stored(LOCATIONS_KEY) ?? "[]");
    if (!Array.isArray(value)) return [];
    const ids = new Set<string>();
    return value.filter((v): v is SavedLocation => {
      if (!v || typeof v !== "object" || typeof v.id !== "string" || !v.id || ids.has(v.id) ||
          typeof v.name !== "string" || !v.name.trim()) return false;
      const local = typeof v.local === "string";
      const cloud = typeof v.account === "string" && !!v.account && typeof v.remote === "string" && v.remote.startsWith("/");
      const valid = v.kind === "local" ? local : v.kind === "cloud" ? cloud : v.kind === "pair" && local && !!v.local && cloud;
      if (valid) ids.add(v.id);
      return valid;
    });
  } catch { return []; }
}

/** Windows local paths are case insensitive; cloud paths keep their case. */
export function locationKey(location: SavedLocation): string {
  let local = "";
  if (location.kind !== "cloud" && location.local) {
    local = location.local.replace(/\\/g, "/").replace(/\/+$/, "").toLowerCase() || "/";
  }
  return JSON.stringify([
    location.kind, local,
    location.kind === "local" ? "" : location.account,
    location.kind === "local" ? "" : location.remote,
  ]);
}
