import { api, type Transfer } from "./api";

const POLL_MS = 400;

type FinishListener = (transfer: Transfer) => void;

function isActive(t: Transfer) {
  return t.state === "queued" || t.state === "running";
}

/**
 * Polls the backend queue while anything is active. Speeds are derived here
 * from the byte counters, smoothed so the numbers do not jump around.
 */
class TransferStore {
  items = $state<Transfer[]>([]);
  speeds = $state<Record<number, number>>({});
  error = $state("");

  activeCount = $derived(this.items.filter(isActive).length);
  failedCount = $derived(this.items.filter((t) => t.state === "failed").length);

  #timer: ReturnType<typeof setTimeout> | null = null;
  #last = new Map<number, { done: number; at: number }>();
  #states = new Map<number, Transfer["state"]>();
  #listeners = new Set<FinishListener>();

  /** Starts polling; call after anything was queued or retried. */
  kick() {
    if (this.#timer === null) this.#tick();
  }

  /** Called whenever a transfer completes, to refresh the folder it went to. */
  onFinished(listener: FinishListener) {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  }

  async refresh() {
    try {
      const items = await api.listTransfers();
      this.#track(items);
      this.items = items;
      this.error = "";
    } catch (err) {
      this.error = String(err);
    }
  }

  #tick = async () => {
    this.#timer = null;
    await this.refresh();
    if (this.items.some(isActive)) this.#timer = setTimeout(this.#tick, POLL_MS);
  };

  #track(items: Transfer[]) {
    const now = performance.now();
    const speeds: Record<number, number> = {};
    for (const t of items) {
      const last = this.#last.get(t.id);
      if (t.state === "running" && last && now > last.at) {
        const instant = (Math.max(0, t.done - last.done) * 1000) / (now - last.at);
        const previous = this.speeds[t.id];
        speeds[t.id] = previous === undefined ? instant : previous * 0.7 + instant * 0.3;
      }
      this.#last.set(t.id, { done: t.done, at: now });

      const before = this.#states.get(t.id);
      if (t.state === "done" && before !== "done") {
        for (const listener of this.#listeners) listener(t);
      }
      this.#states.set(t.id, t.state);
    }
    this.speeds = speeds;
  }

  async cancel(id: number) {
    await api.cancelTransfer(id);
    await this.refresh();
  }

  async cancelAll() {
    await api.cancelAllTransfers();
    await this.refresh();
  }

  async retry(id: number) {
    await api.retryTransfer(id);
    this.kick();
  }

  async clear() {
    await api.clearTransfers();
    await this.refresh();
  }
}

export const transfers = new TransferStore();
