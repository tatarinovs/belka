import type { ProviderConfig, ProviderKind } from "./api";

export interface Field {
  key: string;
  label: string;
  type?: "text" | "password" | "url" | "number" | "checkbox";
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
  /** Items can be shared by a public link. */
  share?: boolean;
  /** Links can have a password and an expiry date. */
  shareOptions?: boolean;
}

export const PROVIDERS: ProviderDef[] = [
  {
    kind: "yandex",
    share: true,
    title: "Яндекс.Диск",
    short: "Я",
    help: "OAuth-токен с доступом к REST API Диска (oauth.yandex.ru).",
    fields: [{ key: "token", label: "OAuth-токен", type: "password", placeholder: "y0__…" }],
  },
  {
    kind: "mailru",
    share: true,
    title: "Облако Mail.ru",
    short: "M",
    help:
      "Пароль для внешних приложений: Настройки → Безопасность. Для ссылок нужен пароль с полным доступом к Почте, Облаку и Календарю.",
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
    share: true,
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
  {
    kind: "synology",
    share: true,
    shareOptions: true,
    title: "Synology NAS",
    short: "DS",
    help:
      "Файлы и ссылки через File Station, как в веб-интерфейсе DSM. Аккаунт DSM без двухфакторной аутентификации.",
    fields: [
      { key: "url", label: "Адрес NAS", placeholder: "192.168.1.10" },
      {
        key: "port",
        label: "Порт",
        type: "number",
        optional: true,
        placeholder: "5000, для HTTPS 5001",
        hint: "Как в адресе DSM в браузере: Панель управления → Портал входа.",
      },
      { key: "https", label: "HTTPS", type: "checkbox", optional: true },
      { key: "user", label: "Пользователь" },
      { key: "password", label: "Пароль", type: "password" },
      {
        key: "insecure",
        label: "Не проверять сертификат HTTPS (самоподписанный или на другое имя)",
        type: "checkbox",
        optional: true,
      },
    ],
  },
  {
    kind: "crypt",
    title: "Зашифрованная папка",
    short: "🔒",
    help: "Папка в подключённом облаке, файлы в которой шифруются на этом компьютере (формат rclone crypt).",
    fields: [],
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
      if (field.type === "number") {
        if (text) config[field.key] = Number(text);
      } else if (text || !field.optional) {
        config[field.key] = text;
      }
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
