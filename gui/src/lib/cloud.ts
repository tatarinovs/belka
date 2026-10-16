import { api, type Account, type Entry, type Listing } from "./api";
import { parentPath, splitPath } from "./format";

/** `«name»` for one item, or how many there are. */
export function describe(items: Entry[]): string {
  return items.length === 1 ? `«${items[0].name}»` : `выбранные объекты (${items.length})`;
}

/** A cloud folder in the shape both panes show, with encrypted data marked. */
export async function cloudListing(accounts: Account[], account: Account, path: string): Promise<Listing> {
  const entries = markEncrypted(accounts, account, await api.listDir(account.id, path));
  return {
    path,
    parent: path === "/" ? null : parentPath(path),
    crumbs: splitPath(path),
    entries,
  };
}

/** Marks what is stored encrypted: everything in an encrypted folder, and
 * in a regular cloud the encrypted folders and what is inside them. */
export function markEncrypted(accounts: Account[], account: Account, entries: Entry[]): Entry[] {
  if (account.kind === "crypt") {
    return entries.map((e) => ({ ...e, locked: "Хранится в облаке зашифрованным" }));
  }
  const folders = accounts
    .filter((a) => a.crypt?.base === account.id)
    .map((a) => ({ account: a, root: "/" + a.crypt!.root.replace(/^\/+|\/+$/g, "") }));
  if (!folders.length) return entries;
  return entries.map((e) => {
    for (const { account: folder, root } of folders) {
      if (e.path === root) {
        return {
          ...e,
          locked: `Зашифрованная папка «${folder.name}»: откройте, чтобы увидеть файлы`,
          opensAccount: folder.id,
        };
      }
      if (root === "/" || e.path.startsWith(root + "/")) {
        return { ...e, locked: `Зашифрованные данные папки «${folder.name}»` };
      }
    }
    return e;
  });
}
