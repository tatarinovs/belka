package io.github.tatarinovs.belka.core

import java.net.URLDecoder
import java.text.NumberFormat
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.util.Locale
import kotlin.math.abs

private val RU_LOCALE = Locale.forLanguageTag("ru-RU")
private val UNITS = arrayOf("Б", "КБ", "МБ", "ГБ", "ТБ")
private val DATE_FMT = DateTimeFormatter.ofPattern("dd.MM.yyyy, HH:mm").withZone(ZoneId.systemDefault())

fun formatBytes(bytes: Long): String {
    var value = bytes.coerceAtLeast(0).toDouble()
    var unit = 0
    while (value >= 1024.0 && unit < UNITS.lastIndex) {
        value /= 1024.0
        unit++
    }
    val digits = if (unit == 0 || value >= 100.0) 0 else 1
    val fmt = NumberFormat.getNumberInstance(RU_LOCALE).apply {
        maximumFractionDigits = digits
        isGroupingUsed = true
    }
    return "${fmt.format(value)} ${UNITS[unit]}"
}

fun formatDate(iso: String?): String {
    if (iso.isNullOrBlank()) return ""
    return runCatching { DATE_FMT.format(Instant.parse(iso)) }.getOrDefault("")
}

fun parentPath(path: String): String {
    val parts = path.split('/').filter { it.isNotEmpty() }
    if (parts.size <= 1) return "/"
    return "/" + parts.dropLast(1).joinToString("/")
}

fun baseName(path: String): String {
    if (path.startsWith("content://")) {
        val last = path.substringAfterLast('/')
        return runCatching {
            URLDecoder.decode(last, "UTF-8").split('/', ':').lastOrNull().orEmpty().ifEmpty { last }
        }.getOrDefault(last)
    }
    return path.split('/', '\\').lastOrNull { it.isNotEmpty() } ?: path
}

fun plural(n: Long, one: String, few: String, many: String): String {
    val absN = abs(n)
    val mod10 = absN % 10
    val mod100 = absN % 100
    val word = when {
        mod10 == 1L && mod100 != 11L -> one
        mod10 in 2L..4L && mod100 !in 12L..14L -> few
        else -> many
    }
    return "$n $word"
}

fun plural(n: Int, one: String, few: String, many: String): String =
    plural(n.toLong(), one, few, many)

fun describeEntries(items: List<Entry>): String =
    if (items.size == 1) "«${items[0].name}»" else "выбранные объекты (${items.size})"

fun markEncrypted(accounts: List<Account>, account: Account, entries: List<Entry>): List<Entry> {
    if (account.kind == ProviderKind.CRYPT) {
        return entries.map { it.copy(locked = "Хранится в облаке зашифрованным") }
    }
    val folders = accounts.mapNotNull { a ->
        val crypt = a.crypt ?: return@mapNotNull null
        if (crypt.base != account.id) return@mapNotNull null
        a to ("/" + crypt.root.trim('/'))
    }
    if (folders.isEmpty()) return entries
    return entries.map { e ->
        for ((folder, root) in folders) {
            if (e.path == root) {
                return@map e.copy(
                    locked = "Зашифрованная папка «${folder.name}»: откройте, чтобы увидеть файлы",
                    opensAccount = folder.id,
                )
            }
            if (root == "/" || e.path.startsWith("$root/")) {
                return@map e.copy(locked = "Зашифрованные данные папки «${folder.name}»")
            }
        }
        e
    }
}

private const val DETAIL_MARK = ". Подробности: "

data class ExplainedError(val summary: String, val detail: String? = null)

fun explainError(raw: String): ExplainedError {
    val text = raw.trim()
    val mark = text.indexOf(DETAIL_MARK)
    if (mark < 0) return ExplainedError(text)
    return ExplainedError(
        summary = text.substring(0, mark) + ".",
        detail = text.substring(mark + DETAIL_MARK.length),
    )
}

enum class PreviewKind {
    IMAGE, VIDEO, AUDIO, PDF, TEXT, PLAYER
}

private val IMAGE_EXTS = setOf("jpg", "jpeg", "png", "gif", "webp", "bmp", "svg", "ico", "avif", "heic", "heif")
private val VIDEO_EXTS = setOf(
    "mp4", "m4v", "webm", "mov", "ogv",
    "mkv", "avi", "m2ts", "mts", "flv", "mpg", "mpeg", "vob", "3gp",
)
private val AUDIO_EXTS = setOf("mp3", "m4a", "aac", "wav", "ogg", "oga", "opus", "flac")
private val PLAYER_EXTS = setOf("wmv")
private val TEXT_EXTS = setOf(
    "txt", "md", "log", "csv", "tsv", "json", "xml", "yaml", "yml", "toml", "ini", "cfg", "conf",
    "env", "sql", "html", "htm", "css", "js", "ts", "jsx", "tsx", "svelte", "vue", "py", "rs", "go",
    "java", "kt", "c", "h", "cpp", "hpp", "cs", "php", "rb", "sh", "bat", "cmd", "ps1", "srt",
    "gitignore", "dockerfile",
)

private const val TS_VIDEO_MIN = 1024L * 1024L
const val PREVIEW_CONFIRM_SIZE = 100L * 1024L * 1024L
const val TEXT_PREVIEW_MAX = 20L * 1024L * 1024L

fun previewKind(name: String, size: Long): PreviewKind? {
    val dot = name.lastIndexOf('.')
    val ext = (if (dot >= 0) name.substring(dot + 1) else name).lowercase()
    if (ext == "ts" && size >= TS_VIDEO_MIN) return PreviewKind.VIDEO
    return when (ext) {
        in IMAGE_EXTS -> PreviewKind.IMAGE
        in VIDEO_EXTS -> PreviewKind.VIDEO
        in AUDIO_EXTS -> PreviewKind.AUDIO
        "pdf" -> PreviewKind.PDF
        in PLAYER_EXTS -> PreviewKind.PLAYER
        in TEXT_EXTS -> PreviewKind.TEXT
        else -> null
    }
}
