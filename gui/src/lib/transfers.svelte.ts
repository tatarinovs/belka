import { api, type Transfer, type TransferCounts } from "./api";

const POLL_MS = 400;
/** A running transfer without a single byte this long is waiting for the server. */
const WAITING_MS = 5000;

type FinishListener = (transfer: Transfer) => void;

const NO_TRANSFERS: TransferCounts = { queued: 0, running: 0, done: 0, failed: 0, cancelled: 0 };

/**
 * Polls the backend queue while anything is active. Speeds are derived here
 * from the byte counters, smoothed so the numbers do not jump around.
 * `items` is a part of a long queue; `counts` cover all of it.
 */
class TransferStore {
  items = $state<Transfer[]>([]);
  counts = $state<TransferCounts>(NO_TRANSFERS);
  speeds = $state<Record<number, number>>({});
  /** Running transfers that have received nothing for a while. */
  waiting = $state<Record<number, boolean>>({});
  error = $state("");

  activeCount = $derived(this.counts.queued + this.counts.running);
  failedCount = $derived(this.counts.failed);
  total = $derived(
    this.counts.queued + this.counts.running + this.counts.done + this.counts.failed + this.counts.cancelled,
  );

  #timer: ReturnType<typeof setTimeout> | null = null;
  #last = new Map<number, { done: number; at: number }>();
  #states = new Map<number, Transfer["state"]>();
  #runningSince = new Map<number, number>();
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
      const { items, counts } = await api.listTransfers();
      this.#track(items);
      this.items = items;
      this.counts = counts;
      this.error = "";
    } catch (err) {
      this.error = String(err);
    }
  }

  #tick = async () => {
    this.#timer = null;
    await this.refresh();
    if (this.activeCount) this.#timer = setTimeout(this.#tick, POLL_MS);
  };

  #track(items: Transfer[]) {
    const now = performance.now();
    const speeds: Record<number, number> = {};
    const waiting: Record<number, boolean> = {};
    // Transfers no longer sent are forgotten.
    const shown = new Set(items.map((t) => t.id));
    for (const id of this.#last.keys()) {
      if (!shown.has(id)) {
        this.#last.delete(id);
        this.#states.delete(id);
        this.#runningSince.delete(id);
      }
    }
    for (const t of items) {
      const last = this.#last.get(t.id);
      if (t.state === "running" && last && now > last.at) {
        const instant = (Math.max(0, t.done - last.done) * 1000) / (now - last.at);
        const previous = this.speeds[t.id];
        speeds[t.id] = previous === undefined ? instant : previous * 0.7 + instant * 0.3;
      }
      this.#last.set(t.id, { done: t.done, at: now });

      if (t.state === "running") {
        const since = this.#runningSince.get(t.id) ?? now;
        this.#runningSince.set(t.id, since);
        if (t.done === 0 && now - since > WAITING_MS) waiting[t.id] = true;
      } else {
        this.#runningSince.delete(t.id);
      }

      const before = this.#states.get(t.id);
      if (t.state === "done" && before !== "done") {
        for (const listener of this.#listeners) listener(t);
      }
      this.#states.set(t.id, t.state);
    }
    this.speeds = speeds;
    this.waiting = waiting;
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
