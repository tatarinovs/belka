import { api, type Account } from "./api";

/**
 * The accounts shown in the page, shared by both layouts. Changes are made
 * through the backend first; reordering shows the new order at once and goes
 * back if saving fails.
 */
class AccountStore {
  list = $state<Account[]>([]);
  /** The first load finished, successfully or not. */
  loaded = $state(false);

  find(id: string | null | undefined): Account | null {
    return this.list.find((a) => a.id === id) ?? null;
  }

  /** Loads the list; a failure is passed on, `loaded` is set either way. */
  async load(): Promise<Account[]> {
    try {
      this.list = await api.listAccounts();
      return this.list;
    } finally {
      this.loaded = true;
    }
  }

  added(account: Account) {
    this.list = [...this.list, account];
  }

  saved(account: Account) {
    this.list = this.list.map((a) => (a.id === account.id ? account : a));
  }

  async remove(account: Account) {
    await api.removeAccount(account.id);
    this.list = this.list.filter((a) => a.id !== account.id);
  }

  async reorder(ids: string[]) {
    const before = this.list;
    this.list = ids.map((id) => before.find((a) => a.id === id)).filter((a) => a !== undefined);
    try {
      await api.reorderAccounts(ids);
    } catch (err) {
      this.list = before;
      throw err;
    }
  }
}

export const accounts = new AccountStore();
