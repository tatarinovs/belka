<script lang="ts">
  import { untrack } from "svelte";
  import { convertFileSrc } from "@tauri-apps/api/core";
  import { api, errorText, type Account, type Entry, type Preview } from "$lib/api";
  import { formatBytes } from "$lib/format";
  import { PREVIEW_CONFIRM_SIZE, TEXT_PREVIEW_MAX, previewKind, runsWhenOpened } from "$lib/preview";
  import { isMobile } from "$lib/platform";
  import Icon from "./Icon.svelte";
  import PdfPages from "./PdfPages.svelte";

  let { account, files, start, onclose }: {
    account: Account;
    /** Files of the open folder; arrows move between them. */
    files: Entry[];
    start: number;
    onclose: () => void;
  } = $props();

  let dialog: HTMLDialogElement;
  let index = $state(0);
  let preview = $state<Preview | null>(null);
  let loading = $state(false);
  let error = $state("");
  let progress = $state<[number, number]>([0, 0]);
  /** What the download is doing when no data comes, or "". */
  let stalled = $state("");
  /** Large files the user agreed to download. */
  let confirmed = $state(new Set<string>());
  /** URL of the media file playing while it downloads, or "". */
  let streamSrc = $state("");
  /** Media files the cloud cannot stream: they are downloaded whole. */
  let unstreamable = $state(new Set<string>());
  /** Why the PDF being shown cannot be drawn on Android, or "". */
  let pdfError = $state("");
  /** The webview cannot play the media file: a codec it lacks, say. */
  let mediaFailed = $state(false);
  let requestSeq = 0;

  const entry = $derived(files[index]);
  const kind = $derived(entry ? previewKind(entry.name, entry.size) : null);
  const media = $derived(kind === "video" || kind === "audio");
  const tooLarge = $derived(
    !!entry &&
      ((kind === "text" && entry.size > TEXT_PREVIEW_MAX) ||
        (entry.size > PREVIEW_CONFIRM_SIZE &&
          !confirmed.has(entry.path) &&
          // Streaming fetches only what is played, so it needs no consent;
          // an external player streams too.
          kind !== "player" &&
          (!media || unstreamable.has(entry.path)))),
  );
  const src = $derived(streamSrc || (preview ? convertFileSrc(preview.path) : ""));
  /** A streamed video goes to an external player, other files to their program. */
  const canOpen = $derived(!!preview || (kind === "video" && !!streamSrc) || kind === "player");
  /** A phone held sideways shows a video alone across the whole screen. */
  let landscape = $state(false);
  const immersive = $derived(isMobile && landscape && kind === "video" && !!src);

  $effect(() => {
    if (!isMobile) return;
    const query = matchMedia("(orientation: landscape)");
    const update = () => (landscape = query.matches);
    update();
    query.addEventListener("change", update);
    return () => query.removeEventListener("change", update);
  });

  // The system bars go away too; a swipe from the edge brings them back.
  $effect(() => {
    if (!immersive) return;
    api.setImmersive(true).catch(() => {});
    return () => void api.setImmersive(false).catch(() => {});
  });

  $effect(() => {
    index = start;
    dialog.showModal();
    return () => api.previewCancel();
  });

  $effect(() => {
    if (entry && kind && !tooLarge) untrack(() => load(entry));
    else {
      requestSeq++;
      preview = null;
      streamSrc = "";
      loading = false;
      error = "";
    }
  });

  async function load(item: Entry) {
    const seq = ++requestSeq;
    preview = null;
    streamSrc = "";
    pdfError = "";
    mediaFailed = false;
    error = "";
    // An external player streams the file itself.
    if (kind === "player") return;
    loading = true;
    progress = [0, item.size];
    stalled = "";
    if (media && !unstreamable.has(item.path)) {
      const url = await api.streamOpen(account.id, item).catch(() => null);
      if (seq !== requestSeq) return;
      if (url) {
        streamSrc = url;
        loading = false;
        return;
      }
      unstreamable = new Set(unstreamable).add(item.path);
      // A large file now asks for consent first; the effect shows that.
      if (item.size > PREVIEW_CONFIRM_SIZE && !confirmed.has(item.path)) return;
    }
    const started = Date.now();
    const timer = setInterval(async () => {
      const { done, total, attempt, max_attempts } = await api.previewProgress();
      if (seq !== requestSeq) return;
      progress = [done, total || item.size];
      if (attempt > 1) stalled = `Связь прервалась · попытка ${attempt} из ${max_attempts}`;
      else if (!done && Date.now() - started > 5000) stalled = "Ждём ответа сервера…";
      else stalled = "";
    }, 200);
    try {
      const result = await api.previewFile(account.id, item, kind === "text");
      if (seq === requestSeq) preview = result;
    } catch (err) {
      const text = errorText(err);
      if (seq === requestSeq && !text.includes("Preview cancelled")) error = text;
    } finally {
      clearInterval(timer);
      if (seq === requestSeq) loading = false;
    }
  }

  function go(step: number) {
    const next = index + step;
    if (next >= 0 && next < files.length) index = next;
  }

  function openExternally() {
    if (kind === "player" || (kind === "video" && streamSrc)) playExternally();
    else if (preview) api.previewOpen(preview.path).catch((err) => (error = errorText(err)));
  }

  /** VLC and the like: the program set for the type, or the app the user picks on Android. */
  function playExternally() {
    if (entry) api.streamPlay(account.id, entry).catch((err) => (error = errorText(err)));
  }

  // A horizontal swipe over the picture moves to the next or previous file.
  let swipe: { x: number; y: number } | null = null;

  function swipeStart(event: PointerEvent) {
    if (!isMobile || event.target instanceof HTMLMediaElement) return;
    swipe = { x: event.clientX, y: event.clientY };
  }

  function swipeEnd(event: PointerEvent) {
    if (!swipe) return;
    const dx = event.clientX - swipe.x;
    const dy = event.clientY - swipe.y;
    swipe = null;
    if (Math.abs(dx) > 60 && Math.abs(dx) > 1.5 * Math.abs(dy)) go(dx < 0 ? 1 : -1);
  }

  function onkeydown(event: KeyboardEvent) {
    // Media elements use the arrows for seeking.
    if (event.target instanceof HTMLMediaElement) return;
    if (event.key === "ArrowLeft") go(-1);
    else if (event.key === "ArrowRight") go(1);
    else return;
    event.preventDefault();
  }
</script>

<dialog
  class="modal"
  class:phone={isMobile}
  class:immersive
  bind:this={dialog}
  onclose={onclose}
  {onkeydown}
>
  {#if isMobile}
    <!-- On a phone the arrows move to a bar at the bottom, within thumb reach. -->
    <header>
      <button class="icon-btn" aria-label="Закрыть" onclick={() => dialog.close()}>
        <Icon name="close" size={22} />
      </button>
      <div class="title">
        <span class="name">{entry?.name}</span>
        <span class="meta">{formatBytes(entry?.size ?? 0)}</span>
      </div>
      <button class="icon-btn" aria-label="Открыть в другом приложении" onclick={openExternally} disabled={!canOpen}>
        <Icon name="external" size={22} />
      </button>
    </header>
  {:else}
    <header>
      <span class="name" title={entry?.path}>{entry?.name}</span>
      <span class="meta">
        {formatBytes(entry?.size ?? 0)}{#if files.length > 1} · {index + 1} из {files.length}{/if}
      </span>
      <button class="icon-btn" title="Предыдущий (←)" onclick={() => go(-1)} disabled={index === 0}>
        <Icon name="arrowLeft" />
      </button>
      <button class="icon-btn" title="Следующий (→)" onclick={() => go(1)} disabled={index >= files.length - 1}>
        <Icon name="arrowRight" />
      </button>
      <button
        class="btn"
        onclick={openExternally}
        disabled={!canOpen || runsWhenOpened(entry?.name ?? "")}
        title={runsWhenOpened(entry?.name ?? "")
          ? "Такой файл при открытии запустится как программа"
          : "Открыть в программе, назначенной в системе"}
      >
        Открыть в программе
      </button>
      <button class="icon-btn" title="Закрыть (Esc)" onclick={() => dialog.close()}>
        <Icon name="close" />
      </button>
    </header>
  {/if}

  <div
    class="stage"
    role="presentation"
    onpointerdown={swipeStart}
    onpointerup={swipeEnd}
    onpointercancel={() => (swipe = null)}
  >
    {#if !kind}
      <p class="message">Предпросмотр для файлов этого типа недоступен.</p>
    {:else if kind === "text" && entry.size > TEXT_PREVIEW_MAX}
      <p class="message">Текстовый файл слишком большой для предпросмотра.</p>
    {:else if tooLarge}
      <div class="message">
        <p>Для просмотра файл нужно скачать целиком: {formatBytes(entry.size)}.</p>
        <button class="btn primary" onclick={() => (confirmed = new Set(confirmed).add(entry.path))}>
          Скачать и показать
        </button>
      </div>
    {:else if error}
      <p class="message error">{error}</p>
    {:else if kind === "player"}
      <div class="message">
        <p>Это видео встроенный плеер не воспроизводит: его покажет VLC или другой видеоплеер.</p>
        <button class="btn primary" onclick={playExternally}>Открыть в плеере</button>
      </div>
    {:else if media && mediaFailed}
      <div class="message">
        <p>Встроенный плеер не может воспроизвести этот файл.</p>
        <button class="btn primary" onclick={playExternally}>Открыть в плеере</button>
      </div>
    {:else if loading || !src}
      <div class="message">
        <p>Загрузка… {progress[1] ? Math.floor((progress[0] / progress[1]) * 100) : 0}%</p>
        <div class="bar"><div style:width="{progress[1] ? (progress[0] / progress[1]) * 100 : 0}%"></div></div>
        {#if stalled}
          <p class="stalled">{stalled}</p>
        {/if}
      </div>
    {:else if kind === "image"}
      <img {src} alt={entry.name} />
    {:else if kind === "video"}
      <!-- Android WebView refuses the player's own full screen: turning
           the phone does it instead. -->
      <!-- svelte-ignore a11y_media_has_caption -->
      <video
        {src}
        controls
        autoplay
        controlslist={isMobile ? "nofullscreen" : undefined}
        onerror={() => (mediaFailed = true)}
      ></video>
    {:else if kind === "audio"}
      <audio {src} controls autoplay onerror={() => (mediaFailed = true)}></audio>
    {:else if kind === "pdf" && isMobile && !pdfError}
      <PdfPages path={preview!.path} onfail={(err) => (pdfError = err)} />
    {:else if kind === "pdf" && isMobile}
      <!-- A PDF with a password, or one the system cannot read. -->
      <div class="message">
        <p>Этот PDF не получается показать здесь: {pdfError}</p>
        <button class="btn primary" onclick={openExternally}>Открыть в другом приложении</button>
      </div>
    {:else if kind === "pdf"}
      <iframe {src} title={entry.name}></iframe>
    {:else if kind === "text"}
      <div class="text">
        {#if preview?.truncated}
          <p class="note">Показан первый мегабайт файла.</p>
        {/if}
        <pre>{preview?.text}</pre>
      </div>
    {/if}
  </div>

  {#if isMobile && files.length > 1}
    <footer>
      <button class="icon-btn" aria-label="Предыдущий" onclick={() => go(-1)} disabled={index === 0}>
        <Icon name="arrowLeft" size={24} />
      </button>
      <span>{index + 1} из {files.length}</span>
      <button class="icon-btn" aria-label="Следующий" onclick={() => go(1)} disabled={index >= files.length - 1}>
        <Icon name="arrowRight" size={24} />
      </button>
    </footer>
  {/if}
</dialog>

<style>
  dialog {
    display: flex;
    flex-direction: column;
    width: calc(100vw - 64px);
    height: calc(100vh - 64px);
    max-width: none;
    max-height: none;
  }

  dialog:not([open]) {
    display: none;
  }

  dialog::backdrop {
    background: rgb(0 0 0 / 0.55);
  }

  header {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 10px 8px 16px;
    border-bottom: 1px solid var(--border);
  }

  .name {
    overflow: hidden;
    font-weight: 600;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .meta {
    flex: 1;
    color: var(--muted);
    font-size: 12px;
    white-space: nowrap;
  }

  /* Phone: the whole screen, a compact title and a bottom bar with the arrows. */
  dialog.modal.phone {
    width: 100vw;
    height: 100dvh;
    max-height: none;
    margin: 0;
    border: 0;
    border-radius: 0;
  }

  .phone header {
    gap: 4px;
    padding: 4px;
  }

  .phone .title {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .phone header .icon-btn,
  .phone footer .icon-btn {
    width: 48px;
    height: 48px;
    border-radius: 24px;
  }

  .phone .stage {
    touch-action: pan-y pinch-zoom;
  }

  dialog.modal.phone footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 4px 12px;
    border-top: 1px solid var(--border);
    color: var(--muted);
  }

  /* Phone held sideways: the video alone on black, as in a player. */
  dialog.modal.phone.immersive header,
  dialog.modal.phone.immersive footer {
    display: none;
  }

  .immersive .stage {
    background: #000;
  }

  .immersive video {
    width: 100%;
    height: 100%;
  }

  .stage {
    flex: 1;
    display: grid;
    place-items: center;
    min-height: 0;
    overflow: hidden;
    background: var(--bg);
  }

  img,
  video {
    max-width: 100%;
    max-height: 100%;
    object-fit: contain;
  }

  iframe {
    width: 100%;
    height: 100%;
    border: 0;
  }

  .stalled {
    color: var(--muted);
    font-size: 12px;
  }

  .text {
    align-self: stretch;
    justify-self: stretch;
    overflow: auto;
    padding: 12px 16px;
  }

  pre {
    margin: 0;
    font-family: Consolas, "Cascadia Mono", monospace;
    font-size: 12.5px;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .note {
    margin: 0 0 8px;
    color: var(--muted);
    font-size: 12px;
  }

  .message {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    margin: 0;
    padding: 24px;
    color: var(--muted);
    text-align: center;
  }

  .message p {
    margin: 0;
  }

  .bar {
    width: 240px;
    height: 4px;
    overflow: hidden;
    border-radius: 2px;
    background: var(--hover);
  }

  .bar div {
    height: 100%;
    background: var(--accent);
    transition: width 0.2s;
  }
</style>
