/** What the preview window can show, by file extension. */
export type PreviewKind = "image" | "video" | "audio" | "pdf" | "text";

const KINDS: Record<PreviewKind, string[]> = {
  image: ["jpg", "jpeg", "png", "gif", "webp", "bmp", "svg", "ico", "avif"],
  video: ["mp4", "m4v", "webm", "mov", "ogv"],
  audio: ["mp3", "m4a", "aac", "wav", "ogg", "oga", "opus", "flac"],
  pdf: ["pdf"],
  text: [
    "txt", "md", "log", "csv", "tsv", "json", "xml", "yaml", "yml", "toml", "ini", "cfg", "conf",
    "env", "sql", "html", "htm", "css", "js", "ts", "jsx", "tsx", "svelte", "vue", "py", "rs", "go",
    "java", "kt", "c", "h", "cpp", "hpp", "cs", "php", "rb", "sh", "bat", "cmd", "ps1", "srt",
    "gitignore", "dockerfile",
  ],
};

const BY_EXTENSION = new Map(
  Object.entries(KINDS).flatMap(([kind, exts]) => exts.map((ext) => [ext, kind as PreviewKind])),
);

export function previewKind(name: string): PreviewKind | null {
  const dot = name.lastIndexOf(".");
  const ext = (dot >= 0 ? name.slice(dot + 1) : name).toLowerCase();
  return BY_EXTENSION.get(ext) ?? null;
}

/** Text kinds that Windows runs instead of showing when opened in "the program";
 * as `RUNNABLE` in `preview.rs`, which refuses them. */
const RUNNABLE = new Set(["bat", "cmd", "js", "ps1"]);

export function runsWhenOpened(name: string): boolean {
  const dot = name.lastIndexOf(".");
  return dot >= 0 && RUNNABLE.has(name.slice(dot + 1).toLowerCase());
}

/** Larger files are downloaded for preview only after confirmation. */
export const PREVIEW_CONFIRM_SIZE = 100 * 1024 * 1024;
/** Text larger than this is not previewed at all. */
export const TEXT_PREVIEW_MAX = 20 * 1024 * 1024;
