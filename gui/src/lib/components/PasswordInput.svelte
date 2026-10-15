<script lang="ts">
  import Icon from "./Icon.svelte";

  let {
    value = $bindable(""),
    placeholder,
    autocomplete = "off",
    disabled = false,
    oninput,
  }: {
    value?: string;
    placeholder?: string;
    autocomplete?: HTMLInputElement["autocomplete"];
    disabled?: boolean;
    oninput?: (value: string) => void;
  } = $props();

  let shown = $state(false);
</script>

<span class="password">
  <input
    type={shown ? "text" : "password"}
    bind:value
    {placeholder}
    {autocomplete}
    spellcheck="false"
    {disabled}
    oninput={(e) => oninput?.(e.currentTarget.value)}
  />
  <button
    type="button"
    class="toggle"
    title={shown ? "Скрыть пароль" : "Показать пароль"}
    aria-label={shown ? "Скрыть пароль" : "Показать пароль"}
    aria-pressed={shown}
    onclick={() => (shown = !shown)}
    {disabled}
  >
    <Icon name={shown ? "eyeOff" : "eye"} size={15} />
  </button>
</span>

<style>
  .password {
    position: relative;
    display: flex;
  }

  input {
    flex: 1;
    min-width: 0;
    height: 30px;
    padding: 0 34px 0 8px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg);
    color: inherit;
  }

  input:focus {
    border-color: var(--accent);
    outline: none;
  }

  .toggle {
    position: absolute;
    top: 3px;
    right: 3px;
    display: grid;
    place-items: center;
    width: 26px;
    height: 24px;
    padding: 0;
    border: 0;
    border-radius: 4px;
    background: none;
    color: var(--muted);
  }

  .toggle:hover:not(:disabled) {
    background: var(--hover);
    color: var(--text);
  }
</style>
