<script lang="ts">
  import { api, errorText, type Account, type ProviderKind } from "$lib/api";
  import { PROVIDERS, buildConfig, missingFields, providerDef } from "$lib/providers";

  let { open, onclose, onadded }: {
    open: boolean;
    onclose: () => void;
    onadded: (account: Account) => void;
  } = $props();

  let dialog: HTMLDialogElement;
  let kind = $state<ProviderKind>("yandex");
  let name = $state("");
  let values = $state<Record<string, string | boolean>>({});
  let busy = $state(false);
  let error = $state("");

  const def = $derived(providerDef(kind));
  const missing = $derived(missingFields(def, values));

  $effect(() => {
    if (open && !dialog.open) {
      kind = "yandex";
      name = "";
      values = {};
      error = "";
      dialog.showModal();
    } else if (!open && dialog.open) {
      dialog.close();
    }
  });

  function selectKind(next: ProviderKind) {
    kind = next;
    values = {};
    error = "";
  }

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    if (busy) return;
    if (missing.length) {
      error = `Заполните: ${missing.join(", ")}`;
      return;
    }
    busy = true;
    error = "";
    try {
      const account = await api.addAccount(name.trim() || def.title, buildConfig(def, values));
      onadded(account);
    } catch (err) {
      error = errorText(err);
    } finally {
      busy = false;
    }
  }
</script>

<dialog bind:this={dialog} onclose={onclose} oncancel={(e) => busy && e.preventDefault()}>
  <form onsubmit={submit}>
    <h2>Новый аккаунт</h2>

    <div class="kinds" role="radiogroup" aria-label="Тип облака">
      {#each PROVIDERS as p (p.kind)}
        <button
          type="button"
          role="radio"
          aria-checked={p.kind === kind}
          class:active={p.kind === kind}
          disabled={busy}
          onclick={() => selectKind(p.kind)}
        >
          {p.title}
        </button>
      {/each}
    </div>

    <p class="help">{def.help}</p>

    <label>
      <span>Название</span>
      <input bind:value={name} placeholder={def.title} disabled={busy} />
    </label>

    {#each def.fields as field (def.kind + field.key)}
      {#if field.type === "checkbox"}
        <label class="check">
          <input
            type="checkbox"
            checked={values[field.key] === true}
            onchange={(e) => (values[field.key] = e.currentTarget.checked)}
            disabled={busy}
          />
          <span>{field.label}</span>
        </label>
      {:else}
        <label>
          <span>{field.label}{field.optional ? "" : " *"}</span>
          <input
            type={field.type ?? "text"}
            placeholder={field.placeholder}
            value={(values[field.key] as string) ?? ""}
            oninput={(e) => (values[field.key] = e.currentTarget.value)}
            autocomplete="off"
            spellcheck="false"
            disabled={busy}
          />
        </label>
      {/if}
    {/each}

    {#if error}
      <div class="error">{error}</div>
    {/if}

    <footer>
      <span class="note">Данные входа хранятся в системном хранилище учётных данных.</span>
      <button type="button" class="btn" onclick={onclose} disabled={busy}>Отмена</button>
      <button type="submit" class="btn primary" disabled={busy}>
        {busy ? "Проверка…" : "Подключить"}
      </button>
    </footer>
  </form>
</dialog>

<style>
  dialog {
    width: min(520px, calc(100vw - 32px));
    max-height: calc(100vh - 32px);
    padding: 0;
    border: 1px solid var(--border);
    border-radius: 10px;
    background: var(--surface);
    color: var(--text);
    box-shadow: var(--shadow);
  }

  dialog::backdrop {
    background: rgb(0 0 0 / 0.35);
  }

  form {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 18px 20px;
  }

  h2 {
    margin: 0;
    font-size: 16px;
    font-weight: 600;
  }

  .kinds {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .kinds button {
    height: 28px;
    padding: 0 10px;
    border: 1px solid var(--border);
    border-radius: 14px;
    background: var(--surface);
  }

  .kinds button:hover:not(:disabled) {
    background: var(--hover);
  }

  .kinds button.active {
    border-color: var(--accent);
    background: var(--selected);
  }

  .help {
    margin: 0;
    color: var(--muted);
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  label > span {
    color: var(--muted);
    font-size: 12px;
  }

  label.check {
    flex-direction: row;
    align-items: center;
    gap: 8px;
  }

  label.check > span {
    color: var(--text);
    font-size: inherit;
  }

  input:not([type="checkbox"]) {
    height: 30px;
    padding: 0 8px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg);
  }

  input:not([type="checkbox"]):focus {
    border-color: var(--accent);
    outline: none;
  }

  footer {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 4px;
  }

  .note {
    flex: 1;
    color: var(--muted);
    font-size: 11px;
  }
</style>
