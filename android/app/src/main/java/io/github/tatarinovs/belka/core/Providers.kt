package io.github.tatarinovs.belka.core

import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject

enum class FieldType {
    TEXT, PASSWORD, URL, NUMBER, CHECKBOX
}

data class ProviderField(
    val key: String,
    val label: String,
    val type: FieldType = FieldType.TEXT,
    val placeholder: String = "",
    val optional: Boolean = false,
    val hint: String = "",
)

data class ProviderPreset(
    val id: String,
    val title: String,
    val help: String,
    val values: Map<String, String> = emptyMap(),
    val placeholders: Map<String, String> = emptyMap(),
)

data class ProviderDef(
    val kind: ProviderKind,
    val kindWire: String,
    val title: String,
    val short: String,
    val help: String,
    val fields: List<ProviderField>,
    val share: Boolean = false,
    val shareOptions: Boolean = false,
    val presets: List<ProviderPreset> = emptyList(),
)

private val S3_PRESETS = listOf(
    ProviderPreset(
        id = "aws",
        title = "Amazon S3",
        help = "Endpoint региона бакета: https://s3.<регион>.amazonaws.com. Ключи — IAM → Users → Security credentials.",
        placeholders = mapOf("endpoint" to "https://s3.eu-central-1.amazonaws.com"),
    ),
    ProviderPreset(
        id = "r2",
        title = "Cloudflare R2",
        help = "Account ID виден на странице R2 в панели Cloudflare, ключи — R2 → Manage API tokens.",
        placeholders = mapOf("endpoint" to "https://<ACCOUNT_ID>.r2.cloudflarestorage.com"),
    ),
    ProviderPreset(
        id = "b2",
        title = "Backblaze B2",
        help = "Endpoint указан на странице бакета (Buckets → Endpoint). Ключ — Application Keys: keyID и applicationKey.",
        placeholders = mapOf("endpoint" to "https://s3.us-west-004.backblazeb2.com"),
    ),
    ProviderPreset(
        id = "wasabi",
        title = "Wasabi",
        help = "Endpoint зависит от региона бакета: https://s3.<регион>.wasabisys.com. Ключи — Access Keys в консоли.",
        placeholders = mapOf("endpoint" to "https://s3.eu-central-1.wasabisys.com"),
    ),
    ProviderPreset(
        id = "yandex",
        title = "Yandex Object Storage",
        help = "Статический ключ доступа сервисного аккаунта с ролью storage.editor.",
        values = mapOf("endpoint" to "https://storage.yandexcloud.net"),
    ),
    ProviderPreset(
        id = "selectel",
        title = "Selectel",
        help = "Адрес пула ru-1; для другого пула замените ru-1 (например на ru-7). Ключи — S3-ключи сервисного пользователя.",
        values = mapOf("endpoint" to "https://s3.ru-1.storage.selcloud.ru"),
    ),
    ProviderPreset(
        id = "vk",
        title = "VK Cloud",
        help = "Ключи создаются в разделе Объектное хранилище → Аккаунты.",
        values = mapOf("endpoint" to "https://hb.ru-msk.vkcloud-storage.ru"),
    ),
    ProviderPreset(
        id = "timeweb",
        title = "Timeweb Cloud",
        help = "Ключи — на вкладке «Дашборд» бакета.",
        values = mapOf("endpoint" to "https://s3.twcstorage.ru"),
    ),
    ProviderPreset(
        id = "storj",
        title = "Storj",
        help = "S3-ключи создаются в Access → Create S3 Credentials.",
        values = mapOf("endpoint" to "https://gateway.storjshare.io"),
    ),
    ProviderPreset(
        id = "do",
        title = "DigitalOcean Spaces",
        help = "Endpoint региона: https://<регион>.digitaloceanspaces.com. Ключи — API → Spaces Keys.",
        placeholders = mapOf("endpoint" to "https://fra1.digitaloceanspaces.com"),
    ),
    ProviderPreset(
        id = "minio",
        title = "MinIO",
        help = "Адрес API сервера MinIO (по умолчанию порт 9000), ключи — Access Keys в консоли.",
        placeholders = mapOf("endpoint" to "http://192.168.1.10:9000"),
    ),
    ProviderPreset(
        id = "other",
        title = "Другой S3-совместимый",
        help = "Любое хранилище с S3 API и path-style адресацией.",
    ),
)

private val WEBDAV_PRESETS = listOf(
    ProviderPreset(
        id = "nextcloud",
        title = "Nextcloud",
        help = "Пароль приложения: Настройки → Безопасность → «Создать новый пароль приложения».",
        placeholders = mapOf("url" to "https://cloud.example.com/remote.php/dav/files/<пользователь>/"),
    ),
    ProviderPreset(
        id = "owncloud",
        title = "ownCloud",
        help = "Адрес WebDAV показан в настройках файлов (значок шестерёнки внизу слева).",
        placeholders = mapOf("url" to "https://cloud.example.com/remote.php/webdav/"),
    ),
    ProviderPreset(
        id = "koofr",
        title = "Koofr",
        help = "Пользователь — e-mail, пароль — пароль приложения: Preferences → Passwords.",
        values = mapOf("url" to "https://app.koofr.net/dav/Koofr"),
    ),
    ProviderPreset(
        id = "pcloud",
        title = "pCloud",
        help = "Для аккаунтов в регионе США адрес https://webdav.pcloud.com, для европейских — https://ewebdav.pcloud.com.",
        values = mapOf("url" to "https://ewebdav.pcloud.com"),
    ),
    ProviderPreset(
        id = "4shared",
        title = "4shared",
        help = "E-mail и пароль от 4shared.",
        values = mapOf("url" to "https://webdav.4shared.com"),
    ),
    ProviderPreset(
        id = "infinicloud",
        title = "InfiniCLOUD",
        help = "Адрес и пароль приложения — My Page → Apps Connection.",
        placeholders = mapOf("url" to "https://<сервер>.teracloud.jp/dav/"),
    ),
    ProviderPreset(
        id = "synology",
        title = "Synology (WebDAV Server)",
        help = "Нужен пакет WebDAV Server; без него подойдёт тип «Synology NAS». HTTPS — порт 5006.",
        placeholders = mapOf("url" to "https://192.168.1.10:5006"),
    ),
    ProviderPreset(
        id = "other",
        title = "Другой WebDAV-сервер",
        help = "Nextcloud, ownCloud, Synology и любой сервер WebDAV.",
    ),
)

val PROVIDERS: List<ProviderDef> = listOf(
    ProviderDef(
        kind = ProviderKind.YANDEX,
        kindWire = "yandex",
        share = true,
        title = "Яндекс.Диск",
        short = "Я",
        help = "OAuth-токен с доступом к REST API Диска (oauth.yandex.ru).",
        fields = listOf(
            ProviderField("token", "OAuth-токен", FieldType.PASSWORD, placeholder = "y0__…"),
        ),
    ),
    ProviderDef(
        kind = ProviderKind.MAILRU,
        kindWire = "mailru",
        share = true,
        title = "Облако Mail.ru",
        short = "M",
        help = "Пароль для внешних приложений: Настройки → Безопасность. Для ссылок нужен пароль с полным доступом к Почте, Облаку и Календарю.",
        fields = listOf(
            ProviderField("user", "E-mail", placeholder = "name@mail.ru"),
            ProviderField("password", "Пароль приложения", FieldType.PASSWORD),
            ProviderField(
                "url",
                "Адрес WebDAV",
                FieldType.URL,
                optional = true,
                placeholder = "https://webdav.mail.ru",
            ),
        ),
    ),
    ProviderDef(
        kind = ProviderKind.GOOGLE,
        kindWire = "google",
        share = true,
        title = "Google Drive",
        short = "G",
        help = "OAuth-клиент и refresh token, например из rclone или OAuth Playground.",
        fields = listOf(
            ProviderField("client_id", "Client ID", placeholder = "….apps.googleusercontent.com"),
            ProviderField("client_secret", "Client secret", FieldType.PASSWORD),
            ProviderField("refresh_token", "Refresh token", FieldType.PASSWORD, placeholder = "1//…"),
        ),
    ),
    ProviderDef(
        kind = ProviderKind.WEBDAV,
        kindWire = "webdav",
        title = "WebDAV",
        short = "W",
        help = "Nextcloud, ownCloud, Synology и любой сервер WebDAV.",
        presets = WEBDAV_PRESETS,
        fields = listOf(
            ProviderField(
                "url",
                "Адрес",
                FieldType.URL,
                placeholder = "https://cloud.example.com/remote.php/dav/files/user/",
            ),
            ProviderField("user", "Пользователь"),
            ProviderField("password", "Пароль", FieldType.PASSWORD),
            ProviderField("etag_is_md5", "ETag сервера — это MD5 файла", FieldType.CHECKBOX, optional = true),
        ),
    ),
    ProviderDef(
        kind = ProviderKind.S3,
        kindWire = "s3",
        title = "S3-совместимое",
        short = "S3",
        help = "AWS S3, Cloudflare R2, Backblaze B2, MinIO, Yandex Object Storage и другие.",
        presets = S3_PRESETS,
        fields = listOf(
            ProviderField("endpoint", "Endpoint", FieldType.URL, placeholder = "https://…r2.cloudflarestorage.com"),
            ProviderField("bucket", "Бакет"),
            ProviderField("access_key_id", "Access key ID"),
            ProviderField("secret_access_key", "Secret access key", FieldType.PASSWORD),
            ProviderField(
                "region",
                "Регион",
                optional = true,
                placeholder = "по адресу endpoint, иначе us-east-1",
                hint = "Для известных сервисов определяется по адресу; укажите, если сервер требует другой.",
            ),
        ),
    ),
    ProviderDef(
        kind = ProviderKind.SYNOLOGY,
        kindWire = "synology",
        share = true,
        shareOptions = true,
        title = "Synology NAS",
        short = "DS",
        help = "Файлы и ссылки через File Station, как в веб-интерфейсе DSM. Аккаунт DSM без двухфакторной аутентификации.",
        fields = listOf(
            ProviderField("url", "Адрес NAS", placeholder = "192.168.1.10"),
            ProviderField(
                "port",
                "Порт",
                FieldType.NUMBER,
                optional = true,
                placeholder = "5000, для HTTPS 5001",
                hint = "Как в адресе DSM в браузере: Панель управления → Портал входа.",
            ),
            ProviderField("https", "HTTPS", FieldType.CHECKBOX, optional = true),
            ProviderField("user", "Пользователь"),
            ProviderField("password", "Пароль", FieldType.PASSWORD),
            ProviderField(
                "insecure",
                "Не проверять сертификат HTTPS (самоподписанный или на другое имя)",
                FieldType.CHECKBOX,
                optional = true,
            ),
        ),
    ),
    ProviderDef(
        kind = ProviderKind.CRYPT,
        kindWire = "crypt",
        title = "Зашифрованная папка",
        short = "🔒",
        help = "Папка в подключённом облаке, файлы в которой шифруются на телефоне (формат rclone crypt).",
        fields = emptyList(),
    ),
)

fun providerDef(kind: ProviderKind): ProviderDef =
    PROVIDERS.firstOrNull { it.kind == kind } ?: PROVIDERS.first()

fun presetFields(def: ProviderDef, preset: ProviderPreset?): List<ProviderField> {
    if (preset == null || preset.placeholders.isEmpty()) return def.fields
    return def.fields.map { f ->
        preset.placeholders[f.key]?.let { f.copy(placeholder = it) } ?: f
    }
}

fun buildProviderConfig(
    def: ProviderDef,
    strings: Map<String, String>,
    bools: Map<String, Boolean>,
): JsonObject = buildJsonObject {
    put("kind", JsonPrimitive(def.kindWire))
    for (field in def.fields) {
        if (field.type == FieldType.CHECKBOX) {
            put(field.key, JsonPrimitive(bools[field.key] == true))
        } else {
            val text = strings[field.key]?.trim().orEmpty()
            if (field.type == FieldType.NUMBER) {
                text.toIntOrNull()?.let { put(field.key, JsonPrimitive(it)) }
            } else if (text.isNotEmpty() || !field.optional) {
                put(field.key, JsonPrimitive(text))
            }
        }
    }
}

fun missingProviderFields(def: ProviderDef, strings: Map<String, String>): List<String> =
    def.fields
        .filter { !it.optional && it.type != FieldType.CHECKBOX }
        .filter { strings[it.key]?.trim().isNullOrEmpty() }
        .map { it.label }
