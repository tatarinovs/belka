const UNITS = ["Б", "КБ", "МБ", "ГБ", "ТБ"];

export function formatBytes(bytes: number): string {
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < UNITS.length - 1) {
    value /= 1024;
    unit++;
  }
  const digits = unit === 0 || value >= 100 ? 0 : 1;
  return `${value.toLocaleString("ru-RU", { maximumFractionDigits: digits })} ${UNITS[unit]}`;
}

const dateFormat = new Intl.DateTimeFormat("ru-RU", {
  day: "2-digit",
  month: "2-digit",
  year: "numeric",
  hour: "2-digit",
  minute: "2-digit",
});

export function formatDate(iso: string | null): string {
  if (!iso) return "";
  const date = new Date(iso);
  return Number.isNaN(date.getTime()) ? "" : dateFormat.format(date);
}

/** `/a/b/c` → [{name:"a",path:"/a"}, {name:"b",path:"/a/b"}, …] */
export function splitPath(path: string): { name: string; path: string }[] {
  const parts = path.split("/").filter(Boolean);
  return parts.map((name, i) => ({ name, path: "/" + parts.slice(0, i + 1).join("/") }));
}

export function parentPath(path: string): string {
  const parts = path.split("/").filter(Boolean);
  parts.pop();
  return "/" + parts.join("/");
}
