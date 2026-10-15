import type { ProviderConfig, ProviderKind } from "./api";

export interface Field {
  key: string;
  label: string;
  type?: "text" | "password" | "url" | "number" | "checkbox";
  placeholder?: string;
  optional?: boolean;
  hint?: string;
}

/** A known service behind a generic provider: fills in what it can. */
export interface Preset {
  id: string;
  title: string;
  /** Where to get the address and the credentials. */
  help: string;
  /** Values filled in when the preset is chosen. */
  values?: Record<string, string>;
  /** Placeholders replacing the provider's ones, for values that vary per account. */
  placeholders?: Record<string, string>;
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
  /** Services offered for quick setup; the last one is a generic server. */
  presets?: Preset[];
}

const S3_PRESETS: Preset[] = [
  {
    id: "aws",
    title: "Amazon S3",
    help: "Endpoint региона бакета: https://s3.<регион>.amazonaws.com. Ключи — IAM → Users → Security credentials.",
    placeholders: { endpoint: "https://s3.eu-central-1.amazonaws.com" },
  },
  {
    id: "r2",
    title: "Cloudflare R2",
    help: "Account ID виден на странице R2 в панели Cloudflare, ключи — R2 → Manage API tokens.",
    placeholders: { endpoint: "https://<ACCOUNT_ID>.r2.cloudflarestorage.com" },
  },
  {
    id: "b2",
    title: "Backblaze B2",
    help: "Endpoint указан на странице бакета (Buckets → Endpoint). Ключ — Application Keys: keyID и applicationKey.",
    placeholders: { endpoint: "https://s3.us-west-004.backblazeb2.com" },
  },
  {
    id: "wasabi",
    title: "Wasabi",
    help: "Endpoint зависит от региона бакета: https://s3.<регион>.wasabisys.com. Ключи — Access Keys в консоли.",
    placeholders: { endpoint: "https://s3.eu-central-1.wasabisys.com" },
  },
  {
    id: "yandex",
    title: "Yandex Object Storage",
    help: "Статический ключ доступа сервисного аккаунта с ролью storage.editor.",
    values: { endpoint: "https://storage.yandexcloud.net" },
  },
  {
    id: "selectel",
    title: "Selectel",
    help: "Адрес пула ru-1; для другого пула замените ru-1 (например на ru-7). Ключи — S3-ключи сервисного пользователя.",
    values: { endpoint: "https://s3.ru-1.storage.selcloud.ru" },
  },
  {
    id: "vk",
    title: "VK Cloud",
    help: "Ключи создаются в разделе Объектное хранилище → Аккаунты.",
    values: { endpoint: "https://hb.ru-msk.vkcloud-storage.ru" },
  },
  {
    id: "timeweb",
    title: "Timeweb Cloud",
    help: "Ключи — на вкладке «Дашборд» бакета.",
    values: { endpoint: "https://s3.twcstorage.ru" },
  },
  {
    id: "storj",
    title: "Storj",
    help: "S3-ключи создаются в Access → Create S3 Credentials.",
    values: { endpoint: "https://gateway.storjshare.io" },
  },
  {
    id: "do",
    title: "DigitalOcean Spaces",
    help: "Endpoint региона: https://<регион>.digitaloceanspaces.com. Ключи — API → Spaces Keys.",
    placeholders: { endpoint: "https://fra1.digitaloceanspaces.com" },
  },
  {
    id: "minio",
    title: "MinIO",
    help: "Адрес API сервера MinIO (по умолчанию порт 9000), ключи — Access Keys в консоли.",
    placeholders: { endpoint: "http://192.168.1.10:9000" },
  },
  {
    id: "other",
    title: "Другой S3-совместимый",
    help: "Любое хранилище с S3 API и path-style адресацией.",
  },
];

const WEBDAV_PRESETS: Preset[] = [
  {
    id: "nextcloud",
    title: "Nextcloud",
    help: "Пароль приложения: Настройки → Безопасность → «Создать новый пароль приложения».",
    placeholders: { url: "https://cloud.example.com/remote.php/dav/files/<пользователь>/" },
  },
  {
    id: "owncloud",
    title: "ownCloud",
    help: "Адрес WebDAV показан в настройках файлов (значок шестерёнки внизу слева).",
    placeholders: { url: "https://cloud.example.com/remote.php/webdav/" },
  },
  {
    id: "koofr",
    title: "Koofr",
    help: "Пользователь — e-mail, пароль — пароль приложения: Preferences → Passwords.",
    values: { url: "https://app.koofr.net/dav/Koofr" },
  },
  {
    id: "pcloud",
    title: "pCloud",
    help: "Для аккаунтов в регионе США адрес https://webdav.pcloud.com, для европейских — https://ewebdav.pcloud.com.",
    values: { url: "https://ewebdav.pcloud.com" },
  },
  {
    id: "4shared",
    title: "4shared",
    help: "E-mail и пароль от 4shared.",
    values: { url: "https://webdav.4shared.com" },
  },
  {
    id: "infinicloud",
    title: "InfiniCLOUD",
    help: "Адрес и пароль приложения — My Page → Apps Connection.",
    placeholders: { url: "https://<сервер>.teracloud.jp/dav/" },
  },
  {
    id: "synology",
    title: "Synology (WebDAV Server)",
    help: "Нужен пакет WebDAV Server; без него подойдёт тип «Synology NAS». HTTPS — порт 5006.",
    placeholders: { url: "https://192.168.1.10:5006" },
  },
  {
    id: "other",
    title: "Другой WebDAV-сервер",
    help: "Nextcloud, ownCloud, Synology и любой сервер WebDAV.",
  },
];

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
    presets: WEBDAV_PRESETS,
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
    help: "AWS S3, Cloudflare R2, Backblaze B2, MinIO, Yandex Object Storage и другие.",
    presets: S3_PRESETS,
    fields: [
      { key: "endpoint", label: "Endpoint", type: "url", placeholder: "https://…r2.cloudflarestorage.com" },
      { key: "bucket", label: "Бакет" },
      { key: "access_key_id", label: "Access key ID" },
      { key: "secret_access_key", label: "Secret access key", type: "password" },
      {
        key: "region",
        label: "Регион",
        optional: true,
        placeholder: "по адресу endpoint, иначе us-east-1",
        hint: "Для известных сервисов определяется по адресу; укажите, если сервер требует другой.",
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

/** The fields of `def` with the placeholders of `preset` applied. */
export function presetFields(def: ProviderDef, preset: Preset | undefined): Field[] {
  if (!preset?.placeholders) return def.fields;
  return def.fields.map((f) =>
    preset.placeholders?.[f.key] ? { ...f, placeholder: preset.placeholders[f.key] } : f,
  );
}

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
