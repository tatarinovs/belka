import type { ProviderConfig, ProviderKind } from "./api";

export interface Field {
  key: string;
  label: string;
  type?: "text" | "password" | "url" | "checkbox";
  placeholder?: string;
  optional?: boolean;
  hint?: string;
}

export interface ProviderDef {
  kind: ProviderKind;
  title: string;
  short: string;
  help: string;
  fields: Field[];
}

export const PROVIDERS: ProviderDef[] = [
  {
    kind: "yandex",
    title: "Яндекс.Диск",
    short: "Я",
    help: "OAuth-токен с доступом к REST API Диска (oauth.yandex.ru).",
    fields: [{ key: "token", label: "OAuth-токен", type: "password", placeholder: "y0__…" }],
  },
  {
    kind: "mailru",
    title: "Облако Mail.ru",
    short: "M",
    help: "Пароль для внешних приложений: Настройки → Безопасность.",
    fields: [
      { key: "user", label: "E-mail", placeholder: "name@mail.ru" },
      { key: "password", label: "Пароль приложения", type: "password" },
      {
        key: "url",
        label: "Адрес WebDAV",
        type: "url",
        optional: true,
        placeholder: "https://webdav.mail.ru",
      },
    ],
  },
  {
    kind: "google",
    title: "Google Drive",
    short: "G",
    help: "OAuth-клиент и refresh token, например из rclone или OAuth Playground.",
    fields: [
      { key: "client_id", label: "Client ID", placeholder: "….apps.googleusercontent.com" },
      { key: "client_secret", label: "Client secret", type: "password" },
      { key: "refresh_token", label: "Refresh token", type: "password", placeholder: "1//…" },
    ],
  },
  {
    kind: "webdav",
    title: "WebDAV",
    short: "W",
    help: "Nextcloud, ownCloud, Synology и любой сервер WebDAV.",
    fields: [
      {
        key: "url",
        label: "Адрес",
        type: "url",
        placeholder: "https://cloud.example.com/remote.php/dav/files/user/",
      },
      { key: "user", label: "Пользователь" },
      { key: "password", label: "Пароль", type: "password" },
      {
        key: "etag_is_md5",
        label: "ETag сервера — это MD5 файла",
        type: "checkbox",
        optional: true,
      },
    ],
  },
  {
    kind: "s3",
    title: "S3-совместимое",
    short: "S3",
    help: "AWS S3, Cloudflare R2, MinIO, Yandex Object Storage.",
    fields: [
      { key: "endpoint", label: "Endpoint", type: "url", placeholder: "https://…r2.cloudflarestorage.com" },
      { key: "bucket", label: "Бакет" },
      { key: "access_key_id", label: "Access key ID" },
      { key: "secret_access_key", label: "Secret access key", type: "password" },
      {
        key: "region",
        label: "Регион",
        optional: true,
        placeholder: "us-east-1 (R2: auto)",
      },
    ],
  },
];

export function providerDef(kind: ProviderKind): ProviderDef {
  return PROVIDERS.find((p) => p.kind === kind) ?? PROVIDERS[0];
}

/** Builds the config sent to Rust; empty optional fields are omitted. */
export function buildConfig(
  def: ProviderDef,
  values: Record<string, string | boolean>,
): ProviderConfig {
  const config: ProviderConfig = { kind: def.kind };
  for (const field of def.fields) {
    const value = values[field.key];
    if (field.type === "checkbox") {
      config[field.key] = value === true;
    } else {
      const text = typeof value === "string" ? value.trim() : "";
      if (text || !field.optional) config[field.key] = text;
    }
  }
  return config;
}

export function missingFields(
  def: ProviderDef,
  values: Record<string, string | boolean>,
): string[] {
  return def.fields
    .filter((f) => !f.optional && f.type !== "checkbox")
    .filter((f) => !(typeof values[f.key] === "string" && (values[f.key] as string).trim()))
    .map((f) => f.label);
}
