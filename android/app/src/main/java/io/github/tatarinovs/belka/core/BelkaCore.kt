package io.github.tatarinovs.belka.core

import android.util.Base64
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.serialization.Serializable
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.decodeFromJsonElement
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive

class BelkaException(message: String) : Exception(message)

object BelkaCore {
    val json = Json {
        ignoreUnknownKeys = true
        encodeDefaults = true
        explicitNulls = false
    }

    private inline fun <reified T> decodeResponse(raw: String): T {
        val root = json.parseToJsonElement(raw).jsonObject
        root["err"]?.let { err ->
            throw BelkaException(err.jsonPrimitive.content)
        }
        val ok = root["ok"] ?: throw BelkaException("Empty response from core")
        return json.decodeFromJsonElement<T>(ok)
    }

    private suspend inline fun <reified T> invoke(method: String, argsJson: String = "{}"): T =
        withContext(Dispatchers.IO) {
            val raw = BelkaNative.call(method, argsJson)
            decodeResponse<T>(raw)
        }

    private suspend inline fun <reified A, reified T> invoke(method: String, args: A): T =
        invoke(method, json.encodeToString(args))

    // ---- accounts ----

    suspend fun listAccounts(): List<Account> = invoke("list_accounts")

    @Serializable
    private data class AddAccountArgs(val name: String, val config: JsonObject)

    suspend fun addAccount(name: String, config: JsonObject): Account =
        invoke("add_account", AddAccountArgs(name, config))

    @Serializable
    private data class ServiceArgs(val service: OAuthService)

    suspend fun oauthApps(service: OAuthService): OAuthApps =
        invoke("oauth_apps", ServiceArgs(service))

    @Serializable
    private data class OAuthSignInArgs(
        val service: OAuthService,
        val customApp: OAuthApp?,
        val name: String,
        val accountId: String?,
    )

    suspend fun oauthSignIn(
        service: OAuthService,
        customApp: OAuthApp?,
        name: String,
        accountId: String?,
        opener: UrlOpener,
    ): Account = withContext(Dispatchers.IO) {
        val raw = BelkaNative.oauthSignIn(
            json.encodeToString(OAuthSignInArgs(service, customApp, name, accountId)),
            opener,
        )
        decodeResponse<Account>(raw)
    }

    suspend fun oauthCancel() {
        invoke<JsonElement>("oauth_cancel")
    }

    @Serializable
    private data class AddCryptArgs(val name: String, val folder: CryptFolder)

    suspend fun addCryptFolder(name: String, folder: CryptFolder): Account =
        invoke("add_crypt_folder", AddCryptArgs(name, folder))

    @Serializable
    private data class IdArgs(val id: String)

    suspend fun accountConfig(id: String): JsonObject? =
        invoke("account_config", IdArgs(id))

    @Serializable
    private data class UpdateAccountArgs(val id: String, val name: String, val config: JsonObject?)

    suspend fun updateAccount(id: String, name: String, config: JsonObject?): Account =
        invoke("update_account", UpdateAccountArgs(id, name, config))

    @Serializable
    private data class ReorderArgs(val ids: List<String>)

    suspend fun reorderAccounts(ids: List<String>) {
        invoke<ReorderArgs, JsonElement>("reorder_accounts", ReorderArgs(ids))
    }

    suspend fun removeAccount(id: String) {
        invoke<IdArgs, JsonElement>("remove_account", IdArgs(id))
    }

    @Serializable
    private data class ExportArgs(val password: String)

    suspend fun exportAccounts(password: String): Pair<Int, ByteArray> {
        val res: ExportResult = invoke("export_accounts_bytes", ExportArgs(password))
        return res.count to Base64.decode(res.data, Base64.DEFAULT)
    }

    @Serializable
    private data class ImportArgs(val data: String, val password: String)

    suspend fun importAccounts(bytes: ByteArray, password: String): Imported =
        invoke("import_accounts_bytes", ImportArgs(Base64.encodeToString(bytes, Base64.NO_WRAP), password))

    // ---- cloud ----

    @Serializable
    private data class AccountPathArgs(val accountId: String, val path: String)

    suspend fun listDir(accountId: String, path: String): List<Entry> =
        invoke("list_dir", AccountPathArgs(accountId, path))

    suspend fun cloudQuota(accountId: String, path: String): Quota? =
        invoke("cloud_quota", AccountPathArgs(accountId, path))

    @Serializable
    private data class CloudMkdirArgs(val accountId: String, val parent: String, val name: String)

    suspend fun cloudMkdir(accountId: String, parent: String, name: String): String =
        invoke("cloud_mkdir", CloudMkdirArgs(accountId, parent, name))

    @Serializable
    private data class CloudRenameArgs(
        val accountId: String,
        val path: String,
        val newName: String,
        val isDir: Boolean,
    )

    suspend fun cloudRename(accountId: String, path: String, newName: String, isDir: Boolean): String =
        invoke("cloud_rename", CloudRenameArgs(accountId, path, newName, isDir))

    @Serializable
    private data class CloudItemArg(val path: String, val is_dir: Boolean)

    @Serializable
    private data class CloudItemsArgs(val accountId: String, val items: List<CloudItemArg>)

    suspend fun cloudDelete(accountId: String, items: List<Entry>) {
        invoke<CloudItemsArgs, JsonElement>(
            "cloud_delete",
            CloudItemsArgs(accountId, items.map { CloudItemArg(it.path, it.is_dir) }),
        )
    }

    @Serializable
    private data class CloudShareArgs(
        val accountId: String,
        val items: List<CloudItemArg>,
        val options: ShareOptions?,
    )

    suspend fun cloudShare(accountId: String, items: List<Entry>, options: ShareOptions? = null): List<String> =
        invoke(
            "cloud_share",
            CloudShareArgs(accountId, items.map { CloudItemArg(it.path, it.is_dir) }, options),
        )

    suspend fun cloudUnshare(accountId: String, items: List<Entry>) {
        invoke<CloudItemsArgs, JsonElement>(
            "cloud_unshare",
            CloudItemsArgs(accountId, items.map { CloudItemArg(it.path, it.is_dir) }),
        )
    }

    @Serializable
    private data class PreviewFileArgs(
        val accountId: String,
        val path: String,
        val size: Long,
        val modified: String?,
        val text: Boolean,
    )

    suspend fun previewFile(accountId: String, entry: Entry, text: Boolean): Preview =
        invoke("preview_file", PreviewFileArgs(accountId, entry.path, entry.size, entry.modified, text))

    suspend fun previewProgress(): PreviewProgress = invoke("preview_progress")

    suspend fun previewCancel() {
        invoke<JsonElement>("preview_cancel")
    }

    @Serializable
    private data class StreamArgs(val accountId: String, val path: String, val size: Long)

    suspend fun streamOpen(accountId: String, entry: Entry): String? =
        invoke("stream_open", StreamArgs(accountId, entry.path, entry.size))

    suspend fun streamOpenPlayer(accountId: String, entry: Entry): StreamPlayerInfo =
        invoke("stream_open_player", StreamArgs(accountId, entry.path, entry.size))

    // ---- local & transfers ----

    @Serializable
    private data class RemoteItemArg(
        val path: String,
        val is_dir: Boolean,
        val size: Long,
        val md5: String? = null,
        val modified: String? = null,
    )

    private fun Entry.toRemoteArg() = RemoteItemArg(path, is_dir, size, md5, modified)

    @Serializable
    private data class DownloadArgs(
        val accountId: String,
        val items: List<RemoteItemArg>,
        val localDir: String,
    )

    suspend fun download(accountId: String, items: List<Entry>, localDir: String): Int =
        invoke("download", DownloadArgs(accountId, items.map { it.toRemoteArg() }, localDir))

    @Serializable
    private data class UploadArgs(
        val accountId: String,
        val localPaths: List<String>,
        val remoteDir: String,
    )

    suspend fun upload(accountId: String, localPaths: List<String>, remoteDir: String): Int =
        invoke("upload", UploadArgs(accountId, localPaths, remoteDir))

    @Serializable
    private data class CloudCopyArgs(
        val accountId: String,
        val items: List<RemoteItemArg>,
        val targetAccountId: String,
        val targetDir: String,
    )

    suspend fun cloudCopy(
        accountId: String,
        items: List<Entry>,
        targetAccountId: String,
        targetDir: String,
    ): Int = invoke(
        "cloud_copy",
        CloudCopyArgs(accountId, items.map { it.toRemoteArg() }, targetAccountId, targetDir),
    )

    @Serializable
    private data class ConflictsArgs(val dir: String, val names: List<String>)

    suspend fun localConflicts(dir: String, names: List<String>): List<String> =
        invoke("local_conflicts", ConflictsArgs(dir, names))

    @Serializable
    private data class PathArgs(val path: String)

    suspend fun listLocal(path: String): Listing = invoke("list_local", PathArgs(path))

    @Serializable
    private data class LocalMkdirArgs(val parent: String, val name: String)

    suspend fun localMkdir(parent: String, name: String): String =
        invoke("local_mkdir", LocalMkdirArgs(parent, name))

    @Serializable
    private data class LocalRenameArgs(val path: String, val newName: String)

    suspend fun localRename(path: String, newName: String): String =
        invoke("local_rename", LocalRenameArgs(path, newName))

    @Serializable
    private data class PathsArgs(val paths: List<String>)

    suspend fun localDelete(paths: List<String>) {
        invoke<PathsArgs, JsonElement>("local_delete", PathsArgs(paths))
    }

    @Serializable
    private data class ListTransfersArgs(val filter: TransferFilter)

    suspend fun listTransfers(filter: TransferFilter = TransferFilter.ALL): TransferListResponse =
        invoke("list_transfers", ListTransfersArgs(filter))

    @Serializable
    private data class TransferIdArgs(val id: Long)

    suspend fun cancelTransfer(id: Long) {
        invoke<TransferIdArgs, JsonElement>("cancel_transfer", TransferIdArgs(id))
    }

    suspend fun cancelAllTransfers() {
        invoke<JsonElement>("cancel_all_transfers")
    }

    suspend fun retryTransfer(id: Long) {
        invoke<TransferIdArgs, JsonElement>("retry_transfer", TransferIdArgs(id))
    }

    suspend fun retryFailedTransfers(): RetryFailures = invoke("retry_failed_transfers")

    suspend fun clearTransfers() {
        invoke<JsonElement>("clear_transfers")
    }

    fun transferActivityBlocking(): TransferActivity {
        val raw = BelkaNative.call("transfer_activity", "{}")
        return decodeResponse(raw)
    }

    // ---- backups & updates ----

    suspend fun backupTasks(): List<BackupTask> = invoke("backup_tasks")

    suspend fun backupList(): List<BackupTask> = invoke("backup_list")

    @Serializable
    private data class BackupSaveArgs(val task: BackupTask)

    suspend fun backupSave(task: BackupTask): BackupTask =
        invoke("backup_save", BackupSaveArgs(task))

    suspend fun backupRemove(id: String) {
        invoke<IdArgs, JsonElement>("backup_remove", IdArgs(id))
    }

    suspend fun backupCancel(id: String) {
        invoke<IdArgs, JsonElement>("backup_cancel", IdArgs(id))
    }

    suspend fun checkUpdate(): UpdateInfo? = invoke("check_update")
}
