package io.github.tatarinovs.belka.ui

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.gestures.detectDragGesturesAfterLongPress
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.compose.ui.zIndex
import io.github.tatarinovs.belka.core.Account
import io.github.tatarinovs.belka.core.BelkaCore
import io.github.tatarinovs.belka.core.plural
import io.github.tatarinovs.belka.core.providerDef
import io.github.tatarinovs.belka.ui.icons.BelkaIcon
import io.github.tatarinovs.belka.ui.theme.BelkaTheme
import io.github.tatarinovs.belka.ui.theme.ProviderBadge
import java.time.LocalDate
import java.time.format.DateTimeFormatter
import kotlin.math.roundToInt
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

private const val MIN_EXPORT_PASSWORD = 8

private sealed interface BackupFileMode {
    data object Export : BackupFileMode
    data class Import(val name: String, val bytes: ByteArray) : BackupFileMode
}

@Composable
fun AccountsDialog(
    accounts: List<Account>,
    onDismiss: () -> Unit,
    onAdd: () -> Unit,
    onEdit: (Account) -> Unit,
    onRemove: suspend (Account) -> Unit,
    onReorder: suspend (List<String>) -> Unit,
    onImported: suspend () -> Unit,
) {
    val colors = BelkaTheme.colors
    val context = LocalContext.current
    val haptic = LocalHapticFeedback.current
    val scope = rememberCoroutineScope()

    var error by remember { mutableStateOf("") }
    var info by remember { mutableStateOf("") }
    var backupMode by remember { mutableStateOf<BackupFileMode?>(null) }
    var password by remember { mutableStateOf("") }
    var repeat by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    var pendingDelete by remember { mutableStateOf<Account?>(null) }

    // Drag-to-reorder state
    var draggedId by remember { mutableStateOf<String?>(null) }
    var dragOrder by remember(accounts) { mutableStateOf(accounts) }
    var dragOffsetY by remember { mutableFloatStateOf(0f) }
    val listState = rememberLazyListState()

    val shown = if (draggedId != null) dragOrder else accounts

    // Export file creator (after password validation)
    var pendingExportBytes by remember { mutableStateOf<Pair<Int, ByteArray>?>(null) }
    var pendingExportFileName by remember { mutableStateOf("") }

    val exportLauncher = rememberLauncherForActivityResult(
        contract = ActivityResultContracts.CreateDocument("application/octet-stream"),
    ) { uri ->
        val payload = pendingExportBytes
        pendingExportBytes = null
        if (uri == null || payload == null) {
            busy = false
            return@rememberLauncherForActivityResult
        }
        scope.launch {
            try {
                withContext(Dispatchers.IO) {
                    context.contentResolver.openOutputStream(uri)?.use { it.write(payload.second) }
                        ?: error("Не удалось открыть файл для записи")
                }
                backupMode = null
                password = ""
                repeat = ""
                info = "Сохранено: ${accountsText(payload.first)} → $pendingExportFileName"
            } catch (e: Exception) {
                error = e.message ?: e.toString()
            } finally {
                busy = false
            }
        }
    }

    val importLauncher = rememberLauncherForActivityResult(
        contract = ActivityResultContracts.OpenDocument(),
    ) { uri ->
        if (uri == null) return@rememberLauncherForActivityResult
        scope.launch {
            try {
                val (name, bytes) = withContext(Dispatchers.IO) {
                    val data = context.contentResolver.openInputStream(uri)?.use { it.readBytes() }
                        ?: error("Не удалось прочитать файл")
                    val displayName = uri.lastPathSegment?.substringAfterLast('/') ?: "accounts.belka"
                    displayName to data
                }
                error = ""
                info = ""
                password = ""
                repeat = ""
                backupMode = BackupFileMode.Import(name, bytes)
            } catch (e: Exception) {
                error = e.message ?: e.toString()
            }
        }
    }

    fun submitBackup() {
        val current = backupMode ?: return
        if (busy) return
        error = ""
        if (password.isEmpty()) {
            error = "Введите пароль"
            return
        }
        if (current is BackupFileMode.Export && password.length < MIN_EXPORT_PASSWORD) {
            error = "Пароль должен быть не короче $MIN_EXPORT_PASSWORD символов"
            return
        }
        if (current is BackupFileMode.Export && password != repeat) {
            error = "Пароли не совпадают"
            return
        }
        busy = true
        scope.launch {
            try {
                if (current is BackupFileMode.Export) {
                    val result = BelkaCore.exportAccounts(password)
                    val date = LocalDate.now().format(DateTimeFormatter.ISO_LOCAL_DATE)
                    val fileName = "belka-accounts-$date.belka"
                    pendingExportBytes = result
                    pendingExportFileName = fileName
                    exportLauncher.launch(fileName)
                } else if (current is BackupFileMode.Import) {
                    val res = BelkaCore.importAccounts(current.bytes, password)
                    if (res.added > 0) onImported()
                    info = "Добавлено: ${accountsText(res.added)}" +
                        if (res.skipped > 0) ", уже были: ${res.skipped}" else ""
                    backupMode = null
                    password = ""
                    repeat = ""
                    busy = false
                }
            } catch (e: Exception) {
                error = e.message ?: e.toString()
                busy = false
            }
        }
    }

    Dialog(
        onDismissRequest = { if (!busy) onDismiss() },
        properties = DialogProperties(usePlatformDefaultWidth = false),
    ) {
        Surface(
            modifier = Modifier
                .fillMaxWidth()
                .padding(8.dp)
                .widthIn(max = 560.dp),
            shape = RoundedCornerShape(16.dp),
            color = colors.surface,
            border = BorderStroke(1.dp, colors.border),
        ) {
            Column(
                modifier = Modifier
                    .verticalScroll(rememberScrollState())
                    .padding(horizontal = 20.dp, vertical = 16.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(
                        text = "Аккаунты",
                        color = colors.text,
                        fontSize = 18.sp,
                        fontWeight = FontWeight.SemiBold,
                        modifier = Modifier.weight(1f),
                    )
                    BelkaToolButton(
                        icon = BelkaIcon.CLOSE,
                        contentDescription = "Закрыть",
                        enabled = !busy,
                        size = 36.dp,
                        iconSize = 18.dp,
                        onClick = onDismiss,
                    )
                }

                if (shown.isNotEmpty()) {
                    LazyColumn(
                        state = listState,
                        modifier = Modifier
                            .fillMaxWidth()
                            .heightIn(max = 360.dp)
                            .clip(RoundedCornerShape(8.dp))
                            .border(1.dp, colors.border, RoundedCornerShape(8.dp)),
                    ) {
                        itemsIndexed(shown, key = { _, a -> a.id }) { index, acc ->
                            val isDragged = draggedId == acc.id
                            val desc = describeAccount(acc, accounts)
                            Row(
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .zIndex(if (isDragged) 1f else 0f)
                                    .offset { IntOffset(0, if (isDragged) dragOffsetY.roundToInt() else 0) }
                                    .then(
                                        if (isDragged) Modifier.shadow(6.dp) else Modifier,
                                    )
                                    .background(if (isDragged) colors.selected else colors.surface)
                                    .pointerInput(accounts, busy) {
                                        if (busy || accounts.size < 2) return@pointerInput
                                        detectDragGesturesAfterLongPress(
                                            onDragStart = {
                                                haptic.performHapticFeedback(HapticFeedbackType.LongPress)
                                                draggedId = acc.id
                                                dragOrder = accounts
                                                dragOffsetY = 0f
                                            },
                                            onDrag = { change, dragAmount ->
                                                change.consume()
                                                dragOffsetY += dragAmount.y
                                                val currentId = draggedId ?: return@detectDragGesturesAfterLongPress
                                                val curIdx = dragOrder.indexOfFirst { it.id == currentId }
                                                if (curIdx < 0) return@detectDragGesturesAfterLongPress
                                                val rowInfo = listState.layoutInfo.visibleItemsInfo
                                                    .firstOrNull { it.key == currentId }
                                                val stepPx = (rowInfo?.size ?: 150).toFloat()
                                                if (dragOffsetY > stepPx * 0.55f && curIdx < dragOrder.lastIndex) {
                                                    val mutable = dragOrder.toMutableList()
                                                    val item = mutable.removeAt(curIdx)
                                                    mutable.add(curIdx + 1, item)
                                                    dragOrder = mutable
                                                    dragOffsetY -= stepPx
                                                } else if (dragOffsetY < -stepPx * 0.55f && curIdx > 0) {
                                                    val mutable = dragOrder.toMutableList()
                                                    val item = mutable.removeAt(curIdx)
                                                    mutable.add(curIdx - 1, item)
                                                    dragOrder = mutable
                                                    dragOffsetY += stepPx
                                                }
                                            },
                                            onDragEnd = {
                                                val ids = dragOrder.map { it.id }
                                                draggedId = null
                                                dragOffsetY = 0f
                                                if (ids != accounts.map { it.id }) {
                                                    scope.launch {
                                                        error = ""
                                                        try {
                                                            onReorder(ids)
                                                        } catch (e: Exception) {
                                                            error = e.message ?: e.toString()
                                                        }
                                                    }
                                                }
                                            },
                                            onDragCancel = {
                                                draggedId = null
                                                dragOffsetY = 0f
                                                dragOrder = accounts
                                            },
                                        )
                                    }
                                    .padding(horizontal = 10.dp, vertical = 8.dp),
                                verticalAlignment = Alignment.CenterVertically,
                                horizontalArrangement = Arrangement.spacedBy(10.dp),
                            ) {
                                ProviderBadge(
                                    kind = acc.kind,
                                    short = providerDef(acc.kind).short,
                                    size = 28.dp,
                                    cornerRadius = 7.dp,
                                    fontSize = 11.sp,
                                )
                                Column(modifier = Modifier.weight(1f)) {
                                    Text(
                                        text = acc.name,
                                        color = colors.text,
                                        fontSize = 15.sp,
                                        maxLines = 1,
                                        overflow = TextOverflow.Ellipsis,
                                    )
                                    Text(
                                        text = desc,
                                        color = colors.muted,
                                        fontSize = 12.5.sp,
                                        maxLines = 1,
                                        overflow = TextOverflow.Ellipsis,
                                    )
                                }
                                BelkaToolButton(
                                    icon = BelkaIcon.SETTINGS,
                                    contentDescription = "Настроить",
                                    enabled = !busy,
                                    size = 40.dp,
                                    iconSize = 18.dp,
                                    tint = colors.muted,
                                    onClick = { onEdit(acc) },
                                )
                                BelkaToolButton(
                                    icon = BelkaIcon.TRASH,
                                    contentDescription = "Удалить аккаунт",
                                    enabled = !busy,
                                    size = 40.dp,
                                    iconSize = 18.dp,
                                    tint = colors.muted,
                                    onClick = { pendingDelete = acc },
                                )
                            }
                            if (index < shown.lastIndex) {
                                HorizontalDivider(color = colors.border)
                            }
                        }
                    }
                    if (accounts.size > 1) {
                        Text(
                            text = "Чтобы изменить порядок, удерживайте аккаунт и перетащите его.",
                            color = colors.muted,
                            fontSize = 12.5.sp,
                        )
                    }
                } else {
                    Text(
                        text = "Нет подключённых облаков.",
                        color = colors.muted,
                        fontSize = 14.5.sp,
                    )
                }

                val currentBackup = backupMode
                if (currentBackup != null) {
                    Column(
                        modifier = Modifier
                            .fillMaxWidth()
                            .clip(RoundedCornerShape(8.dp))
                            .border(1.dp, colors.border, RoundedCornerShape(8.dp))
                            .padding(12.dp),
                        verticalArrangement = Arrangement.spacedBy(10.dp),
                    ) {
                        if (currentBackup is BackupFileMode.Export) {
                            Text(
                                text = "Экспорт аккаунтов",
                                color = colors.text,
                                fontSize = 15.sp,
                                fontWeight = FontWeight.SemiBold,
                            )
                            Text(
                                text = "Все аккаунты вместе с паролями и ключами доступа сохраняются в один файл, зашифрованный паролем. Без пароля файл не открыть, а восстановить пароль нельзя.",
                                color = colors.muted,
                                fontSize = 12.5.sp,
                            )
                            Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                                Text("Пароль файла", color = colors.muted, fontSize = 12.sp)
                                BelkaInputField(
                                    value = password,
                                    onValueChange = { password = it },
                                    isPassword = true,
                                    enabled = !busy,
                                )
                            }
                            Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                                Text("Пароль ещё раз", color = colors.muted, fontSize = 12.sp)
                                BelkaInputField(
                                    value = repeat,
                                    onValueChange = { repeat = it },
                                    isPassword = true,
                                    enabled = !busy,
                                )
                            }
                        } else if (currentBackup is BackupFileMode.Import) {
                            Text(
                                text = "Импорт аккаунтов",
                                color = colors.text,
                                fontSize = 15.sp,
                                fontWeight = FontWeight.SemiBold,
                            )
                            Text(
                                text = buildAnnotatedString {
                                    append("Из файла ")
                                    withStyle(SpanStyle(fontWeight = FontWeight.Bold)) {
                                        append(currentBackup.name)
                                    }
                                    append(" добавятся аккаунты, которых здесь ещё нет.")
                                },
                                color = colors.muted,
                                fontSize = 12.5.sp,
                            )
                            Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                                Text("Пароль файла", color = colors.muted, fontSize = 12.sp)
                                BelkaInputField(
                                    value = password,
                                    onValueChange = { password = it },
                                    isPassword = true,
                                    enabled = !busy,
                                )
                            }
                        }
                        Row(
                            modifier = Modifier.fillMaxWidth(),
                            horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.End),
                        ) {
                            BelkaSecondaryButton(
                                enabled = !busy,
                                onClick = { backupMode = null },
                            ) {
                                Text("Отмена")
                            }
                            BelkaPrimaryButton(
                                enabled = !busy,
                                onClick = ::submitBackup,
                            ) {
                                Text(
                                    when {
                                        currentBackup is BackupFileMode.Export && busy -> "Шифрование…"
                                        currentBackup is BackupFileMode.Export -> "Сохранить в файл…"
                                        busy -> "Расшифровка…"
                                        else -> "Импортировать"
                                    },
                                )
                            }
                        }
                    }
                }

                if (error.isNotEmpty()) {
                    BelkaErrorBox(error)
                } else if (info.isNotEmpty()) {
                    Box(
                        modifier = Modifier
                            .fillMaxWidth()
                            .clip(RoundedCornerShape(8.dp))
                            .background(colors.hover)
                            .padding(horizontal = 10.dp, vertical = 8.dp),
                    ) {
                        Text(text = info, color = colors.text, fontSize = 14.sp)
                    }
                }

                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        BelkaSecondaryButton(
                            enabled = !busy,
                            modifier = Modifier.weight(1f),
                            onClick = { importLauncher.launch(arrayOf("*/*")) },
                        ) {
                            Icon(BelkaIcon.DOWNLOAD.vector, contentDescription = null, modifier = Modifier.size(16.dp))
                            Spacer(Modifier.width(6.dp))
                            Text("Импорт…")
                        }
                        BelkaSecondaryButton(
                            enabled = !busy && accounts.isNotEmpty(),
                            modifier = Modifier.weight(1f),
                            onClick = {
                                error = ""
                                info = ""
                                password = ""
                                repeat = ""
                                backupMode = BackupFileMode.Export
                            },
                        ) {
                            Icon(BelkaIcon.UPLOAD.vector, contentDescription = null, modifier = Modifier.size(16.dp))
                            Spacer(Modifier.width(6.dp))
                            Text("Экспорт…")
                        }
                    }
                    BelkaPrimaryButton(
                        enabled = !busy,
                        modifier = Modifier.fillMaxWidth(),
                        onClick = onAdd,
                    ) {
                        Icon(BelkaIcon.PLUS.vector, contentDescription = null, modifier = Modifier.size(16.dp))
                        Spacer(Modifier.width(6.dp))
                        Text("Добавить аккаунт")
                    }
                }
            }
        }
    }

    pendingDelete?.let { acc ->
        ConfirmDialog(
            state = ConfirmState(
                title = "Удаление аккаунта",
                message = "Удалить аккаунт «${acc.name}»?\n\nФайлы в облаке останутся.",
                okLabel = "Удалить",
                onConfirm = {
                    val target = acc
                    pendingDelete = null
                    scope.launch {
                        error = ""
                        try {
                            onRemove(target)
                        } catch (e: Exception) {
                            error = e.message ?: e.toString()
                        }
                    }
                },
            ),
            onDismiss = { pendingDelete = null },
        )
    }
}

private fun describeAccount(account: Account, all: List<Account>): String {
    val title = providerDef(account.kind).title
    val crypt = account.crypt ?: return title
    val base = all.firstOrNull { it.id == crypt.base }
    return "$title: ${base?.name ?: "?"} · ${crypt.root}"
}

private fun accountsText(n: Int): String = plural(n, "аккаунт", "аккаунта", "аккаунтов")
