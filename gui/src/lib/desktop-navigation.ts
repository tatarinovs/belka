import { api, type Account, type Entry, type Listing } from "./api";
import { cloudListing } from "./cloud";
import { baseName } from "./format";
import type { Tab } from "./components/TabBar.svelte";
import { providerDef } from "./providers";

export type Side = "left" | "right";
export type BrowserTab = { id: string; path: string };
export type PaneTabs = { tabs: BrowserTab[]; active: string };
export type Location = { kind: "root" } | { kind: "local"; path: string } |
  { kind: "cloud"; account: string; path: string };
export const ROOT_LABEL = "Облака и диски";

/** Cloud addresses are UI navigation keys. Native IPC always receives raw paths. */
export function cloudAddress(account: string, path = "/") {
  return `cloud://${encodeURIComponent(account)}${path.startsWith("/") ? path : "/" + path}`;
}

export function location(address: string): Location {
  if (!address) return { kind: "root" };
  if (!address.startsWith("cloud://")) return { kind: "local", path: address };
  const match = /^cloud:\/\/([^/]+)(\/.*)$/.exec(address);
  if (!match) throw new Error("Некорректный путь к облаку");
  return { kind: "cloud", account: decodeURIComponent(match[1]), path: match[2] };
}

export function nativeEntry(entry: Entry): Entry {
  const source = location(entry.path);
  return { ...entry, path: source.kind === "cloud" ? source.path : entry.path };
}

export async function desktopListing(accounts: Account[], address: string): Promise<Listing> {
  const source = location(address);
  if (source.kind === "root") {
    const local = await api.listLocal("");
    return { path: "", parent: null, crumbs: [], entries: [
      ...accounts.map((account): Entry => ({ name: account.name, path: cloudAddress(account.id),
        is_dir: true, size: 0, modified: null, icon: "cloud" })),
      ...local.entries,
    ] };
  }
  if (source.kind === "local") return api.listLocal(source.path);
  const account = accounts.find((a) => a.id === source.account);
  if (!account) throw new Error("Аккаунт этого облака удалён");
  const listing = await cloudListing(accounts, account, source.path);
  return {
    path: cloudAddress(account.id, listing.path),
    parent: listing.parent === null ? "" : cloudAddress(account.id, listing.parent),
    crumbs: [{ name: account.name, path: cloudAddress(account.id) },
      ...listing.crumbs.map((c) => ({ ...c, path: cloudAddress(account.id, c.path) }))],
    entries: listing.entries.map((e) => ({ ...e, path: cloudAddress(account.id, e.path) })),
  };
}

export function tabItem(tab: BrowserTab, accounts: Account[]): Tab {
  const source = location(tab.path);
  if (source.kind === "root") return { id: tab.id, label: ROOT_LABEL, title: ROOT_LABEL };
  if (source.kind === "local") return { id: tab.id, label: baseName(source.path), title: source.path };
  const account = accounts.find((a) => a.id === source.account);
  return { id: tab.id, label: source.path === "/" ? account?.name ?? "Облако" : baseName(source.path),
    title: `${account?.name ?? "Облако"}: ${source.path}`, kind: account?.kind,
    badge: account ? providerDef(account.kind).short : undefined };
}

/** Restore saved panes, dropping invalid tabs and removed accounts. */
export function restorePanes(raw: string | null, accounts: Account[]): Record<Side, PaneTabs> {
  let saved: Record<string, unknown> = {};
  try { saved = JSON.parse(raw ?? "{}") ?? {}; } catch { /* Start at the root. */ }
  const known = new Set(accounts.map((a) => a.id));
  function pane(side: Side): PaneTabs {
    const data = saved[side] as { tabs?: { id?: unknown; path?: unknown }[]; active?: unknown } | undefined;
    const ids = new Set<string>();
    const tabs: BrowserTab[] = [];
    for (const value of Array.isArray(data?.tabs) ? data.tabs : []) {
      if (!value || typeof value.id !== "string" || typeof value.path !== "string" || ids.has(value.id)) continue;
      const path = value.path;
      try {
        const source = location(path);
        if (source.kind === "cloud" && !known.has(source.account)) continue;
      } catch { continue; }
      tabs.push({ id: value.id, path });
      ids.add(value.id);
    }
    if (!tabs.length) tabs.push({ id: crypto.randomUUID(), path: "" });
    return { tabs, active: tabs.find((t) => t.id === data?.active)?.id ?? tabs[0].id };
  }
  return { left: pane("left"), right: pane("right") };
}
