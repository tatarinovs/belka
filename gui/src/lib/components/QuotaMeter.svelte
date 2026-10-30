<script lang="ts">
  import { api, type Account, type Quota } from "$lib/api";
  import { formatBytes } from "$lib/format";

  interface Props {
    account: Account;
    /** The open folder: a NAS reports the space of each shared folder's volume. */
    path: string;
    /** Changing it reads the space again, say after uploads. */
    version?: number;
  }

  let { account, path, version = 0 }: Props = $props();

  let quota = $state<Quota | null>(null);
  /** What the space depends on: the account, and on a NAS the shared folder. */
  const scope = $derived(
    account.kind === "synology" ? `/${path.split("/").find(Boolean) ?? ""}` : "/",
  );
  let shownKey = "";
  let requestSeq = 0;

  $effect(() => {
    const key = `${account.id}:${scope}`;
    void version;
    // Another cloud's numbers must not stay while this one's load.
    if (key !== shownKey) quota = null;
    shownKey = key;
    const seq = ++requestSeq;
    api
      .cloudQuota(account.id, scope)
      .then((q) => {
        if (seq === requestSeq) quota = q;
      })
      .catch(() => {
        if (seq === requestSeq) quota = null;
      });
  });

  const ratio = $derived(
    quota?.total && quota.used !== null ? Math.min(1, quota.used / quota.total) : null,
  );
  /** Less than a tenth left. */
  const low = $derived(ratio !== null && ratio > 0.9);

  const text = $derived.by(() => {
    if (!quota) return "";
    const { total, used, free } = quota;
    if (free !== null && total !== null) return `Свободно ${formatBytes(free)} из ${formatBytes(total)}`;
    if (free !== null) return `Свободно ${formatBytes(free)}`;
    if (used !== null) return `Занято ${formatBytes(used)}`;
    return "";
  });

  const title = $derived(
    quota?.used != null && quota.total
      ? `Занято ${formatBytes(quota.used)} из ${formatBytes(quota.total)} (${Math.round((ratio ?? 0) * 100)}%)`
      : text,
  );
</script>

{#if text}
  <span class="quota" class:low {title}>
    {#if ratio !== null}
      <span class="meter" aria-hidden="true"><span style:width="{ratio * 100}%"></span></span>
    {/if}
    <span class="text">{text}</span>
  </span>
{/if}

<style>
  .quota {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    color: var(--muted);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .text {
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .meter {
    flex: none;
    width: 64px;
    height: 6px;
    overflow: hidden;
    border-radius: 3px;
    background: var(--border);
  }

  .meter span {
    display: block;
    height: 100%;
    border-radius: 3px;
    background: var(--accent);
  }

  .low {
    color: var(--danger);
  }

  .low .meter span {
    background: var(--danger);
  }
</style>
