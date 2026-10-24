<!--
  A PDF on Android, whose WebView cannot show one: the pages are drawn as
  pictures by the system's PdfRenderer, as wide as the screen, each when it
  is about to scroll into view.
-->
<script lang="ts">
  import { convertFileSrc } from "@tauri-apps/api/core";
  import { api, errorText, type PdfPageSize } from "$lib/api";

  let { path, onfail }: {
    /** The cached preview. */
    path: string;
    onfail: (error: string) => void;
  } = $props();

  let box: HTMLDivElement;
  let pages = $state<PdfPageSize[]>([]);
  /** Pictures of the pages drawn so far, by page index. */
  let pictures = $state<Record<number, string>>({});

  $effect(() => {
    const file = path;
    pages = [];
    pictures = {};
    api
      .pdfInfo(file)
      .then((info) => {
        if (file === path) pages = info.pages;
      })
      .catch((err) => onfail(errorText(err)));
  });

  async function draw(index: number) {
    const file = path;
    // Sharp on the screen, within what a phone draws quickly.
    const width = Math.min(2048, Math.round(box.clientWidth * devicePixelRatio));
    try {
      const picture = await api.pdfPage(file, index, width);
      if (file === path) pictures[index] = convertFileSrc(picture);
    } catch {
      // The page stays blank; the others still show.
    }
  }

  /** Draws the page once it comes near the visible part. */
  function whenNear(node: HTMLElement, index: number) {
    const observer = new IntersectionObserver(
      (entries) => {
        if (!entries.some((e) => e.isIntersecting)) return;
        observer.disconnect();
        draw(index);
      },
      { root: box, rootMargin: "100% 0px" },
    );
    observer.observe(node);
    return { destroy: () => observer.disconnect() };
  }
</script>

<div class="pdf" bind:this={box}>
  {#each pages as page, i (i)}
    <div class="page" style:aspect-ratio="{page.width} / {page.height}" use:whenNear={i}>
      {#if pictures[i]}
        <img src={pictures[i]} alt="Страница {i + 1}" />
      {/if}
    </div>
  {/each}
</div>

<style>
  .pdf {
    align-self: stretch;
    justify-self: stretch;
    display: flex;
    flex-direction: column;
    gap: 8px;
    overflow: auto;
    padding: 8px 0;
    background: var(--hover);
  }

  .page {
    flex: none;
    width: 100%;
    background: #fff;
  }

  img {
    display: block;
    width: 100%;
    height: 100%;
  }
</style>
