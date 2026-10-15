<script lang="ts" module>
  const PATHS = {
    folder: "M3 6.5A1.5 1.5 0 0 1 4.5 5h4l2 2h9A1.5 1.5 0 0 1 21 8.5v9a1.5 1.5 0 0 1-1.5 1.5h-15A1.5 1.5 0 0 1 3 17.5z",
    file: "M6 3h8l5 5v12a1 1 0 0 1-1 1H6a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1zm8 0v5h5",
    up: "M12 19V5m-6 6 6-6 6 6",
    refresh: "M20 12a8 8 0 1 1-2.34-5.66M20 4v4h-4",
    plus: "M12 5v14M5 12h14",
    trash: "M4 7h16M9 7V4h6v3m-8 0 1 13h8l1-13",
    chevron: "m9 6 6 6-6 6",
    cloud: "M7 18a4 4 0 0 1-.6-7.95A6 6 0 0 1 18 9a4.5 4.5 0 0 1-.5 9z",
    upload: "M12 16V4m-5 5 5-5 5 5M5 20h14",
    download: "M12 4v12m-5-5 5 5 5-5M5 20h14",
    close: "M6 6l12 12M18 6 6 18",
    retry: "M4 12a8 8 0 1 0 2.34-5.66M4 4v4h4",
    reveal: "M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2zm9 3v6m-3-3 3 3 3-3",
    chevronDown: "m6 9 6 6 6-6",
    drive: "M3 13h18v5a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1zm0 0 2.5-7.2A1 1 0 0 1 6.4 5h11.2a1 1 0 0 1 .9.8L21 13M17 16h.01",
    place: "M3 10.5 12 3l9 7.5V20a1 1 0 0 1-1 1h-5v-6H9v6H4a1 1 0 0 1-1-1z",
    computer: "M3 5h18v11H3zm5 15h8m-4-4v4",
    arrowRight: "M5 12h14m-6-6 6 6-6 6",
    arrowLeft: "M19 12H5m6-6-6 6 6 6",
    link: "M10 14a4 4 0 0 0 5.66 0l3-3a4 4 0 0 0-5.66-5.66l-1 1M14 10a4 4 0 0 0-5.66 0l-3 3a4 4 0 0 0 5.66 5.66l1-1",
  } as const;
  export type IconName = keyof typeof PATHS;
</script>

<script lang="ts">
  let { name, size = 16 }: { name: IconName; size?: number } = $props();
  const filled = $derived(name === "folder");
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill={filled ? "currentColor" : "none"}
  stroke={filled ? "none" : "currentColor"}
  stroke-width="1.8"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
>
  <path d={PATHS[name]} />
</svg>
