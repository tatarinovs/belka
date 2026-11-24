package io.github.tatarinovs.belka.core

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable

@Serializable
enum class ProviderKind {
    @SerialName("yandex") YANDEX,
    @SerialName("mailru") MAILRU,
    @SerialName("google") GOOGLE,
    @SerialName("webdav") WEBDAV,
    @SerialName("s3") S3,
    @SerialName("synology") SYNOLOGY,
    @SerialName("crypt") CRYPT,
}

@Serializable
data class CryptLocation(
    val base: String,
    val root: String,
)

@Serializable
data class Account(
    val id: String,
    val name: String,
    val kind: ProviderKind,
    val crypt: CryptLocation? = null,
)

@Serializable
data class Entry(
    val name: String,
    val path: String,
    val is_dir: Boolean,
    val size: Long = 0,
    val modified: String? = null,
    val md5: String? = null,
    val icon: String? = null,
    val locked: String? = null,
    val opensAccount: String? = null,
)

@Serializable
data class Quota(
    val total: Long? = null,
    val used: Long? = null,
    val free: Long? = null,
)

@Serializable
data class Crumb(
    val name: String,
    val path: String,
)

@Serializable
data class Listing(
    val path: String,
    val parent: String? = null,
    val crumbs: List<Crumb> = emptyList(),
    val entries: List<Entry> = emptyList(),
)

@Serializable
data class ShareOptions(
    val password: String = "",
    val expires: String = "",
)

@Serializable
enum class TransferState {
    @SerialName("queued") QUEUED,
    @SerialName("running") RUNNING,
    @SerialName("done") DONE,
    @SerialName("failed") FAILED,
    @SerialName("cancelled") CANCELLED,
}

@Serializable
enum class TransferFilter(val label: String) {
    @SerialName("all") ALL("Все"),
    @SerialName("active") ACTIVE("Активные"),
    @SerialName("failed") FAILED("Ошибки"),
    @SerialName("done") DONE("Готово"),
    @SerialName("cancelled") CANCELLED("Отменённые"),
}

@Serializable
enum class TransferDirection {
    @SerialName("upload") UPLOAD,
    @SerialName("download") DOWNLOAD,
    @SerialName("copy") COPY,
}

@Serializable
data class TransferCounts(
    val queued: Int = 0,
    val running: Int = 0,
    val done: Int = 0,
    val failed: Int = 0,
    val cancelled: Int = 0,
) {
    val activeCount: Int get() = queued + running
    val total: Int get() = queued + running + done + failed + cancelled

    fun forFilter(filter: TransferFilter): Int = when (filter) {
        TransferFilter.ALL -> total
        TransferFilter.ACTIVE -> activeCount
        TransferFilter.FAILED -> failed
        TransferFilter.DONE -> done
        TransferFilter.CANCELLED -> cancelled
    }
}

@Serializable
data class Transfer(
    val id: Long,
    val direction: TransferDirection,
    val account_id: String,
    val account_name: String,
    val name: String,
    val local_path: String,
    val remote_path: String,
    val remote_dir: String,
    val state: TransferState,
    val error: String? = null,
    val unchanged: Boolean = false,
    val done: Long = 0,
    val total: Long = 0,
    val attempt: Int = 1,
    val max_attempts: Int = 1,
)

@Serializable
data class TransferListResponse(
    val items: List<Transfer> = emptyList(),
    val counts: TransferCounts = TransferCounts(),
    val view_items: List<Transfer>? = null,
)

@Serializable
data class RetryFailures(
    val retried: Int = 0,
    val errors: List<String> = emptyList(),
)

@Serializable
data class TransferActivity(
    val active: Int = 0,
    val uploads: Int = 0,
    val name: String = "",
    val doneBytes: Long = 0,
    val totalBytes: Long = 0,
    val player: String = "",
)

@Serializable
data class CryptFolder(
    val base: String,
    val root: String,
    val password: String,
    val salt: String = "",
    val encrypt_names: Boolean = true,
)

@Serializable
data class Preview(
    val path: String,
    val text: String? = null,
    val truncated: Boolean = false,
)

@Serializable
data class PreviewProgress(
    val done: Long = 0,
    val total: Long = 0,
    val attempt: Int = 1,
    val max_attempts: Int = 1,
)

@Serializable
data class StreamPlayerInfo(
    val url: String,
    val mime: String,
    val name: String,
)

@Serializable
enum class OAuthService {
    @SerialName("google") GOOGLE,
    @SerialName("yandex") YANDEX,
}

@Serializable
data class OAuthApp(
    val client_id: String = "",
    val client_secret: String = "",
)

@Serializable
data class OAuthApps(
    val builtin: Boolean = false,
    val custom: OAuthApp? = null,
)

@Serializable
data class Imported(
    val added: Int = 0,
    val skipped: Int = 0,
)

@Serializable
data class ExportResult(
    val count: Int = 0,
    val data: String = "",
)

@Serializable
data class BackupRun(
    val started: String? = null,
    val finished: String? = null,
    val uploaded: Long = 0,
    val bytes: Long = 0,
    val unchanged: Long = 0,
    val failed: Long = 0,
    val failures: List<List<String>> = emptyList(),
    val freed: Long = 0,
    val freed_bytes: Long = 0,
    val error: String? = null,
    val cancelled: Boolean = false,
)

@Serializable
data class BackupProgress(
    val scanning: Boolean = false,
    val files_total: Long = 0,
    val files_done: Long = 0,
    val bytes_total: Long = 0,
    val bytes_done: Long = 0,
    val current: String = "",
)

@Serializable
data class BackupTask(
    val id: String = "",
    val name: String = "",
    val local: String = "",
    val account_id: String = "",
    val remote: String = "",
    val interval_hours: Int = 24,
    val wifi_only: Boolean = true,
    val charging_only: Boolean = false,
    val enabled: Boolean = true,
    val include: List<String> = emptyList(),
    val exclude: List<String> = emptyList(),
    val free_after_days: Int? = null,
    val history: List<BackupRun> = emptyList(),
    val running: BackupProgress? = null,
)

fun newBackupTask(local: String, accountId: String, remote: String = ""): BackupTask =
    BackupTask(
        local = local,
        account_id = accountId,
        remote = remote,
    )

@Serializable
data class UpdateInfo(
    val version: String,
    val page: String,
    val apk: String? = null,
)
