import { invoke } from "@tauri-apps/api/core";

export type ProviderKind = "yandex" | "mailru" | "google" | "webdav" | "s3";

export interface Account {
  id: string;
  name: string;
  kind: ProviderKind;
}

/** A row of either pane: `RemoteFileInfo` from the core crate or `LocalEntry`. */
export interface Entry {
  name: string;
  path: string;
  is_dir: boolean;
  size: number;
  modified: string | null;
  /** Local virtual root only. */
  icon?: "drive" | "place";
}

export type RemoteEntry = Entry;

export interface Crumb {
  name: string;
  path: string;
}

/** What a pane shows; the cloud listing is adapted to this shape in the page. */
export interface Listing {
  path: string;
  /** `null` at the root. */
  parent: string | null;
  crumbs: Crumb[];
  entries: Entry[];
}

/** Mirrors `ProviderConfig` from the core crate: `kind` plus provider fields. */
export type ProviderConfig = { kind: ProviderKind } & Record<string, string | boolean>;

export type TransferState = "queued" | "running" | "done" | "failed" | "cancelled";

/** Mirrors `TransferView` from the GUI backend. */
export interface Transfer {
  id: number;
  direction: "upload" | "download";
  account_id: string;
  account_name: string;
  name: string;
  local_path: string;
  remote_path: string;
  remote_dir: string;
  state: TransferState;
  error?: string;
  done: number;
  total: number;
}

export const api = {
  listAccounts: () => invoke<Account[]>("list_accounts"),
  addAccount: (name: string, config: ProviderConfig) =>
    invoke<Account>("add_account", { name, config }),
  removeAccount: (id: string) => invoke<void>("remove_account", { id }),
  listDir: (accountId: string, path: string) =>
    invoke<RemoteEntry[]>("list_dir", { accountId, path }),

  /** Resolves to the number of queued files. */
  download: (accountId: string, items: Entry[], localDir: string) =>
    invoke<number>("download", {
      accountId,
      items: items.map(({ path, is_dir, size }) => ({ path, is_dir, size })),
      localDir,
    }),
  /** Resolves to the number of queued files. */
  upload: (accountId: string, localPaths: string[], remoteDir: string) =>
    invoke<number>("upload", { accountId, localPaths, remoteDir }),
  /** The empty path is the virtual "This PC" root. */
  listLocal: (path: string) => invoke<Listing>("list_local", { path }),
  localConflicts: (dir: string, names: string[]) =>
    invoke<string[]>("local_conflicts", { dir, names }),

  cloudMkdir: (accountId: string, parent: string, name: string) =>
    invoke<string>("cloud_mkdir", { accountId, parent, name }),
  /** Resolves to the new path. */
  cloudRename: (accountId: string, path: string, newName: string, isDir: boolean) =>
    invoke<string>("cloud_rename", { accountId, path, newName, isDir }),
  cloudDelete: (accountId: string, items: Entry[]) =>
    invoke<void>("cloud_delete", {
      accountId,
      items: items.map(({ path, is_dir }) => ({ path, is_dir })),
    }),
  localMkdir: (parent: string, name: string) => invoke<string>("local_mkdir", { parent, name }),
  /** Resolves to the new path. */
  localRename: (path: string, newName: string) =>
    invoke<string>("local_rename", { path, newName }),
  /** Moves to the Recycle Bin. */
  localDelete: (paths: string[]) => invoke<void>("local_delete", { paths }),

  listTransfers: () => invoke<Transfer[]>("list_transfers"),
  cancelTransfer: (id: number) => invoke<void>("cancel_transfer", { id }),
  cancelAllTransfers: () => invoke<void>("cancel_all_transfers"),
  retryTransfer: (id: number) => invoke<void>("retry_transfer", { id }),
  clearTransfers: () => invoke<void>("clear_transfers"),
};

/** Commands reject with a plain string; anything else is unexpected. */
export function errorText(err: unknown): string {
  return typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
}
