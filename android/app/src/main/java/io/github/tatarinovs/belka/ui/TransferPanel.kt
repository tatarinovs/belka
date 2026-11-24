package io.github.tatarinovs.belka.ui

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.tatarinovs.belka.core.Transfer
import io.github.tatarinovs.belka.core.TransferCounts
import io.github.tatarinovs.belka.core.TransferDirection
import io.github.tatarinovs.belka.core.TransferFilter
import io.github.tatarinovs.belka.core.TransferState
import io.github.tatarinovs.belka.core.explainError
import io.github.tatarinovs.belka.core.formatBytes
import io.github.tatarinovs.belka.ui.icons.BelkaIcon
import io.github.tatarinovs.belka.ui.theme.BelkaTheme
import kotlin.math.max

@OptIn(ExperimentalLayoutApi::class)
@Composable
fun TransferPanel(
    counts: TransferCounts,
    viewItems: List<Transfer>,
    filter: TransferFilter,
    speeds: Map<Long, Double>,
    waiting: Set<Long>,
    error: String,
    collapsed: Boolean,
    onToggleCollapsed: () -> Unit,
    onSelectFilter: (TransferFilter) -> Unit,
    onCancelAll: () -> Unit,
    onRetryFailed: ((String, String?) -> Unit) -> Unit,
    onClear: () -> Unit,
    onCancel: (Long) -> Unit,
    onRetry: (Long) -> Unit,
    onOpenDownloaded: (String) -> Unit,
    modifier: Modifier = Modifier,
) {
    if (counts.total == 0) return
    val colors = BelkaTheme.colors
    var message by remember { mutableStateOf("") }
    var actionError by remember { mutableStateOf("") }
    val finished = counts.total - counts.activeCount
    val shownCount = counts.forFilter(filter)
    val hidden = max(0, shownCount - viewItems.size)

    val summary = remember(counts) {
        val parts = mutableListOf<String>()
        if (counts.running > 0) parts.add("идёт ${counts.running}")
        if (counts.queued > 0) parts.add("в очереди ${counts.queued}")
        if (counts.failed > 0) parts.add("ошибок ${counts.failed}")
        if (parts.isEmpty()) parts.add("всё завершено")
        parts.joinToString(" · ")
    }

    val chevronRotation by animateFloatAsState(if (collapsed) 180f else 0f, label = "chev")

    Column(
        modifier = modifier
            .fillMaxWidth()
            .background(colors.surface),
    ) {
        HorizontalDivider(color = colors.border)
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 8.dp, vertical = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            Row(
                modifier = Modifier
                    .weight(1f)
                    .clip(RoundedCornerShape(6.dp))
                    .clickable(onClick = onToggleCollapsed)
                    .padding(horizontal = 4.dp, vertical = 6.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Icon(
                    imageVector = BelkaIcon.CHEVRON_DOWN.vector,
                    contentDescription = null,
                    tint = colors.muted,
                    modifier = Modifier
                        .size(16.dp)
                        .rotate(chevronRotation),
                )
                Text(
                    text = "Передачи",
                    color = colors.text,
                    fontWeight = FontWeight.SemiBold,
                    fontSize = 14.sp,
                )
                Text(
                    text = summary,
                    color = colors.muted,
                    fontSize = 13.sp,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            FlowRow(
                horizontalArrangement = Arrangement.spacedBy(6.dp),
                verticalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                if (counts.activeCount > 0) {
                    SmallPanelButton(text = "Отменить все", onClick = onCancelAll)
                }
                if (counts.failed > 0) {
                    SmallPanelButton(text = "Повторить (${counts.failed})") {
                        message = ""
                        actionError = ""
                        onRetryFailed { msg, err ->
                            message = msg
                            actionError = err.orEmpty()
                        }
                    }
                }
                if (finished > 0) {
                    SmallPanelButton(text = "Очистить", onClick = onClear)
                }
            }
        }

        if (actionError.isNotEmpty() || error.isNotEmpty()) {
            Text(
                text = actionError.ifEmpty { error },
                color = colors.danger,
                fontSize = 12.5.sp,
                modifier = Modifier.padding(horizontal = 12.dp, vertical = 4.dp),
            )
        }
        if (message.isNotEmpty()) {
            Text(
                text = message,
                color = colors.muted,
                fontSize = 12.5.sp,
                modifier = Modifier.padding(horizontal = 12.dp, vertical = 4.dp),
            )
        }

        AnimatedVisibility(visible = !collapsed) {
            Column(modifier = Modifier.fillMaxWidth()) {
                HorizontalDivider(color = colors.border)
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .horizontalScroll(rememberScrollState())
                        .padding(horizontal = 8.dp, vertical = 5.dp),
                    horizontalArrangement = Arrangement.spacedBy(4.dp),
                ) {
                    TransferFilter.entries.forEach { f ->
                        val chosen = filter == f
                        Row(
                            modifier = Modifier
                                .clip(RoundedCornerShape(6.dp))
                                .background(if (chosen) colors.selected else colors.surface)
                                .clickable { onSelectFilter(f) }
                                .padding(horizontal = 8.dp, vertical = 5.dp),
                            horizontalArrangement = Arrangement.spacedBy(5.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Text(
                                text = f.label,
                                color = if (chosen) colors.text else colors.muted,
                                fontSize = 13.sp,
                            )
                            Text(
                                text = counts.forFilter(f).toString(),
                                color = colors.muted,
                                fontSize = 11.5.sp,
                            )
                        }
                    }
                }
                HorizontalDivider(color = colors.border)
                LazyColumn(
                    modifier = Modifier
                        .fillMaxWidth()
                        .heightIn(max = 260.dp),
                ) {
                    if (viewItems.isEmpty()) {
                        item {
                            Text(
                                text = if (error.isNotEmpty()) {
                                    "Не удалось получить список передач"
                                } else {
                                    "В этой категории передач нет"
                                },
                                color = colors.muted,
                                fontSize = 12.5.sp,
                                modifier = Modifier.padding(horizontal = 12.dp, vertical = 10.dp),
                            )
                        }
                    } else {
                        items(viewItems, key = { it.id }) { t ->
                            TransferRow(
                                t = t,
                                speed = speeds[t.id],
                                isWaiting = waiting.contains(t.id),
                                onCancel = { onCancel(t.id) },
                                onRetry = { onRetry(t.id) },
                                onOpen = { onOpenDownloaded(t.local_path) },
                            )
                            HorizontalDivider(color = colors.border)
                        }
                        if (hidden > 0) {
                            item {
                                Text(
                                    text = "…и ещё $hidden",
                                    color = colors.muted,
                                    fontSize = 12.5.sp,
                                    modifier = Modifier.padding(horizontal = 12.dp, vertical = 8.dp),
                                )
                            }
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun SmallPanelButton(text: String, onClick: () -> Unit) {
    val colors = BelkaTheme.colors
    OutlinedButton(
        onClick = onClick,
        shape = RoundedCornerShape(14.dp),
        border = BorderStroke(1.dp, colors.border),
        colors = ButtonDefaults.outlinedButtonColors(
            containerColor = colors.surface,
            contentColor = colors.text,
        ),
        contentPadding = PaddingValues(horizontal = 10.dp, vertical = 0.dp),
        modifier = Modifier.height(28.dp),
    ) {
        Text(text = text, fontSize = 12.sp, maxLines = 1)
    }
}

@Composable
private fun TransferRow(
    t: Transfer,
    speed: Double?,
    isWaiting: Boolean,
    onCancel: () -> Unit,
    onRetry: () -> Unit,
    onOpen: () -> Unit,
) {
    val colors = BelkaTheme.colors
    val dirIcon = when (t.direction) {
        TransferDirection.UPLOAD -> BelkaIcon.UPLOAD
        TransferDirection.DOWNLOAD -> BelkaIcon.DOWNLOAD
        TransferDirection.COPY -> BelkaIcon.COPY
    }
    val pct = when {
        t.state == TransferState.DONE -> 1f
        t.total > 0 -> (t.done.toFloat() / t.total.toFloat()).coerceIn(0f, 1f)
        else -> 0f
    }
    val barColor = when (t.state) {
        TransferState.DONE -> colors.success
        TransferState.FAILED, TransferState.CANCELLED -> colors.muted
        else -> colors.accent
    }
    val detailText = when (t.state) {
        TransferState.QUEUED -> if (t.total > 0) "В очереди · ${formatBytes(t.total)}" else "В очереди"
        TransferState.RUNNING -> {
            val retry = if (t.attempt > 1) "попытка ${t.attempt} из ${t.max_attempts}" else ""
            when {
                t.done == 0L && retry.isNotEmpty() -> "Связь прервалась · $retry"
                t.done == 0L && isWaiting -> "Ждём ответа сервера…"
                else -> {
                    val parts = mutableListOf(
                        if (t.total > 0) "${formatBytes(t.done)} из ${formatBytes(t.total)}" else formatBytes(t.done),
                    )
                    if (speed != null && speed > 0.0) parts.add("${formatBytes(speed.toLong())}/с")
                    if (retry.isNotEmpty()) parts.add(retry)
                    parts.joinToString(" · ")
                }
            }
        }
        TransferState.DONE -> "${if (t.unchanged) "Без изменений" else "Готово"} · ${formatBytes(if (t.total > 0) t.total else t.done)}"
        TransferState.CANCELLED -> "Отменено"
        TransferState.FAILED -> explainError(t.error.orEmpty()).summary.ifEmpty { "Ошибка" }
    }

    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 12.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Icon(
            imageVector = dirIcon.vector,
            contentDescription = null,
            tint = colors.muted,
            modifier = Modifier.size(16.dp),
        )
        Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(3.dp)) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    text = t.name,
                    color = colors.text,
                    fontSize = 14.sp,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.weight(1f),
                )
                Text(
                    text = t.account_name,
                    color = colors.muted,
                    fontSize = 12.sp,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            Box(
                modifier = Modifier
                    .fillMaxWidth()
                    .height(4.dp)
                    .clip(RoundedCornerShape(2.dp))
                    .background(colors.hover),
            ) {
                Box(
                    modifier = Modifier
                        .fillMaxWidth(pct)
                        .height(4.dp)
                        .clip(RoundedCornerShape(2.dp))
                        .background(barColor),
                )
            }
            Text(
                text = detailText,
                color = if (t.state == TransferState.FAILED) colors.danger else colors.muted,
                fontSize = 12.sp,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
        }
        Box(modifier = Modifier.size(34.dp), contentAlignment = Alignment.Center) {
            when {
                t.state == TransferState.QUEUED || t.state == TransferState.RUNNING -> {
                    IconButton(onClick = onCancel, modifier = Modifier.size(34.dp)) {
                        Icon(BelkaIcon.CLOSE.vector, contentDescription = "Отменить", tint = colors.muted, modifier = Modifier.size(16.dp))
                    }
                }
                t.state == TransferState.FAILED || t.state == TransferState.CANCELLED -> {
                    IconButton(onClick = onRetry, modifier = Modifier.size(34.dp)) {
                        Icon(BelkaIcon.RETRY.vector, contentDescription = "Повторить", tint = colors.muted, modifier = Modifier.size(16.dp))
                    }
                }
                t.direction == TransferDirection.DOWNLOAD -> {
                    IconButton(onClick = onOpen, modifier = Modifier.size(34.dp)) {
                        Icon(BelkaIcon.FILE.vector, contentDescription = "Открыть файл", tint = colors.muted, modifier = Modifier.size(16.dp))
                    }
                }
            }
        }
    }
}
