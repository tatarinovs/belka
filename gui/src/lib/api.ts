import { invoke } from "@tauri-apps/api/core";

export type ProviderKind = "yandex" | "mailru" | "google" | "webdav" | "s3" | "synology" | "crypt";

export interface Account {
  id: string;
  name: string;
  kind: ProviderKind;
  /** Where an encrypted folder lives. */
  crypt?: { base: string; root: string };
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
  /** Stored encrypted: the row gets a lock, the text is its tooltip. */
  locked?: string;
  /** Opening the row switches to this encrypted folder account. */
  opensAccount?: string;
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

/** Settings of a shared link; empty values mean "not set". */
export interface ShareOptions {
  password: string;
  /** `YYYY-MM-DD`, the last day the link works. */
  expires: string;
}

/** Mirrors `ProviderConfig` from the core crate: `kind` plus provider fields. */
export type ProviderConfig = { kind: ProviderKind } & Record<string, string | number | boolean>;

export type TransferState = "queued" | "running" | "done" | "failed" | "cancelled";

/** How many transfers are in each state, including ones not in the list. */
export type TransferCounts = Record<TransferState, number>;

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
  /** Above 1 while network failures are retried. */
  attempt: number;
  max_attempts: number;
}

/** Mirrors `CryptFolder` from the GUI backend. */
export interface CryptFolder {
  /** Id of the account that stores the encrypted files. */
  base: string;
  /** Folder of that account that holds the encrypted tree. */
  root: string;
  password: string;
  /** rclone's password2; empty for rclone's built-in salt. */
  salt: string;
  encrypt_names: boolean;
}

/** A cloud file downloaded for preview. */
export interface Preview {
  /** The cached file. */
  path: string;
  /** Content of a text file. */
  text: string | null;
  /** The text was cut at 1 MB. */
  truncated: boolean;
}

export type OAuthService = "google" | "yandex";

/** A registered OAuth client used for browser sign-in. */
export interface OAuthApp {
  client_id: string;
  client_secret: string;
}

export interface OAuthApps {
  /** This build has its own OAuth app for the service. */
  builtin: boolean;
  /** The user's own OAuth app, saved after the last sign-in with it. */
  custom: OAuthApp | null;
}

/** Mirrors `tasks::RunResult` of the store crate. */
export interface BackupRun {
  started: string | null;
  finished: string | null;
  uploaded: number;
  bytes: number;
  /** Files that were in the cloud already. */
  unchanged: number;
  failed: number;
  /** The first failed files: [file, reason]. */
  failures: [string, string][];
  /** Local files deleted to free space, and their size. */
  freed: number;
  freed_bytes: number;
  /** Why the run failed as a whole. */
  error: string | null;
  cancelled: boolean;
}

/** Mirrors `tasks::LiveView`: a run in progress. */
export interface BackupProgress {
  scanning: boolean;
  files_total: number;
  files_done: number;
  bytes_total: number;
  bytes_done: number;
  current: string;
}

/** Mirrors `tasks::Task`: a local folder copied into a cloud folder on a schedule. */
export interface BackupTask {
  /** Empty for a task that is not saved yet. */
  id: string;
  name: string;
  local: string;
  account_id: string;
  remote: string;
  interval_hours: number;
  /** Android: wait for Wi‑Fi. */
  wifi_only: boolean;
  /** Android: wait for the charger. */
  charging_only: boolean;
  enabled: boolean;
  /** Patterns of files to take; empty takes all. */
  include: string[];
  /** Patterns of files and folders to leave out. */
  exclude: string[];
  /** Delete local files older than this many days once they are in the cloud. */
  free_after_days: number | null;
  /** The latest runs, newest first. */
  history: BackupRun[];
  running?: BackupProgress | null;
}

/** A task to fill in: by default daily, over Wi‑Fi. */
export function newBackupTask(local: string, accountId: string, remote = ""): BackupTask {
  return {
    id: "",
    name: "",
    local,
    account_id: accountId,
    remote,
    interval_hours: 24,
    wifi_only: true,
    charging_only: false,
    enabled: true,
    include: [],
    exclude: [],
    free_after_days: null,
    history: [],
  };
}

export const api = {
  listAccounts: () => invoke<Account[]>("list_accounts"),
  addAccount: (name: string, config: ProviderConfig) =>
    invoke<Account>("add_account", { name, config }),
  oauthApps: (service: OAuthService) => invoke<OAuthApps>("oauth_apps", { service }),
  /** Opens the browser and resolves once the account is saved. Without
   * `customApp` the built-in OAuth app is used. */
  /** With `accountId` the credentials of that account are replaced. */
  oauthSignIn: (
    service: OAuthService,
    customApp: OAuthApp | null,
    name: string,
    accountId: string | null = null,
  ) => invoke<Account>("oauth_sign_in", { service, customApp, name, accountId }),
  /** Stored credentials for the settings form; null for an encrypted folder. */
  accountConfig: (id: string) => invoke<ProviderConfig | null>("account_config", { id }),
  /** Renames; with `config` also replaces the credentials once they work. */
  updateAccount: (id: string, name: string, config: ProviderConfig | null) =>
    invoke<Account>("update_account", { id, name, config }),
  oauthCancel: () => invoke<void>("oauth_cancel"),
  /** Checks the password against files already in the folder, then saves. */
  addCryptFolder: (name: string, folder: CryptFolder) =>
    invoke<Account>("add_crypt_folder", { name, folder }),
  removeAccount: (id: string) => invoke<void>("remove_account", { id }),
  reorderAccounts: (ids: string[]) => invoke<void>("reorder_accounts", { ids }),
  /** Saves all accounts with credentials into a file encrypted with `password`.
   * Resolves to the number of accounts. */
  exportAccounts: (path: string, password: string) =>
    invoke<number>("export_accounts", { path, password }),
  /** Adds the accounts from an exported file; ones already here are skipped. */
  importAccounts: (path: string, password: string) =>
    invoke<{ added: number; skipped: number }>("import_accounts", { path, password }),
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
  /** Opens access by link; the backend also copies the links to the clipboard.
   * With `options` the links get exactly these settings, replacing existing ones. */
  cloudShare: (accountId: string, items: Entry[], options?: ShareOptions) =>
    invoke<string[]>("cloud_share", {
      accountId,
      items: items.map(({ path, is_dir }) => ({ path, is_dir })),
      options: options ?? null,
    }),
  cloudUnshare: (accountId: string, items: Entry[]) =>
    invoke<void>("cloud_unshare", {
      accountId,
      items: items.map(({ path, is_dir }) => ({ path, is_dir })),
    }),
  /** Downloads a file into the preview cache once; cancels the previous preview. */
  previewFile: (accountId: string, entry: Entry, text: boolean) =>
    invoke<Preview>("preview_file", {
      accountId,
      path: entry.path,
      size: entry.size,
      modified: entry.modified,
      text,
    }),
  /** Bytes downloaded and total of the preview in progress. */
  previewProgress: () =>
    invoke<{ done: number; total: number; attempt: number; max_attempts: number }>("preview_progress"),
  previewCancel: () => invoke<void>("preview_cancel"),
  /** Opens a cached preview in the program associated with it. */
  previewOpen: (path: string) => invoke<void>("preview_open", { path }),
  /** Opens a local file in the program associated with it. */
  localOpen: (path: string) => invoke<void>("local_open", { path }),
  localMkdir: (parent: string, name: string) => invoke<string>("local_mkdir", { parent, name }),
  /** Resolves to the new path. */
  localRename: (path: string, newName: string) =>
    invoke<string>("local_rename", { path, newName }),
  /** Moves to the Recycle Bin. */
  localDelete: (paths: string[]) => invoke<void>("local_delete", { paths }),

  /** The running transfers, the head of the queue and the latest finished ones. */
  listTransfers: () => invoke<{ items: Transfer[]; counts: TransferCounts }>("list_transfers"),
  cancelTransfer: (id: number) => invoke<void>("cancel_transfer", { id }),
  cancelAllTransfers: () => invoke<void>("cancel_all_transfers"),
  retryTransfer: (id: number) => invoke<void>("retry_transfer", { id }),
  clearTransfers: () => invoke<void>("clear_transfers"),

  /** Android: the app may browse the shared storage. Always true on desktops. */
  storageAccess: () => invoke<boolean>("storage_access"),
  /** Android: opens the system page that grants access to all files. */
  requestStorageAccess: () => invoke<void>("request_storage_access"),
  /** The latest release on GitHub if it is newer than this build. */
  checkUpdate: () => invoke<Update | null>("check_update"),

  /** Backup tasks with the progress of running ones. */
  backupTasks: () => invoke<BackupTask[]>("backup_tasks"),
  /** Adds a task (an empty id) or changes one; resolves to the saved task. */
  backupSave: (task: BackupTask) => invoke<BackupTask>("backup_save", { task }),
  backupRemove: (id: string) => invoke<void>("backup_remove", { id }),
  /** Starts a task now; on Android it runs in the background under a notification. */
  backupRun: (id: string) => invoke<void>("backup_run", { id }),
  backupCancel: (id: string) => invoke<void>("backup_cancel", { id }),
};

export interface Update {
  version: string;
  /** The release page. */
  page: string;
  /** The Android package, if the release has one. */
  apk: string | null;
}

/** Commands reject with a plain string; anything else is unexpected. */
export function errorText(err: unknown): string {
  return typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
}
