package io.github.tatarinovs.belka.ui

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.systemBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.tatarinovs.belka.core.Entry
import io.github.tatarinovs.belka.core.PaneMode
import io.github.tatarinovs.belka.core.baseName
import io.github.tatarinovs.belka.core.formatBytes
import io.github.tatarinovs.belka.core.formatDate
import io.github.tatarinovs.belka.core.providerDef
import io.github.tatarinovs.belka.ui.icons.BelkaIcon
import io.github.tatarinovs.belka.ui.theme.BelkaTheme
import io.github.tatarinovs.belka.ui.theme.ProviderBadge

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun MainScreen(vm: BelkaViewModel) {
    val colors = BelkaTheme.colors
    val haptic = LocalHapticFeedback.current

    val account = vm.account
    val pick = vm.pick
    val entries = vm.sortedEntries
    val keyedEntries = remember(entries) {
        val seen = mutableMapOf<String, Int>()
        entries.map { e ->
            val n = seen.getOrDefault(e.path, 0)
            seen[e.path] = n + 1
            val key = if (n == 0) e.path else "${e.path}\u0000$n"
            key to e
        }
    }

    Box(
        modifier = Modifier
            .fillMaxSize()
            .background(colors.surface)
            .systemBarsPadding()
            .imePadding(),
    ) {
        Column(modifier = Modifier.fillMaxSize()) {
            // Top Bar
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .height(56.dp)
                    .padding(horizontal = 4.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                if (vm.selecting && pick == null) {
                    BelkaToolButton(
                        icon = BelkaIcon.CLOSE,
                        contentDescription = "Снять выделение",
                        onClick = vm::clearSelection,
                    )
                    Text(
                        text = "Выбрано: ${vm.selectedPaths.size}",
                        color = colors.text,
                        fontSize = 19.sp,
                        fontWeight = FontWeight.SemiBold,
                        modifier = Modifier.weight(1f),
                    )
                    BelkaToolButton(
                        icon = BelkaIcon.CHECK_ALL,
                        contentDescription = "Выбрать все",
                        onClick = vm::selectAll,
                    )
                } else {
                    if (!vm.atRoot) {
                        BelkaToolButton(
                            icon = BelkaIcon.ARROW_LEFT,
                            contentDescription = "Вверх",
                            onClick = vm::goUp,
                        )
                    }
                    Column(
                        modifier = Modifier
                            .weight(1f)
                            .padding(start = if (vm.atRoot) 12.dp else 0.dp),
                    ) {
                        Text(
                            text = vm.title,
                            color = colors.text,
                            fontSize = 19.sp,
                            fontWeight = FontWeight.SemiBold,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                        if (vm.subtitle.isNotEmpty()) {
                            Text(
                                text = vm.subtitle,
                                color = colors.muted,
                                fontSize = 12.sp,
                                maxLines = 1,
                                overflow = TextOverflow.StartEllipsis,
                            )
                        }
                    }
                    BelkaToolButton(
                        icon = BelkaIcon.REFRESH,
                        contentDescription = "Обновить",
                        iconSize = 20.dp,
                        enabled = !(vm.mode == PaneMode.CLOUD && account == null),
                        onClick = { vm.reload() },
                    )
                    BelkaToolButton(
                        icon = BelkaIcon.MORE,
                        contentDescription = "Меню",
                        onClick = vm::openMainMenu,
                    )
                }
            }

            // Pill Tabs
            if (pick == null || pick is PickMode.Upload || pick is PickMode.Copy || pick is PickMode.BackupRemote) {
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(start = 12.dp, end = 12.dp, bottom = 8.dp),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    val cloudActive = vm.mode == PaneMode.CLOUD
                    Row(
                        modifier = Modifier
                            .weight(1f)
                            .height(40.dp)
                            .clip(CircleShape)
                            .background(if (cloudActive) colors.selected else colors.surface)
                            .then(
                                if (cloudActive) Modifier else Modifier.border(1.dp, colors.border, CircleShape),
                            )
                            .clickable {
                                if (cloudActive && vm.accounts.isNotEmpty()) vm.openAccountsSheet()
                                else vm.show(PaneMode.CLOUD)
                            }
                            .padding(horizontal = 12.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.CenterHorizontally),
                    ) {
                        if (account != null) {
                            ProviderBadge(
                                kind = account.kind,
                                short = providerDef(account.kind).short,
                                size = 22.dp,
                            )
                            Text(
                                text = account.name,
                                color = if (cloudActive) colors.text else colors.muted,
                                fontWeight = if (cloudActive) FontWeight.SemiBold else FontWeight.Normal,
                                fontSize = 14.5.sp,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                                modifier = Modifier.weight(1f, fill = false),
                            )
                            Icon(
                                imageVector = BelkaIcon.CHEVRON_DOWN.vector,
                                contentDescription = null,
                                tint = if (cloudActive) colors.text else colors.muted,
                                modifier = Modifier.size(14.dp),
                            )
                        } else {
                            Icon(
                                imageVector = BelkaIcon.CLOUD.vector,
                                contentDescription = null,
                                tint = if (cloudActive) colors.text else colors.muted,
                                modifier = Modifier.size(18.dp),
                            )
                            Text(
                                text = "Облако",
                                color = if (cloudActive) colors.text else colors.muted,
                                fontWeight = if (cloudActive) FontWeight.SemiBold else FontWeight.Normal,
                                fontSize = 14.5.sp,
                                maxLines = 1,
                            )
                        }
                    }

                    val localActive = vm.mode == PaneMode.LOCAL
                    val localDisabled = pick != null && pick !is PickMode.Copy
                    Row(
                        modifier = Modifier
                            .weight(1f)
                            .height(40.dp)
                            .alpha(if (localDisabled) 0.4f else 1f)
                            .clip(CircleShape)
                            .background(if (localActive) colors.selected else colors.surface)
                            .then(
                                if (localActive) Modifier else Modifier.border(1.dp, colors.border, CircleShape),
                            )
                            .clickable(enabled = !localDisabled) { vm.show(PaneMode.LOCAL) }
                            .padding(horizontal = 12.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.CenterHorizontally),
                    ) {
                        Icon(
                            imageVector = BelkaIcon.PHONE.vector,
                            contentDescription = null,
                            tint = if (localActive) colors.text else colors.muted,
                            modifier = Modifier.size(18.dp),
                        )
                        Text(
                            text = "Телефон",
                            color = if (localActive) colors.text else colors.muted,
                            fontWeight = if (localActive) FontWeight.SemiBold else FontWeight.Normal,
                            fontSize = 14.5.sp,
                            maxLines = 1,
                        )
                    }
                }
            }

            if (vm.mode == PaneMode.CLOUD && account != null && vm.atRoot && pick == null && vm.quota != null) {
                HorizontalDivider(color = colors.border)
                QuotaMeterBar(vm.quota)
            }

            if (pick != null) {
                val pickIcon = when (pick) {
                    is PickMode.Download -> BelkaIcon.DOWNLOAD
                    is PickMode.Upload -> BelkaIcon.UPLOAD
                    is PickMode.Copy -> BelkaIcon.COPY
                    is PickMode.BackupLocal, is PickMode.BackupRemote -> BelkaIcon.CLOCK
                }
                val pickText = when (pick) {
                    is PickMode.Copy -> "Откройте папку на телефоне или в облаке, куда скопировать ${
                        if (pick.items.size == 1) "«${pick.items[0].name}»" else "объекты (${pick.items.size})"
                    }"
                    is PickMode.Download -> "Откройте папку на телефоне, куда скачать ${
                        if (pick.items.size == 1) "«${pick.items[0].name}»" else "объекты (${pick.items.size})"
                    }"
                    is PickMode.BackupLocal -> "Откройте папку на телефоне, которую нужно копировать в облако, или выберите её длинным тапом"
                    is PickMode.BackupRemote -> "Откройте папку в облаке, куда складывать копии, или выберите её длинным тапом"
                    is PickMode.Upload -> "Откройте папку в облаке, куда загрузить ${
                        if (pick.paths.size == 1) "«${baseName(pick.paths[0])}»" else "объекты (${pick.paths.size})"
                    }"
                }
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(start = 12.dp, end = 12.dp, bottom = 8.dp)
                        .clip(RoundedCornerShape(10.dp))
                        .background(colors.selected)
                        .padding(horizontal = 12.dp, vertical = 10.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(10.dp),
                ) {
                    Icon(pickIcon.vector, contentDescription = null, tint = colors.text, modifier = Modifier.size(18.dp))
                    Text(text = pickText, color = colors.text, fontSize = 14.sp, modifier = Modifier.weight(1f))
                }
            }

            vm.updateInfo?.let { update ->
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(start = 12.dp, end = 12.dp, bottom = 8.dp)
                        .clip(RoundedCornerShape(10.dp))
                        .background(colors.selected)
                        .padding(start = 12.dp, end = 4.dp, top = 6.dp, bottom = 6.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    Text(
                        text = "Доступна версия ${update.version}",
                        color = colors.text,
                        fontSize = 14.sp,
                        modifier = Modifier.weight(1f),
                    )
                    BelkaPrimaryButton(height = 36.dp, onClick = vm::downloadUpdate) {
                        Text("Скачать", fontSize = 13.5.sp)
                    }
                    BelkaToolButton(
                        icon = BelkaIcon.CLOSE,
                        contentDescription = "Скрыть",
                        size = 36.dp,
                        iconSize = 16.dp,
                        onClick = vm::dismissUpdate,
                    )
                }
            }

            if (vm.loading || vm.busy.isNotEmpty()) {
                LinearProgressIndicator(
                    color = colors.accent,
                    trackColor = colors.surface,
                    modifier = Modifier.fillMaxWidth().height(2.dp),
                )
            } else {
                Spacer(Modifier.height(2.dp))
            }

            HorizontalDivider(color = colors.border)

            // Main content list
            val canRefresh = !(vm.mode == PaneMode.CLOUD && account == null) &&
                !(vm.mode == PaneMode.LOCAL && !vm.storageOk)

            val listContent: @Composable () -> Unit = {
                when {
                    vm.mode == PaneMode.CLOUD && account == null -> {
                        if (vm.accountsLoaded) {
                            EmptyState(
                                icon = BelkaIcon.CLOUD,
                                title = "Belka",
                                message = "Подключите облако, чтобы скачивать и загружать файлы.",
                            ) {
                                BelkaPrimaryButton(
                                    height = 44.dp,
                                    modifier = Modifier.fillMaxWidth(0.78f),
                                    onClick = vm::openAddAccount,
                                ) {
                                    Text("Добавить аккаунт", fontSize = 15.sp)
                                }
                                BelkaSecondaryButton(
                                    height = 44.dp,
                                    modifier = Modifier.fillMaxWidth(0.78f),
                                    onClick = vm::openManageAccounts,
                                ) {
                                    Text("Импорт аккаунтов из файла", fontSize = 15.sp)
                                }
                            }
                        }
                    }
                    vm.mode == PaneMode.LOCAL && !vm.storageOk -> {
                        EmptyState(
                            icon = BelkaIcon.LOCK,
                            title = "Нужен доступ к файлам",
                            message = "Чтобы показывать папки телефона и сохранять в них файлы, разрешите Belka доступ ко всем файлам.",
                        ) {
                            BelkaPrimaryButton(
                                height = 44.dp,
                                modifier = Modifier.fillMaxWidth(0.7f),
                                onClick = vm::requestStorage,
                            ) {
                                Text("Разрешить", fontSize = 15.sp)
                            }
                        }
                    }
                    else -> {
                        Column(modifier = Modifier.fillMaxSize()) {
                            if (vm.error.isNotEmpty()) {
                                BelkaErrorBox(
                                    rawError = vm.error,
                                    modifier = Modifier.padding(12.dp),
                                )
                            }
                            LazyColumn(modifier = Modifier.weight(1f).fillMaxWidth()) {
                                if (keyedEntries.isEmpty() && !vm.loading && vm.error.isEmpty() && vm.listing != null) {
                                    item(key = "__empty__") {
                                        Text(
                                            text = "Папка пуста",
                                            color = colors.muted,
                                            fontSize = 15.sp,
                                            textAlign = TextAlign.Center,
                                            modifier = Modifier
                                                .fillMaxWidth()
                                                .padding(horizontal = 16.dp, vertical = 48.dp),
                                        )
                                    }
                                } else {
                                    items(keyedEntries, key = { it.first }) { (_, entry) ->
                                        val isSelected = vm.selectedPaths.contains(entry.path)
                                        val isMuted = pick != null && !entry.is_dir
                                        EntryRow(
                                            entry = entry,
                                            isSelected = isSelected,
                                            isMuted = isMuted,
                                            onTap = { vm.onEntryTap(entry) },
                                            onLongPress = {
                                                haptic.performHapticFeedback(HapticFeedbackType.LongPress)
                                                vm.onEntryLongPress(entry)
                                            },
                                        )
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if (canRefresh) {
                PullToRefreshBox(
                    isRefreshing = vm.refreshing,
                    onRefresh = { vm.reload(fromPullRefresh = true) },
                    modifier = Modifier
                        .weight(1f)
                        .fillMaxWidth()
                        .alpha(if (vm.loading) 0.6f else 1f),
                ) {
                    listContent()
                }
            } else {
                Box(
                    modifier = Modifier
                        .weight(1f)
                        .fillMaxWidth()
                        .alpha(if (vm.loading) 0.6f else 1f),
                ) {
                    listContent()
                }
            }

            if (vm.toast != null && !(vm.backupsOpen && pick == null)) {
                ToastBanner(toast = vm.toast!!, onDismiss = vm::dismissToast)
            }

            if (pick != null) {
                HorizontalDivider(color = colors.border)
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .background(colors.surface)
                        .padding(horizontal = 12.dp, vertical = 10.dp),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    BelkaSecondaryButton(height = 44.dp, onClick = vm::cancelPick) {
                        Text("Отмена", fontSize = 15.sp)
                    }
                    val confirmEnabled = vm.busy.isEmpty() && !vm.loading && vm.currentPath.isNotEmpty() &&
                        !(vm.mode == PaneMode.LOCAL && !vm.storageOk) &&
                        !(vm.mode == PaneMode.CLOUD && account == null)
                    BelkaPrimaryButton(
                        height = 44.dp,
                        enabled = confirmEnabled,
                        modifier = Modifier.weight(1f),
                        onClick = vm::confirmPick,
                    ) {
                        val icon = when (pick) {
                            is PickMode.Copy -> BelkaIcon.COPY
                            is PickMode.Download -> BelkaIcon.DOWNLOAD
                            is PickMode.Upload -> BelkaIcon.UPLOAD
                            else -> BelkaIcon.CHECK
                        }
                        val label = when (pick) {
                            is PickMode.Copy -> "Копировать сюда"
                            is PickMode.Download -> "Скачать сюда"
                            is PickMode.Upload -> "Загрузить сюда"
                            else -> "Выбрать эту папку"
                        }
                        Icon(icon.vector, contentDescription = null, modifier = Modifier.size(18.dp))
                        Spacer(Modifier.width(6.dp))
                        Text(label, fontSize = 15.sp, maxLines = 1)
                    }
                }
            } else if (vm.selecting) {
                HorizontalDivider(color = colors.border)
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .background(colors.surface)
                        .padding(horizontal = 12.dp, vertical = 10.dp),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    if (vm.mode == PaneMode.CLOUD) {
                        BelkaPrimaryButton(
                            height = 44.dp,
                            enabled = vm.busy.isEmpty(),
                            modifier = Modifier.weight(1f),
                            onClick = vm::startDownload,
                        ) {
                            Icon(BelkaIcon.DOWNLOAD.vector, contentDescription = null, modifier = Modifier.size(18.dp))
                            Spacer(Modifier.width(6.dp))
                            Text("Скачать", fontSize = 15.sp)
                        }
                        if (account != null && providerDef(account.kind).share) {
                            BelkaSecondaryButton(
                                height = 44.dp,
                                enabled = vm.busy.isEmpty(),
                                onClick = { vm.shareQuick(vm.selectedEntries) },
                            ) {
                                Icon(BelkaIcon.LINK.vector, contentDescription = "Поделиться ссылкой", modifier = Modifier.size(18.dp))
                            }
                        }
                    } else {
                        BelkaPrimaryButton(
                            height = 44.dp,
                            enabled = vm.busy.isEmpty() && vm.selectedEntries.any { it.icon == null },
                            modifier = Modifier.weight(1f),
                            onClick = vm::startUpload,
                        ) {
                            Icon(BelkaIcon.UPLOAD.vector, contentDescription = null, modifier = Modifier.size(18.dp))
                            Spacer(Modifier.width(6.dp))
                            Text("Загрузить в облако", fontSize = 15.sp)
                        }
                    }
                    BelkaSecondaryButton(
                        height = 44.dp,
                        enabled = vm.busy.isEmpty(),
                        onClick = vm::openSelectionMenu,
                    ) {
                        Icon(BelkaIcon.MORE.vector, contentDescription = "Ещё", modifier = Modifier.size(18.dp))
                    }
                }
            }

            TransferPanel(
                counts = vm.transferCounts,
                viewItems = vm.transferViewItems,
                filter = vm.transferFilter,
                speeds = vm.transferSpeeds,
                waiting = vm.transferWaiting,
                error = vm.transferError,
                collapsed = vm.transferCollapsed,
                onToggleCollapsed = { vm.transferCollapsed = !vm.transferCollapsed },
                onSelectFilter = vm::selectTransferFilter,
                onCancelAll = vm::cancelAllTransfers,
                onRetryFailed = vm::retryFailedTransfers,
                onClear = vm::clearTransfers,
                onCancel = vm::cancelTransfer,
                onRetry = vm::retryTransfer,
                onOpenDownloaded = vm::openDownloadedFile,
            )
        }

        if (vm.backupsOpen && pick == null) {
            BackupScreen(
                accounts = vm.accounts,
                storageOk = vm.storageOk,
                tasks = vm.backupTasks,
                loaded = vm.backupsLoaded,
                draft = vm.backupDraft,
                onDraftChange = { vm.backupDraft = it },
                onClose = vm::closeBackups,
                onPickFolder = vm::pickBackupFolder,
                onRequestStorage = vm::requestStorage,
                onRefresh = vm::refreshBackups,
                onRunNow = vm::runBackupNow,
                onCancelRun = vm::cancelBackup,
                onToggleEnabled = vm::toggleBackupEnabled,
                onSave = vm::saveBackupDraft,
                onRemove = vm::removeBackupTask,
                onToast = vm::showToast,
            )
            if (vm.toast != null) {
                ToastBanner(
                    toast = vm.toast!!,
                    onDismiss = vm::dismissToast,
                    modifier = Modifier.align(Alignment.BottomCenter),
                )
            }
        }
    }

    vm.previewing?.let { session ->
        PreviewScreen(
            session = session,
            onClose = vm::closePreview,
        )
    }

    vm.sheet?.let { menu ->
        ActionBottomSheet(
            menu = menu,
            onDismiss = vm::dismissSheet,
        )
    }

    vm.prompt?.let { p ->
        PromptDialog(state = p)
    }

    vm.confirm?.let { c ->
        ConfirmDialog(
            state = c,
            onDismiss = vm::dismissConfirm,
        )
    }

    vm.sharing?.let { entriesToShare ->
        if (account != null) {
            ShareDialog(
                entries = entriesToShare,
                onDismiss = vm::closeShareDialog,
                onShare = { options -> vm.shareWithOptions(entriesToShare, options) },
            )
        }
    }

    if (vm.managingAccounts) {
        AccountsDialog(
            accounts = vm.accounts,
            onDismiss = vm::closeManageAccounts,
            onAdd = vm::openAddAccount,
            onEdit = vm::openEditAccount,
            onRemove = vm::removeAccount,
            onReorder = vm::reorderAccounts,
            onImported = vm::reloadAccountsAfterImport,
        )
    }

    if (vm.addingAccount || vm.editingAccount != null) {
        AccountDialog(
            account = vm.editingAccount,
            accounts = vm.accounts,
            onDismiss = vm::closeAccountDialog,
            onSaved = vm::onAccountSaved,
        )
    }
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun EntryRow(
    entry: Entry,
    isSelected: Boolean,
    isMuted: Boolean,
    onTap: () -> Unit,
    onLongPress: () -> Unit,
) {
    val colors = BelkaTheme.colors
    val icon = when (entry.icon) {
        "drive" -> BelkaIcon.DRIVE
        "place" -> BelkaIcon.PLACE
        "cloud" -> BelkaIcon.CLOUD
        else -> if (entry.is_dir) BelkaIcon.FOLDER else BelkaIcon.FILE
    }
    val iconTint = when {
        entry.icon == "place" || entry.icon == "drive" -> colors.accent
        entry.icon == null && entry.is_dir -> colors.folder
        else -> colors.muted
    }
    val metaText = if (entry.icon != null) {
        entry.path
    } else {
        val date = formatDate(entry.modified)
        when {
            entry.is_dir -> date
            date.isNotEmpty() -> "${formatBytes(entry.size)} · $date"
            else -> formatBytes(entry.size)
        }
    }

    Row(
        modifier = Modifier
            .fillMaxWidth()
            .heightIn(min = 60.dp)
            .background(if (isSelected) colors.selected else colors.surface)
            .alpha(if (isMuted) 0.45f else 1f)
            .combinedClickable(
                onClick = onTap,
                onLongClick = onLongPress,
            )
            .padding(horizontal = 16.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        Box(modifier = Modifier.size(36.dp), contentAlignment = Alignment.Center) {
            if (isSelected) {
                Box(
                    modifier = Modifier
                        .size(32.dp)
                        .clip(CircleShape)
                        .background(colors.accent),
                    contentAlignment = Alignment.Center,
                ) {
                    Icon(
                        imageVector = BelkaIcon.CHECK.vector,
                        contentDescription = null,
                        tint = colors.accentText,
                        modifier = Modifier.size(18.dp),
                    )
                }
            } else {
                Icon(
                    imageVector = icon.vector,
                    contentDescription = null,
                    tint = iconTint,
                    modifier = Modifier.size(26.dp),
                )
                if (!entry.locked.isNullOrEmpty()) {
                    Box(
                        modifier = Modifier
                            .align(Alignment.BottomEnd)
                            .clip(CircleShape)
                            .background(colors.surface)
                            .padding(1.dp),
                    ) {
                        Icon(
                            imageVector = BelkaIcon.LOCK.vector,
                            contentDescription = entry.locked,
                            tint = colors.crypt,
                            modifier = Modifier.size(11.dp),
                        )
                    }
                }
            }
        }
        Column(
            modifier = Modifier.weight(1f),
            verticalArrangement = Arrangement.spacedBy(2.dp),
        ) {
            Text(
                text = entry.name,
                color = colors.text,
                fontSize = 15.sp,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
            if (metaText.isNotEmpty()) {
                Text(
                    text = metaText,
                    color = colors.muted,
                    fontSize = 12.5.sp,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
    }
}

@Composable
private fun EmptyState(
    icon: BelkaIcon,
    title: String,
    message: String,
    actions: @Composable () -> Unit,
) {
    val colors = BelkaTheme.colors
    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(horizontal = 24.dp, vertical = 48.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Icon(
            imageVector = icon.vector,
            contentDescription = null,
            tint = colors.muted,
            modifier = Modifier.size(48.dp),
        )
        Text(
            text = title,
            color = colors.text,
            fontSize = 20.sp,
            fontWeight = FontWeight.SemiBold,
        )
        Text(
            text = message,
            color = colors.muted,
            fontSize = 15.sp,
            textAlign = TextAlign.Center,
            modifier = Modifier.padding(bottom = 8.dp),
        )
        actions()
    }
}
