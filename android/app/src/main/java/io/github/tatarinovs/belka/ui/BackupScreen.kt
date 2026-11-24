package io.github.tatarinovs.belka.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.Switch
import androidx.compose.material3.SwitchDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.tatarinovs.belka.core.Account
import io.github.tatarinovs.belka.core.BackupRun
import io.github.tatarinovs.belka.core.BackupTask
import io.github.tatarinovs.belka.core.formatBytes
import io.github.tatarinovs.belka.core.formatDate
import io.github.tatarinovs.belka.core.plural
import io.github.tatarinovs.belka.core.providerDef
import io.github.tatarinovs.belka.ui.icons.BelkaIcon
import io.github.tatarinovs.belka.ui.theme.BelkaTheme
import io.github.tatarinovs.belka.ui.theme.ProviderBadge
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

private val INTERVALS = listOf(
    1 to "Каждый час",
    3 to "Каждые 3 часа",
    6 to "Каждые 6 часов",
    12 to "Каждые 12 часов",
    24 to "Раз в сутки",
    72 to "Раз в 3 дня",
    168 to "Раз в неделю",
)

private val FREE_AFTER = listOf(
    Triple(7, "через неделю", "недели"),
    Triple(30, "через месяц", "месяца"),
    Triple(90, "через 3 месяца", "3 месяцев"),
    Triple(365, "через год", "года"),
)

private val PHONE_ROOT = Regex("^/storage/emulated/0(/|$)")

@Composable
fun BackupScreen(
    accounts: List<Account>,
    storageOk: Boolean,
    tasks: List<BackupTask>,
    loaded: Boolean,
    draft: BackupTask?,
    onDraftChange: (BackupTask?) -> Unit,
    onClose: () -> Unit,
    onPickFolder: (String) -> Unit,
    onRequestStorage: () -> Unit,
    onRefresh: () -> Unit,
    onRunNow: (BackupTask) -> Unit,
    onCancelRun: (BackupTask) -> Unit,
    onToggleEnabled: (BackupTask) -> Unit,
    onSave: suspend (BackupTask) -> BackupTask,
    onRemove: suspend (String) -> Unit,
    onToast: (String, Boolean) -> Unit,
) {
    val colors = BelkaTheme.colors

    LaunchedEffect(Unit) {
        while (true) {
            onRefresh()
            delay(if (tasks.any { it.running != null }) 1000L else 4000L)
        }
    }

    val editingSaved = !draft?.id.isNullOrEmpty()

    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(colors.surface),
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .height(56.dp)
                .padding(horizontal = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            BelkaToolButton(
                icon = BelkaIcon.ARROW_LEFT,
                contentDescription = "Назад",
                onClick = { if (draft != null) onDraftChange(null) else onClose() },
            )
            Text(
                text = when {
                    draft != null && editingSaved -> "Задача автобэкапа"
                    draft != null -> "Новая задача"
                    else -> "Автобэкап"
                },
                color = colors.text,
                fontSize = 19.sp,
                fontWeight = FontWeight.SemiBold,
                modifier = Modifier.weight(1f),
            )
            if (draft == null) {
                BelkaToolButton(
                    icon = BelkaIcon.PLUS,
                    contentDescription = "Новая задача",
                    enabled = accounts.isNotEmpty(),
                    onClick = {
                        onDraftChange(
                            BackupTask(
                                account_id = accounts.firstOrNull()?.id.orEmpty(),
                            ),
                        )
                    },
                )
            }
        }
        HorizontalDivider(color = colors.border)

        if (!storageOk) {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 12.dp, vertical = 8.dp)
                    .clip(RoundedCornerShape(10.dp))
                    .background(colors.selected)
                    .padding(horizontal = 12.dp, vertical = 10.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                Text(
                    text = "Чтобы читать папки телефона в фоне, нужен доступ ко всем файлам.",
                    color = colors.text,
                    fontSize = 14.sp,
                    modifier = Modifier.weight(1f),
                )
                BelkaPrimaryButton(height = 38.dp, onClick = onRequestStorage) {
                    Text("Разрешить", fontSize = 13.5.sp)
                }
            }
        }

        if (draft != null) {
            BackupEditor(
                draft = draft,
                accounts = accounts,
                savedHistory = tasks.firstOrNull { it.id == draft.id },
                onDraftChange = { onDraftChange(it) },
                onCancel = { onDraftChange(null) },
                onPickFolder = onPickFolder,
                onSave = onSave,
                onRemove = onRemove,
                onToast = onToast,
            )
        } else {
            Box(modifier = Modifier.weight(1f).fillMaxWidth()) {
                if (tasks.isEmpty() && loaded) {
                    Column(
                        modifier = Modifier
                            .fillMaxSize()
                            .padding(horizontal = 24.dp, vertical = 48.dp),
                        horizontalAlignment = Alignment.CenterHorizontally,
                        verticalArrangement = Arrangement.spacedBy(12.dp),
                    ) {
                        Icon(
                            imageVector = BelkaIcon.CLOCK.vector,
                            contentDescription = null,
                            tint = colors.muted,
                            modifier = Modifier.size(48.dp),
                        )
                        Text(
                            text = "Автобэкап",
                            color = colors.text,
                            fontSize = 20.sp,
                            fontWeight = FontWeight.SemiBold,
                        )
                        Text(
                            text = "Выберите папку на телефоне, например с фотографиями, и папку в облаке. Belka будет регулярно загружать туда новые файлы, даже когда приложение закрыто.",
                            color = colors.muted,
                            fontSize = 15.sp,
                            textAlign = TextAlign.Center,
                        )
                        Spacer(Modifier.height(4.dp))
                        if (accounts.isNotEmpty()) {
                            BelkaPrimaryButton(
                                height = 44.dp,
                                onClick = {
                                    onDraftChange(BackupTask(account_id = accounts.first().id))
                                },
                            ) {
                                Text("Новая задача")
                            }
                        } else {
                            Text(
                                text = "Сначала подключите облако.",
                                color = colors.muted,
                                fontSize = 15.sp,
                            )
                        }
                    }
                } else {
                    LazyColumn(modifier = Modifier.fillMaxSize()) {
                        items(tasks, key = { it.id }) { task ->
                            BackupTaskRow(
                                task = task,
                                account = accounts.firstOrNull { it.id == task.account_id },
                                onEdit = { onDraftChange(task) },
                                onRun = { onRunNow(task) },
                                onStop = { onCancelRun(task) },
                                onToggle = { onToggleEnabled(task) },
                            )
                            HorizontalDivider(color = colors.border)
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun BackupTaskRow(
    task: BackupTask,
    account: Account?,
    onEdit: () -> Unit,
    onRun: () -> Unit,
    onStop: () -> Unit,
    onToggle: () -> Unit,
) {
    val colors = BelkaTheme.colors
    val status = taskStatus(task)

    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onEdit)
            .padding(start = 16.dp, end = 8.dp, top = 12.dp, bottom = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Column(
            modifier = Modifier
                .weight(1f)
                .alpha(if (task.enabled) 1f else 0.55f),
            verticalArrangement = Arrangement.spacedBy(3.dp),
        ) {
            Text(
                text = task.name,
                color = colors.text,
                fontSize = 15.sp,
                fontWeight = FontWeight.SemiBold,
            )
            Text(
                text = "${localLabel(task.local)} → ${if (account != null) "${account.name}: ${task.remote}" else "аккаунт удалён"}",
                color = colors.muted,
                fontSize = 12.5.sp,
            )
            Text(
                text = if (task.enabled) taskConditions(task) else "Выключена",
                color = colors.muted,
                fontSize = 12.5.sp,
            )
            val r = task.running
            if (r != null) {
                Text(
                    text = if (r.scanning) {
                        "Поиск новых файлов…"
                    } else {
                        "${r.files_done} из ${r.files_total} · ${formatBytes(r.bytes_done)} из ${formatBytes(r.bytes_total)}" +
                            if (r.current.isNotEmpty()) " · ${r.current}" else ""
                    },
                    color = colors.muted,
                    fontSize = 12.5.sp,
                )
                if (r.scanning || r.bytes_total <= 0L) {
                    LinearProgressIndicator(
                        color = colors.accent,
                        trackColor = colors.border,
                        modifier = Modifier
                            .fillMaxWidth()
                            .padding(top = 3.dp)
                            .height(4.dp)
                            .clip(RoundedCornerShape(2.dp)),
                    )
                } else {
                    LinearProgressIndicator(
                        progress = { (r.bytes_done.toFloat() / r.bytes_total.toFloat()).coerceIn(0f, 1f) },
                        color = colors.accent,
                        trackColor = colors.border,
                        modifier = Modifier
                            .fillMaxWidth()
                            .padding(top = 3.dp)
                            .height(4.dp)
                            .clip(RoundedCornerShape(2.dp)),
                    )
                }
            } else {
                Text(
                    text = status.first,
                    color = if (status.second) colors.danger else colors.muted,
                    fontSize = 12.5.sp,
                )
            }
        }
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(2.dp),
        ) {
            if (task.running != null) {
                BelkaToolButton(
                    icon = BelkaIcon.STOP,
                    contentDescription = "Остановить",
                    iconSize = 20.dp,
                    onClick = onStop,
                )
            } else {
                BelkaToolButton(
                    icon = BelkaIcon.PLAY,
                    contentDescription = "Запустить сейчас",
                    iconSize = 20.dp,
                    enabled = account != null,
                    onClick = onRun,
                )
            }
            Switch(
                checked = task.enabled,
                onCheckedChange = { onToggle() },
                colors = SwitchDefaults.colors(
                    checkedThumbColor = Color.White,
                    checkedTrackColor = colors.accent,
                    uncheckedThumbColor = Color.White,
                    uncheckedTrackColor = colors.border,
                    uncheckedBorderColor = Color.Transparent,
                ),
            )
        }
    }
}

@Composable
private fun BackupEditor(
    draft: BackupTask,
    accounts: List<Account>,
    savedHistory: BackupTask?,
    onDraftChange: (BackupTask) -> Unit,
    onCancel: () -> Unit,
    onPickFolder: (String) -> Unit,
    onSave: suspend (BackupTask) -> BackupTask,
    onRemove: suspend (String) -> Unit,
    onToast: (String, Boolean) -> Unit,
) {
    val colors = BelkaTheme.colors
    val scope = rememberCoroutineScope()
    val editingSaved = draft.id.isNotEmpty()
    val acc = accounts.firstOrNull { it.id == draft.account_id }

    var includeText by remember(draft.id) { mutableStateOf(draft.include.joinToString(", ")) }
    var excludeText by remember(draft.id) { mutableStateOf(draft.exclude.joinToString(", ")) }
    var saving by remember(draft.id) { mutableStateOf(false) }
    var error by remember(draft.id) { mutableStateOf("") }
    var openRunKey by remember(draft.id) { mutableStateOf<String?>(null) }
    var confirmFreeSpace by remember { mutableStateOf(false) }
    var confirmDelete by remember { mutableStateOf(false) }

    fun performSave() {
        saving = true
        error = ""
        scope.launch {
            try {
                val isNew = draft.id.isEmpty()
                val updated = draft.copy(
                    include = parsePatterns(includeText),
                    exclude = parsePatterns(excludeText),
                )
                val saved = onSave(updated)
                if (isNew && saved.enabled) {
                    onToast(
                        if (saved.wifi_only) {
                            "Задача сохранена. Первый бэкап начнётся при подключении к Wi‑Fi"
                        } else {
                            "Задача сохранена. Первый бэкап начнётся в ближайшее время"
                        },
                        false,
                    )
                }
            } catch (e: Exception) {
                error = e.message ?: e.toString()
            } finally {
                saving = false
            }
        }
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Text("Название", color = colors.muted, fontSize = 12.5.sp)
            BelkaInputField(
                value = draft.name,
                onValueChange = { onDraftChange(draft.copy(name = it)) },
                placeholder = "По имени папки",
            )
        }

        Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Text("Что копировать", color = colors.muted, fontSize = 12.5.sp)
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .clip(RoundedCornerShape(8.dp))
                    .background(colors.bg)
                    .border(1.dp, colors.border, RoundedCornerShape(8.dp))
                    .clickable { onPickFolder("local") }
                    .padding(horizontal = 12.dp, vertical = 12.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                Icon(BelkaIcon.PHONE.vector, contentDescription = null, tint = colors.muted, modifier = Modifier.size(20.dp))
                Text(
                    text = if (draft.local.isNotEmpty()) localLabel(draft.local) else "Выбрать папку на телефоне",
                    color = colors.text,
                    fontSize = 15.sp,
                    modifier = Modifier.weight(1f),
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Icon(BelkaIcon.CHEVRON.vector, contentDescription = null, tint = colors.muted, modifier = Modifier.size(18.dp))
            }
        }

        Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Text("Куда", color = colors.muted, fontSize = 12.5.sp)
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .clip(RoundedCornerShape(8.dp))
                    .background(colors.bg)
                    .border(1.dp, colors.border, RoundedCornerShape(8.dp))
                    .clickable { onPickFolder("remote") }
                    .padding(horizontal = 12.dp, vertical = 12.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                if (acc != null && draft.remote.isNotEmpty()) {
                    ProviderBadge(
                        kind = acc.kind,
                        short = providerDef(acc.kind).short,
                        size = 22.dp,
                    )
                    Text(
                        text = "${acc.name}: ${draft.remote}",
                        color = colors.text,
                        fontSize = 15.sp,
                        modifier = Modifier.weight(1f),
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                } else {
                    Icon(BelkaIcon.CLOUD.vector, contentDescription = null, tint = colors.muted, modifier = Modifier.size(20.dp))
                    Text(
                        text = "Выбрать папку в облаке",
                        color = colors.text,
                        fontSize = 15.sp,
                        modifier = Modifier.weight(1f),
                    )
                }
                Icon(BelkaIcon.CHEVRON.vector, contentDescription = null, tint = colors.muted, modifier = Modifier.size(18.dp))
            }
        }

        BelkaDropdown(
            label = "Как часто",
            selectedText = intervalLabel(draft.interval_hours),
            options = INTERVALS,
            onSelect = { onDraftChange(draft.copy(interval_hours = it)) },
        )

        Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Text("Только эти файлы (маски через запятую, пусто — все)", color = colors.muted, fontSize = 12.5.sp)
            BelkaInputField(
                value = includeText,
                onValueChange = { includeText = it },
                placeholder = "*.jpg, *.mp4",
            )
        }

        Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Text("Пропускать (файлы и папки)", color = colors.muted, fontSize = 12.5.sp)
            BelkaInputField(
                value = excludeText,
                onValueChange = { excludeText = it },
                placeholder = "*.tmp, cache",
            )
        }

        BelkaCheckboxRow(
            checked = draft.wifi_only,
            onCheckedChange = { onDraftChange(draft.copy(wifi_only = it)) },
            label = "Только по Wi‑Fi",
        )
        BelkaCheckboxRow(
            checked = draft.charging_only,
            onCheckedChange = { onDraftChange(draft.copy(charging_only = it)) },
            label = "Только во время зарядки",
        )
        BelkaCheckboxRow(
            checked = draft.free_after_days != null,
            onCheckedChange = { checked ->
                onDraftChange(draft.copy(free_after_days = if (checked) 30 else null))
            },
            label = "Освобождать место на телефоне",
        )
        if (draft.free_after_days != null) {
            BelkaDropdown(
                label = "Удалять с телефона файлы, которые уже в облаке,",
                selectedText = FREE_AFTER.firstOrNull { it.first == draft.free_after_days }?.second
                    ?: "через ${draft.free_after_days} дн.",
                options = FREE_AFTER.map { it.first to it.second },
                onSelect = { onDraftChange(draft.copy(free_after_days = it)) },
                modifier = Modifier.padding(start = 28.dp),
            )
        }
        BelkaCheckboxRow(
            checked = draft.enabled,
            onCheckedChange = { onDraftChange(draft.copy(enabled = it)) },
            label = "Включена",
        )

        Text(
            text = "Загружаются только новые и изменённые файлы, подпапки сохраняются. В облаке ничего не удаляется, даже если удалить файл на телефоне. Скрытые файлы пропускаются, в том числе корзина галереи.",
            color = colors.muted,
            fontSize = 13.sp,
        )

        if (error.isNotEmpty()) {
            BelkaErrorBox(error)
        }

        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            if (editingSaved) {
                BelkaSecondaryButton(
                    danger = true,
                    enabled = !saving,
                    onClick = { confirmDelete = true },
                ) {
                    Text("Удалить")
                }
            }
            Spacer(Modifier.weight(1f))
            BelkaSecondaryButton(enabled = !saving, onClick = onCancel) {
                Text("Отмена")
            }
            BelkaPrimaryButton(
                enabled = !saving,
                onClick = {
                    if (draft.local.isEmpty()) {
                        error = "Выберите папку на телефоне"
                        return@BelkaPrimaryButton
                    }
                    if (draft.account_id.isEmpty() || draft.remote.trim().isEmpty()) {
                        error = "Выберите папку в облаке"
                        return@BelkaPrimaryButton
                    }
                    if (draft.free_after_days != null) {
                        confirmFreeSpace = true
                    } else {
                        performSave()
                    }
                },
            ) {
                Text("Сохранить")
            }
        }

        val history = savedHistory?.history.orEmpty()
        if (history.isNotEmpty()) {
            HorizontalDivider(color = colors.border, modifier = Modifier.padding(top = 6.dp))
            Text(
                text = "История",
                color = colors.muted,
                fontSize = 14.sp,
                fontWeight = FontWeight.SemiBold,
            )
            history.forEachIndexed { index, h ->
                val isRunning = index == 0 && savedHistory?.running != null
                val (text, isErr) = runText(h, isRunning)
                val key = h.started ?: index.toString()
                val expanded = openRunKey == key
                Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .clip(RoundedCornerShape(6.dp))
                            .clickable(enabled = h.failures.isNotEmpty()) {
                                openRunKey = if (expanded) null else key
                            }
                            .padding(vertical = 4.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(6.dp),
                    ) {
                        Text(
                            text = text,
                            color = if (isErr) colors.danger else colors.muted,
                            fontSize = 13.5.sp,
                            modifier = Modifier.weight(1f),
                        )
                        if (h.failures.isNotEmpty()) {
                            Icon(
                                imageVector = (if (expanded) BelkaIcon.CHEVRON_DOWN else BelkaIcon.CHEVRON).vector,
                                contentDescription = null,
                                tint = colors.muted,
                                modifier = Modifier.size(14.dp),
                            )
                        }
                    }
                    if (expanded) {
                        Column(
                            modifier = Modifier.padding(start = 14.dp),
                            verticalArrangement = Arrangement.spacedBy(3.dp),
                        ) {
                            h.failures.forEach { pair ->
                                val file = pair.getOrNull(0).orEmpty()
                                val reason = pair.getOrNull(1).orEmpty()
                                Text(
                                    text = buildAnnotatedString {
                                        withStyle(SpanStyle(fontWeight = FontWeight.SemiBold, color = colors.text)) {
                                            append(file)
                                        }
                                        append(": $reason")
                                    },
                                    color = colors.muted,
                                    fontSize = 12.5.sp,
                                )
                            }
                            if (h.failed > h.failures.size) {
                                Text(
                                    text = "…и ещё ${h.failed - h.failures.size}",
                                    color = colors.muted,
                                    fontSize = 12.5.sp,
                                )
                            }
                        }
                    }
                }
            }
        }
    }

    if (confirmFreeSpace) {
        val days = draft.free_after_days ?: 30
        val label = FREE_AFTER.firstOrNull { it.first == days }?.third
            ?: plural(days, "дня", "дней", "дней")
        ConfirmDialog(
            state = ConfirmState(
                title = "Освобождение места",
                message = "Файлы старше $label будут удаляться с телефона после проверки содержимого копии в облаке. Если облако не позволяет проверить содержимое, файлы останутся на телефоне.\n\nВосстановить удалённые файлы можно только из облака. Продолжить?",
                okLabel = "Да, удалять",
                onConfirm = {
                    confirmFreeSpace = false
                    performSave()
                },
            ),
            onDismiss = { confirmFreeSpace = false },
        )
    }

    if (confirmDelete) {
        ConfirmDialog(
            state = ConfirmState(
                title = "Автобэкап",
                message = "Удалить задачу «${draft.name}»?\n\nУже загруженные файлы останутся в облаке.",
                okLabel = "Удалить",
                onConfirm = {
                    confirmDelete = false
                    saving = true
                    scope.launch {
                        try {
                            onRemove(draft.id)
                        } catch (e: Exception) {
                            error = e.message ?: e.toString()
                        } finally {
                            saving = false
                        }
                    }
                },
            ),
            onDismiss = { confirmDelete = false },
        )
    }
}

private fun localLabel(path: String): String =
    path.replace(PHONE_ROOT, "/").ifEmpty { "/" }

private fun intervalLabel(hours: Int): String =
    INTERVALS.firstOrNull { it.first == hours }?.second ?: "Каждые $hours ч"

private fun taskConditions(task: BackupTask): String {
    val parts = mutableListOf(intervalLabel(task.interval_hours).lowercase())
    if (task.wifi_only) parts.add("по Wi‑Fi")
    if (task.charging_only) parts.add("на зарядке")
    if (task.include.isNotEmpty() || task.exclude.isNotEmpty()) parts.add("с фильтром")
    if (task.free_after_days != null) parts.add("освобождает место")
    return parts.joinToString(", ")
}

private fun runText(run: BackupRun, running: Boolean): Pair<String, Boolean> {
    if (run.finished == null) {
        return "${formatDate(run.started)} · ${if (running) "идёт" else "прерван"}" to false
    }
    val whenStr = formatDate(run.finished)
    if (run.cancelled) return "$whenStr · остановлен" to false
    if (!run.error.isNullOrEmpty()) return "$whenStr · ${run.error}" to true
    val parts = mutableListOf(
        if (run.uploaded > 0) {
            "загружено ${plural(run.uploaded, "файл", "файла", "файлов")} (${formatBytes(run.bytes)})"
        } else {
            "новых файлов нет"
        },
    )
    if (run.freed > 0) parts.add("освобождено ${formatBytes(run.freed_bytes)}")
    if (run.failed > 0) parts.add("не удалось: ${run.failed}")
    return "$whenStr · ${parts.joinToString(", ")}" to (run.failed > 0)
}

private fun taskStatus(task: BackupTask): Pair<String, Boolean> {
    val last = task.history.firstOrNull()
        ?: return (if (task.enabled) "Ещё не запускался" else "Выключен") to false
    return runText(last, false)
}

private fun parsePatterns(text: String): List<String> =
    text.split(',', '\n').map { it.trim() }.filter { it.isNotEmpty() }
