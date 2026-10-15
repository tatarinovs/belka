<script lang="ts">
  import {
    api,
    errorText,
    type Account,
    type CryptFolder,
    type OAuthApp,
    type OAuthService,
    type ProviderKind,
  } from "$lib/api";
  import { PROVIDERS, buildConfig, missingFields, providerDef } from "$lib/providers";
  import PasswordInput from "./PasswordInput.svelte";

  let { open, account = null, accounts, onclose, onsaved }: {
    open: boolean;
    /** The account whose settings are edited; a new one is added without it. */
    account?: Account | null;
    /** Clouds an encrypted folder can be placed in. */
    accounts: Account[];
    onclose: () => void;
    onsaved: (account: Account) => void;
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
  let crypt = $state<CryptFolder>(emptyCrypt());
  let repeat = $state("");
  /** Settings of the edited account as loaded, to tell whether they changed. */
  let loadedConfig = $state("");
  let loading = $state(false);

  function emptyCrypt(): CryptFolder {
    return { base: "", root: "/Encrypted", password: "", salt: "", encrypt_names: true };
  }

  const clouds = $derived(accounts.filter((a) => a.kind !== "crypt"));

  const def = $derived(providerDef(kind));
  const service = $derived<OAuthService | null>(kind === "google" || kind === "yandex" ? kind : null);
  const browserMode = $derived(service !== null && !manual);
  const missing = $derived(missingFields(def, values));

  const editing = $derived(account !== null);

  $effect(() => {
    if (open && !dialog.open) {
      error = "";
      if (account) {
        loadSettings(account);
      } else {
        name = "";
        selectKind("yandex");
      }
      dialog.showModal();
    } else if (!open && dialog.open) {
      dialog.close();
    }
  });

  /** Fills the form with the stored settings of `target`. */
  async function loadSettings(target: Account) {
    kind = target.kind;
    name = target.name;
    values = {};
    loadedConfig = "";
    // Stored tokens are shown as fields; signing in again is one click away.
    manual = true;
    if (target.kind === "google" || target.kind === "yandex") loadOAuthApp(target.kind);
    if (target.kind === "crypt") return;
    loading = true;
    try {
      const config = await api.accountConfig(target.id);
      if (account?.id !== target.id || !config) return;
      const def = providerDef(target.kind);
      const loaded: Record<string, string | boolean> = {};
      for (const field of def.fields) {
        const value = config[field.key];
        if (typeof value === "boolean") loaded[field.key] = value;
        else if (value !== undefined && value !== null) loaded[field.key] = String(value);
      }
      values = loaded;
      loadedConfig = JSON.stringify(buildConfig(def, loaded));
    } catch (err) {
      error = errorText(err);
    } finally {
      loading = false;
    }
  }

  async function selectKind(next: ProviderKind) {
    kind = next;
    values = {};
    error = "";
    manual = false;
    if (next === "crypt") {
      crypt = { ...emptyCrypt(), base: clouds[0]?.id ?? "" };
      repeat = "";
    }
    if (next === "google" || next === "yandex") await loadOAuthApp(next);
  }

  /** Which OAuth app a browser sign-in to `service` uses by default. */
  async function loadOAuthApp(service: OAuthService) {
    app = { client_id: "", client_secret: "" };
    hasBuiltin = false;
    useCustom = false;
    const apps = await api.oauthApps(service).catch(() => null);
    if (kind !== service) return;
    if (apps?.custom) app = apps.custom;
    hasBuiltin = apps?.builtin ?? false;
    useCustom = !hasBuiltin || apps?.custom != null;
  }

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    if (busy || loading) return;
    error = "";
    if (browserMode && service) {
      await signIn(service);
      return;
    }
    if (kind === "crypt" && !account) {
      await addCrypt();
      return;
    }
    if (kind !== "crypt" && missing.length) {
      error = `Заполните: ${missing.join(", ")}`;
      return;
    }
    if (account && !name.trim()) {
      error = "Укажите название.";
      return;
    }
    busy = true;
    try {
      const config = kind === "crypt" ? null : buildConfig(def, values);
      const saved = account
        ? // Only changed credentials are checked against the cloud.
          await api.updateAccount(
            account.id,
            name.trim(),
            config && JSON.stringify(config) !== loadedConfig ? config : null,
          )
        : await api.addAccount(name.trim() || def.title, config!);
      onsaved(saved);
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
      const saved = await api.oauthSignIn(service, useCustom ? app : null, name, account?.id ?? null);
      onsaved(saved);
    } catch (err) {
      const text = errorText(err);
      if (!text.includes("Sign-in cancelled")) error = text;
    } finally {
      busy = false;
      waiting = false;
    }
  }

  async function addCrypt() {
    const base = clouds.find((a) => a.id === crypt.base);
    if (!base) error = "Выберите облако.";
    else if (!crypt.root.trim()) error = "Укажите папку в облаке.";
    else if (!crypt.password) error = "Задайте пароль.";
    else if (crypt.password !== repeat) error = "Пароли не совпадают.";
    if (error || !base) return;
    busy = true;
    try {
      const root = crypt.root.trim();
      const saved = await api.addCryptFolder(name.trim() || `${base.name} · ${root} 🔒`, {
        ...crypt,
        root,
      });
      onsaved(saved);
    } catch (err) {
      error = errorText(err);
    } finally {
      busy = false;
    }
  }

  function close() {
    if (waiting) api.oauthCancel();
    onclose();
  }
</script>

<dialog class="modal"
  bind:this={dialog}
  onclose={close}
  oncancel={(e) => {
    // Esc aborts a browser sign-in, but not a credentials check.
    if (busy && !waiting) e.preventDefault();
  }}
>
  <form onsubmit={submit}>
    <h2>{editing ? "Настройки аккаунта" : "Новый аккаунт"}</h2>

    {#if editing}
      <p class="kind-fixed">{def.title}</p>
    {:else}
      <label>
        <span>Тип</span>
        <select
          value={kind}
          onchange={(e) => selectKind(e.currentTarget.value as ProviderKind)}
          disabled={busy}
        >
          {#each PROVIDERS as p (p.kind)}
            <option value={p.kind}>{p.title}</option>
          {/each}
        </select>
      </label>
    {/if}

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
        {#if browserMode}
          {editing ? "Войдите заново, если доступ отозван или истёк: " : "Вход через браузер: "}вы разрешите
          доступ на странице {def.title}.
        {:else if editing && kind === "crypt"}
          Можно изменить название. Пароль и папку шифрования изменить нельзя: для этого создайте новую
          зашифрованную папку.
        {:else}
          {def.help}
        {/if}
      </p>

      <label>
        <span>Название</span>
        <input
          bind:value={name}
          placeholder={editing ? "" : browserMode ? "По умолчанию — e-mail или логин" : def.title}
          disabled={busy}
        />
      </label>

      {#if kind === "crypt" && editing}
        <!-- Only the name of an encrypted folder can be changed. -->
      {:else if kind === "crypt"}
        {#if !clouds.length}
          <p class="error">Сначала подключите облако, в котором будет лежать папка.</p>
        {:else}
          <label>
            <span>Облако *</span>
            <select bind:value={crypt.base} disabled={busy}>
              {#each clouds as cloud (cloud.id)}
                <option value={cloud.id}>{cloud.name} — {providerDef(cloud.kind).title}</option>
              {/each}
            </select>
          </label>
          <label>
            <span>Папка в облаке *</span>
            <input bind:value={crypt.root} spellcheck="false" disabled={busy} />
          </label>
          <label>
            <span>Пароль *</span>
            <PasswordInput bind:value={crypt.password} autocomplete="new-password" disabled={busy} />
          </label>
          <label>
            <span>Пароль ещё раз *</span>
            <PasswordInput bind:value={repeat} autocomplete="new-password" disabled={busy} />
          </label>
          <label class="check">
            <input type="checkbox" bind:checked={crypt.encrypt_names} disabled={busy} />
            <span>Шифровать имена файлов и папок</span>
          </label>
          <details>
            <summary>Совместимость с rclone</summary>
            <p class="help small">
              Файлы можно открыть через rclone: remote типа crypt на эту папку с тем же паролем,
              filename_encryption = {crypt.encrypt_names ? "standard" : "off"}.
            </p>
            <label>
              <span>Соль (password2 в rclone), если задана</span>
              <PasswordInput bind:value={crypt.salt} disabled={busy} />
            </label>
          </details>
          <p class="warning">
            Пароль невозможно восстановить: без него файлы не расшифровать ни Belka, ни кому-либо
            ещё. Сохраните его в надёжном месте.
          </p>
        {/if}
      {:else if browserMode && service}
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
              <PasswordInput bind:value={app.client_secret} disabled={busy} />
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
              {#if field.type === "password"}
                <PasswordInput
                  value={(values[field.key] as string) ?? ""}
                  placeholder={field.placeholder}
                  oninput={(v) => (values[field.key] = v)}
                  disabled={busy || loading}
                />
              {:else}
                <input
                  type={field.type ?? "text"}
                  placeholder={field.placeholder}
                  value={(values[field.key] as string) ?? ""}
                  oninput={(e) => (values[field.key] = e.currentTarget.value)}
                  autocomplete="off"
                  spellcheck="false"
                  disabled={busy || loading}
                />
              {/if}
              {#if field.hint}
                <span class="help small">{field.hint}</span>
              {/if}
            </label>
          {/if}
        {/each}
      {/if}

      {#if service}
        <button type="button" class="link" onclick={() => ((manual = !manual), (error = ""))} disabled={busy}>
          {manual ? (editing ? "Войти заново через браузер" : "Войти через браузер") : "Ввести токен вручную"}
        </button>
      {/if}
    {/if}

    {#if error}
      <div class="error">{error}</div>
    {/if}

    <footer>
      <span class="note">
        {kind === "crypt" ? "Пароль хранится" : "Данные входа хранятся"} в системном хранилище учётных данных.
      </span>
      {#if waiting}
        <button type="button" class="btn" onclick={() => api.oauthCancel()}>Отменить вход</button>
      {:else}
        <button type="button" class="btn" onclick={close} disabled={busy}>Отмена</button>
        <button type="submit" class="btn primary" disabled={busy}>
          {busy
            ? "Проверка…"
            : browserMode
              ? "Войти через браузер"
              : editing
                ? "Сохранить"
                : kind === "crypt"
                  ? "Создать"
                  : "Подключить"}
        </button>
      {/if}
    </footer>
  </form>
</dialog>

<style>
  dialog {
    width: min(520px, calc(100vw - 32px));
    max-height: calc(100vh - 32px);
  }

  form {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 18px 20px;
  }

  .kind-fixed {
    margin: 0;
    font-weight: 600;
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

  label.check {
    flex-direction: row;
    align-items: center;
    gap: 8px;
  }

  label.check > span {
    color: var(--text);
    font-size: inherit;
  }

  .warning {
    margin: 0;
    padding: 8px 10px;
    border-radius: 6px;
    background: color-mix(in srgb, #e0a43a 18%, transparent);
    font-size: 12px;
  }

  details summary {
    cursor: pointer;
    color: var(--muted);
    font-size: 12px;
  }

  details[open] {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  select,
  input:not([type="checkbox"]) {
    height: 30px;
    padding: 0 8px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg);
  }

  select:focus,
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
