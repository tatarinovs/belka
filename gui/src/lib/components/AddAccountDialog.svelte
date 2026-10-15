<script lang="ts">
  import {
    api,
    errorText,
    type Account,
    type OAuthApp,
    type OAuthService,
    type ProviderKind,
  } from "$lib/api";
  import { PROVIDERS, buildConfig, missingFields, providerDef } from "$lib/providers";

  let { open, onclose, onadded }: {
    open: boolean;
    onclose: () => void;
    onadded: (account: Account) => void;
  } = $props();

  const OAUTH_HELP: Record<OAuthService, { steps: string; secretRequired: boolean }> = {
    google: {
      steps:
        "Google Cloud Console → APIs & Services: включите Google Drive API и создайте OAuth client ID типа «Desktop app».",
      secretRequired: true,
    },
    yandex: {
      steps:
        "oauth.yandex.ru → «Создать приложение»: платформа «Веб-сервисы», Redirect URI http://127.0.0.1:53682/, доступы Яндекс.Диска: чтение, запись и информация о Диске.",
      secretRequired: false,
    },
  };

  let dialog: HTMLDialogElement;
  let kind = $state<ProviderKind>("yandex");
  let name = $state("");
  let values = $state<Record<string, string | boolean>>({});
  let busy = $state(false);
  let error = $state("");
  /** Google and Yandex sign in through the browser unless the user enters tokens. */
  let manual = $state(false);
  let app = $state<OAuthApp>({ client_id: "", client_secret: "" });
  /** The build has its own OAuth app, so the user's one is optional. */
  let hasBuiltin = $state(false);
  let useCustom = $state(false);
  let waiting = $state(false);

  const def = $derived(providerDef(kind));
  const service = $derived<OAuthService | null>(kind === "google" || kind === "yandex" ? kind : null);
  const browserMode = $derived(service !== null && !manual);
  const missing = $derived(missingFields(def, values));

  $effect(() => {
    if (open && !dialog.open) {
      name = "";
      error = "";
      selectKind("yandex");
      dialog.showModal();
    } else if (!open && dialog.open) {
      dialog.close();
    }
  });

  async function selectKind(next: ProviderKind) {
    kind = next;
    values = {};
    error = "";
    manual = false;
    if (next === "google" || next === "yandex") {
      app = { client_id: "", client_secret: "" };
      hasBuiltin = false;
      useCustom = false;
      const apps = await api.oauthApps(next).catch(() => null);
      if (kind !== next) return;
      if (apps?.custom) app = apps.custom;
      hasBuiltin = apps?.builtin ?? false;
      useCustom = !hasBuiltin || apps?.custom != null;
    }
  }

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    if (busy) return;
    error = "";
    if (browserMode && service) {
      await signIn(service);
      return;
    }
    if (missing.length) {
      error = `Заполните: ${missing.join(", ")}`;
      return;
    }
    busy = true;
    try {
      const account = await api.addAccount(name.trim() || def.title, buildConfig(def, values));
      onadded(account);
    } catch (err) {
      error = errorText(err);
    } finally {
      busy = false;
    }
  }

  async function signIn(service: OAuthService) {
    if (
      useCustom &&
      (!app.client_id.trim() || (OAUTH_HELP[service].secretRequired && !app.client_secret.trim()))
    ) {
      error = OAUTH_HELP[service].secretRequired
        ? "Укажите Client ID и Client secret OAuth-приложения."
        : "Укажите Client ID OAuth-приложения.";
      return;
    }
    busy = true;
    waiting = true;
    try {
      const account = await api.oauthSignIn(service, useCustom ? app : null, name);
      onadded(account);
    } catch (err) {
      const text = errorText(err);
      if (!text.includes("Sign-in cancelled")) error = text;
    } finally {
      busy = false;
      waiting = false;
    }
  }

  function close() {
    if (waiting) api.oauthCancel();
    onclose();
  }
</script>

<dialog
  bind:this={dialog}
  onclose={close}
  oncancel={(e) => {
    // Esc aborts a browser sign-in, but not a credentials check.
    if (busy && !waiting) e.preventDefault();
  }}
>
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

    {#if waiting}
      <div class="waiting">
        <div class="spinner" aria-hidden="true"></div>
        <p>
          Откройте вкладку браузера и разрешите доступ к {def.title}.<br />
          Окно можно не закрывать: вход завершится автоматически.
        </p>
      </div>
    {:else}
      <p class="help">
        {browserMode ? `Вход через браузер: вы разрешите доступ на странице ${def.title}.` : def.help}
      </p>

      <label>
        <span>Название</span>
        <input
          bind:value={name}
          placeholder={browserMode ? "По умолчанию — e-mail или логин" : def.title}
          disabled={busy}
        />
      </label>

      {#if browserMode && service}
        <div class="app">
          {#if hasBuiltin}
            <label class="check">
              <input type="checkbox" bind:checked={useCustom} disabled={busy} />
              <span>Своё OAuth-приложение</span>
            </label>
            {#if !useCustom}
              <p class="help small">Используется встроенное приложение Belka, ничего настраивать не нужно.</p>
            {/if}
          {:else}
            <p class="help small">В этой сборке нет встроенного приложения, укажите своё.</p>
          {/if}
          {#if useCustom}
            <p class="help small">{OAUTH_HELP[service].steps}</p>
            <label>
              <span>Client ID *</span>
              <input bind:value={app.client_id} autocomplete="off" spellcheck="false" disabled={busy} />
            </label>
            <label>
              <span>Client secret{OAUTH_HELP[service].secretRequired ? " *" : " (не обязателен)"}</span>
              <input
                type="password"
                bind:value={app.client_secret}
                autocomplete="off"
                spellcheck="false"
                disabled={busy}
              />
            </label>
          {/if}
        </div>
      {:else}
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
      {/if}

      {#if service}
        <button type="button" class="link" onclick={() => ((manual = !manual), (error = ""))} disabled={busy}>
          {manual ? "Войти через браузер" : "Ввести токен вручную"}
        </button>
      {/if}
    {/if}

    {#if error}
      <div class="error">{error}</div>
    {/if}

    <footer>
      <span class="note">Данные входа хранятся в системном хранилище учётных данных.</span>
      {#if waiting}
        <button type="button" class="btn" onclick={() => api.oauthCancel()}>Отменить вход</button>
      {:else}
        <button type="button" class="btn" onclick={close} disabled={busy}>Отмена</button>
        <button type="submit" class="btn primary" disabled={busy}>
          {busy ? "Проверка…" : browserMode ? "Войти через браузер" : "Подключить"}
        </button>
      {/if}
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

  .help.small {
    font-size: 12px;
  }

  .app {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 10px;
    border: 1px solid var(--border);
    border-radius: 8px;
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

  .link {
    align-self: flex-start;
    padding: 0;
    border: 0;
    background: none;
    color: var(--accent);
  }

  .link:hover:not(:disabled) {
    text-decoration: underline;
  }

  .waiting {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 12px 4px;
  }

  .waiting p {
    margin: 0;
    color: var(--muted);
  }

  .spinner {
    flex: none;
    width: 22px;
    height: 22px;
    border: 3px solid var(--hover);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .spinner {
      animation-duration: 3s;
    }
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
