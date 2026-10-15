<script lang="ts">
  import { errorText, type Entry, type ShareOptions } from "$lib/api";

  let { entries, onclose, onshare }: {
    /** Items to share; the dialog is open while set. */
    entries: Entry[] | null;
    onclose: () => void;
    /** Creates the links; a rejection is shown in the dialog. */
    onshare: (options: ShareOptions) => Promise<void>;
  } = $props();

  let dialog: HTMLDialogElement;
  let password = $state("");
  let expires = $state("");
  let busy = $state(false);
  let error = $state("");

  /** Today in local time as `YYYY-MM-DD`, the earliest expiry date. */
  function today() {
    const d = new Date();
    return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
  }

  $effect(() => {
    if (entries && !dialog.open) {
      password = "";
      expires = "";
      error = "";
      dialog.showModal();
    } else if (!entries && dialog.open) {
      dialog.close();
    }
  });

  const subject = $derived(
    entries?.length === 1 ? `«${entries[0].name}»` : `выбранных объектов (${entries?.length ?? 0})`,
  );

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    if (busy) return;
    busy = true;
    error = "";
    try {
      await onshare({ password, expires });
    } catch (err) {
      error = errorText(err);
    } finally {
      busy = false;
    }
  }
</script>

<dialog class="modal"
  bind:this={dialog}
  onclose={onclose}
  oncancel={(e) => {
    if (busy) e.preventDefault();
  }}
>
  <form onsubmit={submit}>
    <h2>Ссылка для {subject}</h2>

    <label>
      <span>Пароль</span>
      <input
        bind:value={password}
        placeholder="Без пароля"
        autocomplete="off"
        spellcheck="false"
        disabled={busy}
      />
    </label>
    <label>
      <span>Действует по</span>
      <input type="date" bind:value={expires} min={today()} disabled={busy} />
    </label>
    <p class="help">
      Пустые поля — без пароля и бессрочно. Если ссылка уже есть, она будет заменена новой: старая
      перестанет открываться.
    </p>

    {#if error}
      <div class="error">{error}</div>
    {/if}

    <footer>
      <button type="button" class="btn" onclick={() => dialog.close()} disabled={busy}>Отмена</button>
      <button type="submit" class="btn primary" disabled={busy}>
        {busy ? "Создание…" : "Создать ссылку"}
      </button>
    </footer>
  </form>
</dialog>

<style>
  dialog {
    width: min(420px, calc(100vw - 32px));
  }

  form {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 18px 20px;
  }

  h2 {
    margin: 0;
    overflow: hidden;
    font-size: 16px;
    font-weight: 600;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  input {
    height: 30px;
    padding: 0 8px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg);
    color: inherit;
  }

  input:focus {
    border-color: var(--accent);
    outline: none;
  }

  .help {
    margin: 0;
    color: var(--muted);
    font-size: 12px;
  }

  footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 4px;
  }
</style>
