package io.github.tatarinovs.belka.ui

import android.app.Application
import android.os.SystemClock
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import io.github.tatarinovs.belka.BackupWorker
import io.github.tatarinovs.belka.BuildConfig
import io.github.tatarinovs.belka.core.Account
import io.github.tatarinovs.belka.core.AndroidPlatform
import io.github.tatarinovs.belka.core.BackupTask
import io.github.tatarinovs.belka.core.BelkaCore
import io.github.tatarinovs.belka.core.Crumb
import io.github.tatarinovs.belka.core.Entry
import io.github.tatarinovs.belka.core.Listing
import io.github.tatarinovs.belka.core.PaneMode
import io.github.tatarinovs.belka.core.Prefs
import io.github.tatarinovs.belka.core.Quota
import io.github.tatarinovs.belka.core.ShareOptions
import io.github.tatarinovs.belka.core.SortKey
import io.github.tatarinovs.belka.core.Transfer
import io.github.tatarinovs.belka.core.TransferCounts
import io.github.tatarinovs.belka.core.TransferDirection
import io.github.tatarinovs.belka.core.TransferFilter
import io.github.tatarinovs.belka.core.TransferState
import io.github.tatarinovs.belka.core.UpdateInfo
import io.github.tatarinovs.belka.core.baseName
import io.github.tatarinovs.belka.core.describeEntries
import io.github.tatarinovs.belka.core.markEncrypted
import io.github.tatarinovs.belka.core.newBackupTask
import io.github.tatarinovs.belka.core.parentPath
import io.github.tatarinovs.belka.core.plural
import io.github.tatarinovs.belka.core.providerDef
import io.github.tatarinovs.belka.ui.icons.BelkaIcon
import java.text.Collator
import java.time.Instant
import java.util.Locale
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

sealed interface PickMode {
    data class Download(val account: Account, val items: List<Entry>) : PickMode
    data class Upload(val paths: List<String>) : PickMode
    data class Copy(val account: Account, val items: List<Entry>) : PickMode
    data object BackupLocal : PickMode
    data object BackupRemote : PickMode
}

data class SheetMenuItem(
    val label: String,
    val icon: BelkaIcon? = null,
    val badge: Account? = null,
    val checked: Boolean = false,
    val danger: Boolean = false,
    val disabled: Boolean = false,
    val isSeparator: Boolean = false,
    val action: () -> Unit = {},
)

data class SheetMenu(
    val title: String? = null,
    val items: List<SheetMenuItem>,
)

data class PromptState(
    val title: String,
    val initialValue: String,
    val okLabel: String,
    val selectFileStem: Boolean = true,
    val onResult: (String?) -> Unit,
)

data class ConfirmState(
    val title: String,
    val message: String,
    val okLabel: String,
    val cancelLabel: String = "Отмена",
    val onConfirm: () -> Unit,
)

data class ToastMessage(
    val text: String,
    val isError: Boolean = false,
)

data class PreviewSession(
    val account: Account,
    val files: List<Entry>,
    val start: Int,
)

class BelkaViewModel(application: Application) : AndroidViewModel(application) {
    private val appCtx get() = getApplication<Application>()
    private val prefs = Prefs(application)
    private val ruCollator = Collator.getInstance(Locale.forLanguageTag("ru-RU")).apply {
        strength = Collator.PRIMARY
    }

    var accounts by mutableStateOf<List<Account>>(emptyList())
        private set
    var accountsLoaded by mutableStateOf(false)
        private set

    var mode by mutableStateOf(prefs.mode)
        private set
    var accountId by mutableStateOf(prefs.accountId)
        private set
    private val cloudPaths = prefs.getCloudPaths()
    private var localPath = prefs.localPath
    var sortKey by mutableStateOf(prefs.sortKey)
        private set

    var listing by mutableStateOf<Listing?>(null)
        private set
    var loading by mutableStateOf(false)
        private set
    var refreshing by mutableStateOf(false)
        private set
    var error by mutableStateOf("")
        private set
    var selectedPaths by mutableStateOf<Set<String>>(emptySet())
        private set
    var pick by mutableStateOf<PickMode?>(null)
        private set
    var busy by mutableStateOf("")
        private set

    var sheet by mutableStateOf<SheetMenu?>(null)
        private set
    var prompt by mutableStateOf<PromptState?>(null)
        private set
    var confirm by mutableStateOf<ConfirmState?>(null)
        private set

    var addingAccount by mutableStateOf(false)
        private set
    var editingAccount by mutableStateOf<Account?>(null)
        private set
    var managingAccounts by mutableStateOf(false)
        private set

    var previewing by mutableStateOf<PreviewSession?>(null)
        private set
    var sharing by mutableStateOf<List<Entry>?>(null)
        private set

    var toast by mutableStateOf<ToastMessage?>(null)
        private set
    var storageOk by mutableStateOf(AndroidPlatform.hasStorageAccess(application))
        private set

    var backupsOpen by mutableStateOf(false)
        private set
    var backupDraft by mutableStateOf<BackupTask?>(null)
    var backupTasks by mutableStateOf<List<BackupTask>>(emptyList())
        private set
    var backupsLoaded by mutableStateOf(false)
        private set

    var updateInfo by mutableStateOf<UpdateInfo?>(null)
        private set
    var quota by mutableStateOf<Quota?>(null)
        private set
    private var quotaVersion by mutableIntStateOf(0)
    private var quotaKey = ""
    private var quotaSeq = 0

    // ---- transfers ----
    var transferItems by mutableStateOf<List<Transfer>>(emptyList())
        private set
    var transferViewItems by mutableStateOf<List<Transfer>>(emptyList())
        private set
    var transferFilter by mutableStateOf(TransferFilter.ALL)
        private set
    var transferCounts by mutableStateOf(TransferCounts())
        private set
    var transferSpeeds by mutableStateOf<Map<Long, Double>>(emptyMap())
        private set
    var transferWaiting by mutableStateOf<Set<Long>>(emptySet())
        private set
    var transferError by mutableStateOf("")
        private set
    var transferCollapsed by mutableStateOf(false)

    private var transferPollJob: Job? = null
    private var transferReqSeq = 0
    private val lastBytes = mutableMapOf<Long, Pair<Long, Long>>()
    private val prevStates = mutableMapOf<Long, TransferState>()
    private val runningSince = mutableMapOf<Long, Long>()

    private var requestSeq = 0
    private var toastJob: Job? = null
    private var reloadJob: Job? = null
    private var quotaJob: Job? = null
    private var exitRequestedAt by mutableLongStateOf(0L)

    val account: Account?
        get() = accounts.firstOrNull { it.id == accountId }

    val currentPath: String
        get() = listing?.path.orEmpty()

    val atRoot: Boolean
        get() = listing?.parent == null

    val title: String
        get() = if (mode == PaneMode.CLOUD) {
            if (atRoot) (account?.name ?: "Облако") else baseName(currentPath)
        } else {
            when {
                currentPath.isEmpty() -> "Телефон"
                currentPath.trimEnd('/') == ANDROID_STORAGE -> "Внутренняя память"
                else -> baseName(currentPath)
            }
        }

    val subtitle: String
        get() = if (mode == PaneMode.CLOUD) {
            val acc = account
            if (acc != null && !atRoot) "${acc.name}: $currentPath" else ""
        } else {
            currentPath
        }

    val sortedEntries: List<Entry>
        get() {
            val raw = listing?.entries.orEmpty()
            if (raw.isEmpty()) return raw
            val times = if (sortKey == SortKey.MODIFIED) {
                raw.associateWith { e -> e.modified?.let { parseIsoMillis(it) } ?: 0L }
            } else {
                emptyMap()
            }
            return raw.sortedWith { a, b ->
                if (a.icon != null || b.icon != null) return@sortedWith 0
                if (a.is_dir != b.is_dir) return@sortedWith if (a.is_dir) -1 else 1
                val cmp = when (sortKey) {
                    SortKey.MODIFIED -> (times[b] ?: 0L).compareTo(times[a] ?: 0L)
                    SortKey.SIZE -> b.size.compareTo(a.size)
                    SortKey.NAME -> 0
                }
                if (cmp != 0) cmp else compareNatural(a.name, b.name, ruCollator)
            }
        }

    val selectedEntries: List<Entry>
        get() = sortedEntries.filter { selectedPaths.contains(it.path) }

    val selecting: Boolean
        get() = selectedPaths.isNotEmpty()

    val canModify: Boolean
        get() = if (mode == PaneMode.CLOUD) account != null else currentPath.isNotEmpty()

    init {
        kickTransfers()
        val lastUpdateCheck = prefs.updateCheckedAt
        if (System.currentTimeMillis() - lastUpdateCheck > UPDATE_CHECK_MS) {
            checkUpdate(manual = false)
        }
        viewModelScope.launch {
            try {
                val list = BelkaCore.listAccounts()
                accounts = list
                if (list.none { it.id == accountId }) {
                    accountId = list.firstOrNull()?.id
                }
            } catch (e: Exception) {
                fail(e)
            } finally {
                accountsLoaded = true
                show(mode)
            }
        }
    }

    fun onResume() {
        exitRequestedAt = 0L
        checkStorage()
        kickTransfers()
        if (backupsOpen) refreshBackups()
    }

    fun showToast(text: String, isError: Boolean = false) {
        toast = ToastMessage(text, isError)
        toastJob?.cancel()
        toastJob = viewModelScope.launch {
            delay(if (isError) 8000L else 4000L)
            toast = null
        }
    }

    fun dismissToast() {
        toastJob?.cancel()
        toast = null
    }

    fun fail(err: Throwable) {
        showToast(err.message ?: err.toString(), isError = true)
    }

    // ---- listings ----

    fun navigate(target: String, fromPullRefresh: Boolean = false) {
        exitRequestedAt = 0L
        val seq = ++requestSeq
        val forMode = mode
        val acc = account
        if (fromPullRefresh) refreshing = true else loading = true
        error = ""
        viewModelScope.launch {
            try {
                if (forMode == PaneMode.CLOUD && acc == null) return@launch
                val result = if (forMode == PaneMode.CLOUD) {
                    val entries = markEncrypted(accounts, acc!!, BelkaCore.listDir(acc.id, target))
                    Listing(
                        path = target,
                        parent = if (target == "/") null else parentPath(target),
                        crumbs = splitCloudCrumbs(target),
                        entries = entries,
                    )
                } else {
                    BelkaCore.listLocal(target)
                }
                if (seq != requestSeq) return@launch
                if (result.path != listing?.path) selectedPaths = emptySet()
                listing = result
                if (forMode == PaneMode.CLOUD) {
                    cloudPaths[acc!!.id] = result.path
                    prefs.setCloudPaths(cloudPaths)
                    refreshQuota(acc, result.path)
                } else {
                    localPath = result.path
                    prefs.localPath = localPath
                    quota = null
                }
            } catch (e: Exception) {
                if (seq != requestSeq) return@launch
                error = e.message ?: e.toString()
                if (listing == null) {
                    listing = Listing(
                        path = target,
                        parent = if (forMode == PaneMode.CLOUD) {
                            if (target == "/") null else parentPath(target)
                        } else {
                            val trimmed = target.trimEnd('/')
                            when {
                                target.isEmpty() -> null
                                target == "/" || trimmed == ANDROID_STORAGE || parentPath(trimmed) == "/storage" -> ""
                                else -> parentPath(target)
                            }
                        },
                    )
                }
            } finally {
                if (seq == requestSeq) {
                    loading = false
                    refreshing = false
                }
            }
        }
    }

    fun reload(fromPullRefresh: Boolean = false) {
        navigate(currentPath, fromPullRefresh)
    }

    fun show(next: PaneMode, target: String? = null) {
        exitRequestedAt = 0L
        ++requestSeq
        mode = next
        prefs.mode = next
        listing = null
        error = ""
        selectedPaths = emptySet()
        if (next == PaneMode.CLOUD) {
            val acc = account
            if (acc != null) {
                navigate(target ?: cloudPaths[acc.id] ?: "/")
            } else {
                loading = false
                quota = null
            }
        } else {
            quota = null
            if (storageOk) {
                navigate(target ?: localPath)
            } else {
                loading = false
            }
        }
    }

    fun selectAccount(acc: Account, target: String? = null) {
        accountId = acc.id
        prefs.accountId = acc.id
        show(PaneMode.CLOUD, target)
    }

    fun goUp() {
        val parent = listing?.parent ?: return
        navigate(parent)
    }

    private fun refreshQuota(acc: Account, path: String, versionBump: Boolean = false) {
        if (path != "/" || pick != null) return
        val scope = if (acc.kind == io.github.tatarinovs.belka.core.ProviderKind.SYNOLOGY) {
            "/" + path.split('/').firstOrNull { it.isNotEmpty() }.orEmpty()
        } else {
            "/"
        }
        val key = "${acc.id}:$scope:${if (versionBump) quotaVersion else 0}"
        if (!versionBump && key == quotaKey && quota != null) return
        if ("${acc.id}:$scope" != quotaKey.substringBeforeLast(':')) quota = null
        quotaKey = key
        val seq = ++quotaSeq
        viewModelScope.launch {
            val q = runCatching { BelkaCore.cloudQuota(acc.id, scope) }.getOrNull()
            if (seq == quotaSeq) quota = q
        }
    }

    // ---- taps & selection ----

    fun onEntryTap(entry: Entry) {
        val currentPick = pick
        if (selecting && currentPick == null) {
            if (entry.icon == null) toggleSelection(entry)
            return
        }
        val opens = entry.opensAccount
        if (opens != null && currentPick == null) {
            accounts.firstOrNull { it.id == opens }?.let { selectAccount(it) }
        } else if (entry.is_dir) {
            navigate(entry.path)
        } else if (currentPick != null) {
            return
        } else if (mode == PaneMode.CLOUD) {
            val files = sortedEntries.filter { !it.is_dir }
            val start = files.indexOfFirst { it.path == entry.path }.coerceAtLeast(0)
            account?.let { acc -> previewing = PreviewSession(acc, files, start) }
        } else {
            runCatching { AndroidPlatform.openFile(appCtx, entry.path) }.onFailure(::fail)
        }
    }

    fun onEntryLongPress(entry: Entry) {
        val currentPick = pick
        if (currentPick is PickMode.BackupLocal || currentPick is PickMode.BackupRemote) {
            if (!entry.is_dir || loading || error.isNotEmpty() || busy.isNotEmpty() ||
                (mode == PaneMode.LOCAL && !storageOk)
            ) {
                return
            }
            chooseBackupFolder(entry.path)
            return
        }
        if (currentPick != null || entry.icon != null) return
        toggleSelection(entry)
    }

    fun toggleSelection(entry: Entry) {
        selectedPaths = if (selectedPaths.contains(entry.path)) {
            selectedPaths - entry.path
        } else {
            selectedPaths + entry.path
        }
    }

    fun clearSelection() {
        selectedPaths = emptySet()
    }

    fun selectAll() {
        val all = sortedEntries.filter { it.icon == null }.map { it.path }.toSet()
        selectedPaths = if (selectedPaths.size == all.size && selectedPaths.containsAll(all)) {
            emptySet()
        } else {
            all
        }
    }

    // ---- transfers & picks ----

    fun startDownload() {
        val acc = account ?: return
        val items = selectedEntries
        if (items.isEmpty()) return
        pick = PickMode.Download(acc, items)
        show(PaneMode.LOCAL, localPath.ifEmpty { Prefs.ANDROID_DOWNLOADS })
    }

    fun startUpload() {
        val items = selectedEntries.filter { it.icon == null }
        if (items.isEmpty()) return
        if (accounts.isEmpty()) {
            showToast("Сначала подключите облако")
            addingAccount = true
            return
        }
        pick = PickMode.Upload(items.map { it.path })
        if (account == null) {
            accountId = accounts.first().id
            prefs.accountId = accountId
        }
        show(PaneMode.CLOUD)
    }

    fun startCopy() {
        val acc = account ?: return
        val items = selectedEntries.filter { it.icon == null }
        if (items.isEmpty()) return
        pick = PickMode.Copy(acc, items)
        selectedPaths = emptySet()
    }

    fun cancelPick() {
        val currentPick = pick ?: return
        if (currentPick is PickMode.BackupLocal || currentPick is PickMode.BackupRemote) {
            pick = null
            return
        }
        val backMode = if (currentPick is PickMode.Upload) PaneMode.LOCAL else PaneMode.CLOUD
        when (currentPick) {
            is PickMode.Download -> accountId = currentPick.account.id
            is PickMode.Copy -> accountId = currentPick.account.id
            else -> {}
        }
        prefs.accountId = accountId
        pick = null
        show(backMode)
    }

    fun confirmPick() {
        val currentPick = pick ?: return
        val dir = currentPath
        if (dir.isEmpty() || busy.isNotEmpty()) return
        if (currentPick is PickMode.BackupLocal || currentPick is PickMode.BackupRemote) {
            chooseBackupFolder(dir)
            return
        }
        viewModelScope.launch {
            busy = when (currentPick) {
                is PickMode.Download -> "Подготовка скачивания…"
                is PickMode.Copy -> "Подготовка копирования…"
                is PickMode.Upload -> "Подготовка загрузки…"
                else -> ""
            }
            try {
                when {
                    currentPick is PickMode.Copy && mode == PaneMode.CLOUD -> {
                        val acc = account ?: return@launch
                        if (currentPick.account.id == acc.id &&
                            currentPick.items.any { parentPath(it.path) == dir }
                        ) {
                            showToast("Это та же папка: откройте другую", isError = true)
                            return@launch
                        }
                        val existing = sortedEntries.map { it.name.lowercase() }.toSet()
                        val conflicts = currentPick.items
                            .map { it.name }
                            .filter { existing.contains(it.lowercase()) }
                        if (conflicts.isNotEmpty()) {
                            requestReplaceConfirm(conflicts, "В этой папке облака") {
                                executeCopy(currentPick, acc, dir)
                            }
                            return@launch
                        }
                        executeCopy(currentPick, acc, dir)
                    }

                    currentPick is PickMode.Download || currentPick is PickMode.Copy -> {
                        val srcAccount = if (currentPick is PickMode.Download) {
                            currentPick.account
                        } else {
                            (currentPick as PickMode.Copy).account
                        }
                        val items = if (currentPick is PickMode.Download) {
                            currentPick.items
                        } else {
                            (currentPick as PickMode.Copy).items
                        }
                        val conflicts = BelkaCore.localConflicts(dir, items.map { it.name })
                        if (conflicts.isNotEmpty()) {
                            requestReplaceConfirm(conflicts, "В этой папке") {
                                executeDownload(srcAccount, items, dir)
                            }
                            return@launch
                        }
                        executeDownload(srcAccount, items, dir)
                    }

                    currentPick is PickMode.Upload -> {
                        val acc = account ?: return@launch
                        val existing = sortedEntries.map { it.name.lowercase() }.toSet()
                        val conflicts = currentPick.paths
                            .map(::baseName)
                            .filter { existing.contains(it.lowercase()) }
                        if (conflicts.isNotEmpty()) {
                            requestReplaceConfirm(conflicts, "В этой папке облака") {
                                executeUpload(acc, currentPick.paths, dir)
                            }
                            return@launch
                        }
                        executeUpload(acc, currentPick.paths, dir)
                    }
                }
            } catch (e: Exception) {
                fail(e)
            } finally {
                busy = ""
            }
        }
    }

    private fun requestReplaceConfirm(
        conflicts: List<String>,
        where: String,
        proceed: suspend () -> Unit,
    ) {
        val list = conflicts.take(5).joinToString("\n") { "• $it" }
        val more = if (conflicts.size > 5) "\n…и ещё ${conflicts.size - 5}" else ""
        confirm = ConfirmState(
            title = "Файлы уже существуют",
            message = "$where уже есть:\n$list$more\n\nЗаменить?",
            okLabel = "Заменить",
            onConfirm = {
                confirm = null
                viewModelScope.launch {
                    busy = "Подготовка…"
                    try {
                        proceed()
                    } catch (e: Exception) {
                        fail(e)
                    } finally {
                        busy = ""
                    }
                }
            },
        )
    }

    private suspend fun executeCopy(currentPick: PickMode.Copy, acc: Account, dir: String) {
        val count = BelkaCore.cloudCopy(currentPick.account.id, currentPick.items, acc.id, dir)
        showToast("Копирование: ${plural(count, "файл", "файла", "файлов")}")
        kickTransfers()
        pick = null
    }

    private suspend fun executeDownload(srcAccount: Account, items: List<Entry>, dir: String) {
        val count = BelkaCore.download(srcAccount.id, items, dir)
        showToast("Скачивание: ${plural(count, "файл", "файла", "файлов")}")
        kickTransfers()
        cancelPick()
    }

    private suspend fun executeUpload(acc: Account, paths: List<String>, dir: String) {
        val count = BelkaCore.upload(acc.id, paths, dir)
        showToast("Загрузка в облако: ${plural(count, "файл", "файла", "файлов")}")
        kickTransfers()
        cancelPick()
    }

    // ---- backups ----

    fun openBackups() {
        backupsOpen = true
        refreshBackups()
    }

    fun closeBackups() {
        backupsOpen = false
        backupDraft = null
    }

    fun refreshBackups() {
        viewModelScope.launch {
            try {
                backupTasks = BelkaCore.backupTasks()
            } catch (e: Exception) {
                if (backupDraft == null) fail(e)
            } finally {
                backupsLoaded = true
            }
        }
    }

    fun startNewBackup() {
        backupDraft = newBackupTask("", accountId ?: accounts.firstOrNull()?.id.orEmpty(), "")
    }

    fun editBackup(task: BackupTask) {
        backupDraft = task
    }

    fun closeBackupDraft() {
        backupDraft = null
    }

    fun pickBackupFolder(side: String) {
        val draft = backupDraft ?: return
        if (side == "local") {
            pick = PickMode.BackupLocal
            show(PaneMode.LOCAL, draft.local.ifEmpty { BACKUP_DEFAULT_LOCAL })
        } else {
            pick = PickMode.BackupRemote
            val acc = accounts.firstOrNull { it.id == draft.account_id }
                ?: account
                ?: accounts.firstOrNull()
            if (acc != null) {
                accountId = acc.id
                prefs.accountId = acc.id
            }
            show(PaneMode.CLOUD, draft.remote.ifEmpty { null })
        }
    }

    private fun chooseBackupFolder(dir: String) {
        val draft = backupDraft ?: return
        if (dir.isEmpty()) return
        when (pick) {
            is PickMode.BackupLocal -> backupDraft = draft.copy(local = dir)
            is PickMode.BackupRemote -> {
                val acc = account ?: return
                backupDraft = draft.copy(account_id = acc.id, remote = dir)
            }
            else -> return
        }
        pick = null
    }

    fun backupFolderFromEntry(entry: Entry) {
        backupDraft = newBackupTask(entry.path, accountId ?: accounts.firstOrNull()?.id.orEmpty())
        selectedPaths = emptySet()
        openBackups()
    }

    fun runBackupNow(task: BackupTask) {
        BackupWorker.runNow(appCtx, task.id, task.name)
        showToast("Автобэкап «${task.name}» запущен")
        viewModelScope.launch {
            delay(1500)
            refreshBackups()
        }
    }

    fun cancelBackup(task: BackupTask) {
        viewModelScope.launch {
            runCatching { BelkaCore.backupCancel(task.id) }.onFailure(::fail)
            refreshBackups()
        }
    }

    fun toggleBackupEnabled(task: BackupTask) {
        viewModelScope.launch {
            try {
                BelkaCore.backupSave(task.copy(enabled = !task.enabled))
                BackupWorker.schedule(appCtx, BelkaCore.backupList())
                refreshBackups()
            } catch (e: Exception) {
                fail(e)
            }
        }
    }

    suspend fun saveBackupDraft(task: BackupTask): BackupTask {
        val saved = BelkaCore.backupSave(task)
        BackupWorker.schedule(appCtx, BelkaCore.backupList())
        backupDraft = null
        refreshBackups()
        return saved
    }

    suspend fun removeBackupTask(id: String) {
        BelkaCore.backupRemove(id)
        BackupWorker.schedule(appCtx, BelkaCore.backupList())
        backupDraft = null
        refreshBackups()
    }

    // ---- file operations ----

    private fun runOperation(label: String, block: suspend () -> Unit) {
        viewModelScope.launch {
            busy = label
            try {
                block()
            } catch (e: Exception) {
                fail(e)
            } finally {
                busy = ""
            }
        }
    }

    fun createNewFolder() {
        if (!canModify) return
        prompt = PromptState(
            title = "Новая папка",
            initialValue = "Новая папка",
            okLabel = "Создать",
            selectFileStem = false,
            onResult = { value ->
                prompt = null
                val trimmed = value?.trim().orEmpty()
                if (trimmed.isEmpty()) return@PromptState
                val parent = currentPath
                runOperation("Создание папки…") {
                    if (mode == PaneMode.CLOUD) {
                        BelkaCore.cloudMkdir(account!!.id, parent, trimmed)
                    } else {
                        BelkaCore.localMkdir(parent, trimmed)
                    }
                    reload()
                }
            },
        )
    }

    fun renameEntry(entry: Entry) {
        prompt = PromptState(
            title = "Переименовать",
            initialValue = entry.name,
            okLabel = "Сохранить",
            selectFileStem = !entry.is_dir,
            onResult = { value ->
                prompt = null
                val trimmed = value?.trim().orEmpty()
                if (trimmed.isEmpty() || trimmed == entry.name) return@PromptState
                runOperation("Переименование…") {
                    if (mode == PaneMode.CLOUD) {
                        BelkaCore.cloudRename(account!!.id, entry.path, trimmed, entry.is_dir)
                    } else {
                        BelkaCore.localRename(entry.path, trimmed)
                    }
                    selectedPaths = emptySet()
                    reload()
                }
            },
        )
    }

    fun deleteEntries(items: List<Entry>) {
        if (items.isEmpty()) return
        val where = if (mode == PaneMode.CLOUD) "из облака «${account?.name}»" else "с телефона"
        confirm = ConfirmState(
            title = "Удаление",
            message = "Удалить ${describeEntries(items)} $where?\n\nФайлы удаляются безвозвратно.",
            okLabel = "Удалить",
            onConfirm = {
                confirm = null
                runOperation("Удаление…") {
                    if (mode == PaneMode.CLOUD) {
                        BelkaCore.cloudDelete(account!!.id, items)
                    } else {
                        BelkaCore.localDelete(items.map { it.path })
                    }
                    selectedPaths = emptySet()
                    reload()
                }
            },
        )
    }

    fun shareQuick(items: List<Entry>) {
        val acc = account ?: return
        runOperation("Создание ссылки…") {
            val links = BelkaCore.cloudShare(acc.id, items, null)
            AndroidPlatform.copyToClipboard(appCtx, links.joinToString("\n"))
            selectedPaths = emptySet()
            showToast(
                if (links.size == 1) "Ссылка скопирована: ${links[0]}"
                else "Ссылки скопированы (${links.size})",
            )
        }
    }

    suspend fun shareWithOptions(items: List<Entry>, options: ShareOptions) {
        val acc = account ?: return
        val links = BelkaCore.cloudShare(acc.id, items, options)
        AndroidPlatform.copyToClipboard(appCtx, links.joinToString("\n"))
        sharing = null
        selectedPaths = emptySet()
        showToast(
            if (links.size == 1) "Ссылка скопирована: ${links[0]}"
            else "Ссылки скопированы (${links.size})",
        )
    }

    fun unshare(items: List<Entry>) {
        val acc = account ?: return
        runOperation("Закрытие доступа…") {
            BelkaCore.cloudUnshare(acc.id, items)
            selectedPaths = emptySet()
            showToast("Доступ по ссылке закрыт: ${describeEntries(items)}")
        }
    }

    // ---- menus ----

    fun dismissSheet() {
        sheet = null
    }

    fun openMainMenu() {
        val items = mutableListOf(
            SheetMenuItem("Новая папка", BelkaIcon.PLUS, disabled = !canModify, action = ::createNewFolder),
            SheetMenuItem("Обновить", BelkaIcon.REFRESH, action = { reload() }),
            SheetMenuItem("", isSeparator = true),
            SheetMenuItem("По имени", BelkaIcon.SORT, checked = sortKey == SortKey.NAME, action = { setSort(SortKey.NAME) }),
            SheetMenuItem("Сначала новые", BelkaIcon.CLOCK, checked = sortKey == SortKey.MODIFIED, action = { setSort(SortKey.MODIFIED) }),
            SheetMenuItem("Сначала большие", BelkaIcon.SORT, checked = sortKey == SortKey.SIZE, action = { setSort(SortKey.SIZE) }),
            SheetMenuItem("", isSeparator = true),
            SheetMenuItem("Автобэкап", BelkaIcon.CLOCK, action = ::openBackups),
            SheetMenuItem("Аккаунты", BelkaIcon.SETTINGS, action = { managingAccounts = true }),
        )
        if (!storageOk) {
            items.add(SheetMenuItem("Разрешить доступ к файлам", BelkaIcon.LOCK, action = ::requestStorage))
        }
        items.add(SheetMenuItem("Проверить обновления", BelkaIcon.DOWNLOAD, action = { checkUpdate(manual = true) }))
        sheet = SheetMenu(title = "Belka ${BuildConfig.VERSION_NAME}", items = items)
    }

    fun setSort(key: SortKey) {
        sortKey = key
        prefs.sortKey = key
    }

    fun openAccountsSheet() {
        val items = accounts.map { acc ->
            SheetMenuItem(
                label = acc.name,
                badge = acc,
                checked = acc.id == accountId,
                action = { selectAccount(acc) },
            )
        }.toMutableList()
        items.add(SheetMenuItem("", isSeparator = true))
        items.add(SheetMenuItem("Добавить аккаунт", BelkaIcon.PLUS, action = { addingAccount = true }))
        items.add(SheetMenuItem("Управление аккаунтами", BelkaIcon.SETTINGS, action = { managingAccounts = true }))
        sheet = SheetMenu(title = "Облака", items = items)
    }

    fun openSelectionMenu() {
        val items = selectedEntries.filter { it.icon == null }
        val single = items.singleOrNull()
        val acc = account
        val def = acc?.let { providerDef(it.kind) }
        val list = mutableListOf<SheetMenuItem>()
        if (single != null && !single.is_dir && mode == PaneMode.CLOUD) {
            list.add(SheetMenuItem("Просмотр", BelkaIcon.EYE, action = { onEntryTap(single) }))
        }
        if (single != null && !single.is_dir && mode == PaneMode.LOCAL) {
            list.add(SheetMenuItem("Открыть", BelkaIcon.FILE, action = {
                runCatching { AndroidPlatform.openFile(appCtx, single.path) }.onFailure(::fail)
            }))
        }
        if (single != null && single.is_dir && mode == PaneMode.LOCAL && accounts.isNotEmpty()) {
            list.add(SheetMenuItem("Автобэкап этой папки", BelkaIcon.CLOCK, action = { backupFolderFromEntry(single) }))
        }
        list.add(SheetMenuItem("Переименовать", BelkaIcon.EDIT, disabled = single == null, action = {
            single?.let(::renameEntry)
        }))
        if (mode == PaneMode.CLOUD && items.isNotEmpty()) {
            list.add(SheetMenuItem("Копировать в другую папку или облако", BelkaIcon.COPY, action = ::startCopy))
        }
        if (mode == PaneMode.CLOUD && def?.share == true) {
            if (def.shareOptions) {
                list.add(SheetMenuItem("Ссылка с паролем или сроком", BelkaIcon.LINK, action = { sharing = items }))
            }
            list.add(SheetMenuItem("Закрыть доступ по ссылке", BelkaIcon.LOCK, action = { unshare(items) }))
        }
        list.add(SheetMenuItem("", isSeparator = true))
        list.add(SheetMenuItem("Удалить", BelkaIcon.TRASH, danger = true, action = { deleteEntries(items) }))
        sheet = SheetMenu(items = list)
    }

    // ---- accounts ----

    fun openAddAccount() {
        addingAccount = true
    }

    fun openEditAccount(acc: Account) {
        editingAccount = acc
    }

    fun closeAccountDialog() {
        addingAccount = false
        editingAccount = null
    }

    fun openManageAccounts() {
        managingAccounts = true
    }

    fun closeManageAccounts() {
        managingAccounts = false
    }

    fun onAccountSaved(saved: Account) {
        val wasEditing = editingAccount != null
        if (!wasEditing) {
            accounts = accounts + saved
            addingAccount = false
            editingAccount = null
            if (pick is PickMode.Upload) {
                accountId = saved.id
                prefs.accountId = saved.id
                show(PaneMode.CLOUD, "/")
            } else {
                selectAccount(saved, "/")
            }
            return
        }
        accounts = accounts.map { if (it.id == saved.id) saved else it }
        editingAccount = null
        if (mode == PaneMode.CLOUD && accountId == saved.id) reload()
    }

    suspend fun removeAccount(acc: Account) {
        BelkaCore.removeAccount(acc.id)
        accounts = accounts.filterNot { it.id == acc.id }
        cloudPaths.remove(acc.id)
        prefs.setCloudPaths(cloudPaths)
        if (accountId == acc.id) {
            accountId = accounts.firstOrNull()?.id
            prefs.accountId = accountId
            if (mode == PaneMode.CLOUD) show(PaneMode.CLOUD)
        }
    }

    suspend fun reorderAccounts(ids: List<String>) {
        BelkaCore.reorderAccounts(ids)
        val byId = accounts.associateBy { it.id }
        accounts = ids.mapNotNull { byId[it] }
    }

    suspend fun reloadAccountsAfterImport() {
        accounts = BelkaCore.listAccounts()
        if (account == null) {
            accounts.firstOrNull()?.let { selectAccount(it) }
        }
    }

    // ---- storage access & updates ----

    fun checkStorage() {
        val was = storageOk
        storageOk = AndroidPlatform.hasStorageAccess(appCtx)
        if (storageOk && !was && mode == PaneMode.LOCAL) show(PaneMode.LOCAL)
    }

    fun requestStorage() {
        runCatching { AndroidPlatform.requestStorageAccess(appCtx) }.onFailure(::fail)
    }

    fun checkUpdate(manual: Boolean = false) {
        viewModelScope.launch {
            try {
                prefs.updateCheckedAt = System.currentTimeMillis()
                val res = BelkaCore.checkUpdate()
                updateInfo = res
                if (manual && res == null) showToast("Установлена последняя версия")
            } catch (e: Exception) {
                if (manual) fail(e)
            }
        }
    }

    fun dismissUpdate() {
        updateInfo = null
    }

    fun downloadUpdate() {
        val u = updateInfo ?: return
        runCatching { AndroidPlatform.openUrl(appCtx, u.apk ?: u.page) }.onFailure(::fail)
    }

    fun closePreview() {
        previewing = null
    }

    fun closeShareDialog() {
        sharing = null
    }

    fun dismissConfirm() {
        confirm = null
    }

    fun dismissPrompt() {
        prompt?.onResult?.invoke(null)
        prompt = null
    }

    // ---- transfers polling ----

    fun kickTransfers() {
        if (transferPollJob?.isActive == true) return
        transferPollJob = viewModelScope.launch {
            while (true) {
                refreshTransfersOnce()
                if (transferCounts.activeCount == 0) break
                delay(TRANSFER_POLL_MS)
            }
            transferPollJob = null
        }
    }

    suspend fun refreshTransfersOnce() {
        val seq = ++transferReqSeq
        try {
            val res = BelkaCore.listTransfers(transferFilter)
            if (seq != transferReqSeq) return
            trackTransfers(res.items)
            transferItems = res.items
            transferViewItems = res.view_items ?: res.items
            transferCounts = res.counts
            transferError = ""
        } catch (e: Exception) {
            if (seq != transferReqSeq) return
            transferError = e.message ?: e.toString()
        }
    }

    fun selectTransferFilter(filter: TransferFilter) {
        transferFilter = filter
        transferViewItems = emptyList()
        viewModelScope.launch { refreshTransfersOnce() }
    }

    private fun trackTransfers(items: List<Transfer>) {
        val now = SystemClock.elapsedRealtime()
        val speeds = mutableMapOf<Long, Double>()
        val waiting = mutableSetOf<Long>()
        val shownIds = items.map { it.id }.toSet()
        lastBytes.keys.retainAll(shownIds)
        prevStates.keys.retainAll(shownIds)
        runningSince.keys.retainAll(shownIds)

        var anyFinished = false
        for (t in items) {
            val prev = lastBytes[t.id]
            if (t.state == TransferState.RUNNING && prev != null && now > prev.second) {
                val instant = ((t.done - prev.first).coerceAtLeast(0) * 1000.0) / (now - prev.second)
                val oldSpeed = transferSpeeds[t.id]
                speeds[t.id] = if (oldSpeed == null) instant else oldSpeed * 0.7 + instant * 0.3
            }
            lastBytes[t.id] = t.done to now

            if (t.state == TransferState.RUNNING) {
                val since = runningSince.getOrPut(t.id) { now }
                if (t.done == 0L && now - since > WAITING_MS) waiting.add(t.id)
            } else {
                runningSince.remove(t.id)
            }

            val before = prevStates[t.id]
            if (t.state == TransferState.DONE && before != TransferState.DONE) {
                onTransferFinished(t)
                anyFinished = true
            }
            prevStates[t.id] = t.state
        }
        transferSpeeds = speeds
        transferWaiting = waiting
        if (anyFinished) {
            reloadJob?.cancel()
            reloadJob = viewModelScope.launch {
                delay(600)
                if (busy.isEmpty() && !selecting) reload()
            }
        }
    }

    private fun onTransferFinished(t: Transfer) {
        val acc = account
        if (t.direction != TransferDirection.DOWNLOAD && t.account_id == accountId && acc != null) {
            quotaJob?.cancel()
            quotaJob = viewModelScope.launch {
                delay(2000)
                quotaVersion++
                refreshQuota(acc, currentPath, versionBump = true)
            }
        }
    }

    fun cancelTransfer(id: Long) {
        viewModelScope.launch {
            runCatching { BelkaCore.cancelTransfer(id) }
            refreshTransfersOnce()
        }
    }

    fun cancelAllTransfers() {
        viewModelScope.launch {
            runCatching { BelkaCore.cancelAllTransfers() }
            refreshTransfersOnce()
        }
    }

    fun retryTransfer(id: Long) {
        viewModelScope.launch {
            runCatching { BelkaCore.retryTransfer(id) }.onFailure(::fail)
            kickTransfers()
        }
    }

    fun retryFailedTransfers(onMessage: (String, String?) -> Unit) {
        viewModelScope.launch {
            try {
                val res = BelkaCore.retryFailedTransfers()
                val msg = "Поставлено на повтор: ${res.retried}"
                val err = if (res.errors.isNotEmpty()) {
                    "Не удалось повторить: ${res.errors.size}. ${res.errors[0]}"
                } else {
                    null
                }
                onMessage(msg, err)
                refreshTransfersOnce()
                kickTransfers()
            } catch (e: Exception) {
                onMessage("", e.message ?: e.toString())
            }
        }
    }

    fun clearTransfers() {
        viewModelScope.launch {
            runCatching { BelkaCore.clearTransfers() }
            refreshTransfersOnce()
        }
    }

    fun openDownloadedFile(path: String) {
        runCatching { AndroidPlatform.openFile(appCtx, path) }.onFailure(::fail)
    }

    // ---- system back ----

    /** Returns `true` when the back event was consumed, `false` to move task to back. */
    fun onSystemBack(): Boolean {
        when {
            prompt != null -> dismissPrompt()
            confirm != null -> dismissConfirm()
            sheet != null -> dismissSheet()
            previewing != null -> closePreview()
            sharing != null -> closeShareDialog()
            addingAccount || editingAccount != null -> {
                viewModelScope.launch { runCatching { BelkaCore.oauthCancel() } }
                closeAccountDialog()
            }
            managingAccounts -> closeManageAccounts()
            selecting -> clearSelection()
            pick != null -> cancelPick()
            backupDraft != null -> closeBackupDraft()
            backupsOpen -> closeBackups()
            loading -> return true
            !atRoot -> goUp()
            else -> {
                val now = SystemClock.elapsedRealtime()
                if (exitRequestedAt == 0L || now - exitRequestedAt > 2000L) {
                    exitRequestedAt = now
                    showToast("Нажмите «Назад» ещё раз, чтобы выйти")
                    return true
                }
                return false
            }
        }
        exitRequestedAt = 0L
        return true
    }

    companion object {
        private const val UPDATE_CHECK_MS = 24L * 60L * 60L * 1000L
        private const val TRANSFER_POLL_MS = 400L
        private const val WAITING_MS = 5000L
        const val ANDROID_STORAGE = "/storage/emulated/0"
        const val BACKUP_DEFAULT_LOCAL = "/storage/emulated/0/DCIM"
    }
}

private fun splitCloudCrumbs(path: String): List<Crumb> {
    val parts = path.split('/').filter { it.isNotEmpty() }
    return parts.mapIndexed { i, name ->
        Crumb(name = name, path = "/" + parts.take(i + 1).joinToString("/"))
    }
}

private fun parseIsoMillis(iso: String): Long =
    runCatching { Instant.parse(iso).toEpochMilli() }.getOrDefault(0L)

private fun compareNatural(a: String, b: String, collator: Collator): Int {
    var ia = 0
    var ib = 0
    while (ia < a.length && ib < b.length) {
        val ca = a[ia]
        val cb = b[ib]
        if (ca.isDigit() && cb.isDigit()) {
            var ea = ia
            while (ea < a.length && a[ea].isDigit()) ea++
            var eb = ib
            while (eb < b.length && b[eb].isDigit()) eb++
            val na = a.substring(ia, ea).trimStart('0')
            val nb = b.substring(ib, eb).trimStart('0')
            if (na.length != nb.length) return na.length.compareTo(nb.length)
            val numCmp = na.compareTo(nb)
            if (numCmp != 0) return numCmp
            ia = ea
            ib = eb
        } else {
            var ea = ia
            while (ea < a.length && !a[ea].isDigit()) ea++
            var eb = ib
            while (eb < b.length && !b[eb].isDigit()) eb++
            val cmp = collator.compare(a.substring(ia, ea), b.substring(ib, eb))
            if (cmp != 0) return cmp
            ia = ea
            ib = eb
        }
    }
    return a.length.compareTo(b.length)
}
