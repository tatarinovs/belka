package io.github.tatarinovs.belka.ui

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.material3.DatePicker
import androidx.compose.material3.DatePickerDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.SelectableDates
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberDatePickerState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import io.github.tatarinovs.belka.core.Entry
import io.github.tatarinovs.belka.core.ShareOptions
import io.github.tatarinovs.belka.ui.theme.BelkaTheme
import java.time.Instant
import java.time.LocalDate
import java.time.ZoneOffset
import java.time.format.DateTimeFormatter
import kotlinx.coroutines.launch

@Composable
fun PromptDialog(
    state: PromptState,
) {
    val colors = BelkaTheme.colors
    val focusRequester = remember { FocusRequester() }
    val initialRange = remember(state) {
        if (state.selectFileStem) {
            val dot = state.initialValue.lastIndexOf('.')
            val end = if (dot > 0) dot else state.initialValue.length
            TextRange(0, end)
        } else {
            TextRange(0, state.initialValue.length)
        }
    }
    var field by remember(state) {
        mutableStateOf(TextFieldValue(state.initialValue, selection = initialRange))
    }

    LaunchedEffect(state) {
        focusRequester.requestFocus()
    }

    Dialog(
        onDismissRequest = { state.onResult(null) },
        properties = DialogProperties(usePlatformDefaultWidth = false),
    ) {
        Surface(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp)
                .widthIn(max = 400.dp),
            shape = RoundedCornerShape(16.dp),
            color = colors.surface,
            border = BorderStroke(1.dp, colors.border),
        ) {
            Column(
                modifier = Modifier.padding(20.dp),
                verticalArrangement = Arrangement.spacedBy(16.dp),
            ) {
                Text(
                    text = state.title,
                    color = colors.text,
                    fontSize = 18.sp,
                    fontWeight = FontWeight.SemiBold,
                )
                androidx.compose.foundation.text.BasicTextField(
                    value = field,
                    onValueChange = { field = it },
                    singleLine = true,
                    textStyle = androidx.compose.ui.text.TextStyle(color = colors.text, fontSize = 16.sp),
                    cursorBrush = androidx.compose.ui.graphics.SolidColor(colors.accent),
                    keyboardActions = KeyboardActions(
                        onDone = {
                            if (field.text.trim().isNotEmpty()) state.onResult(field.text)
                        },
                    ),
                    modifier = Modifier
                        .fillMaxWidth()
                        .heightIn(min = 44.dp)
                        .clip(RoundedCornerShape(8.dp))
                        .background(colors.surface)
                        .border(1.dp, colors.accent, RoundedCornerShape(8.dp))
                        .padding(horizontal = 12.dp, vertical = 10.dp)
                        .focusRequester(focusRequester),
                )
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.End,
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    BelkaSecondaryButton(onClick = { state.onResult(null) }) {
                        Text("Отмена")
                    }
                    Spacer(Modifier.padding(horizontal = 4.dp))
                    BelkaPrimaryButton(
                        enabled = field.text.trim().isNotEmpty(),
                        onClick = { state.onResult(field.text) },
                    ) {
                        Text(state.okLabel)
                    }
                }
            }
        }
    }
}

@Composable
fun ConfirmDialog(
    state: ConfirmState,
    onDismiss: () -> Unit,
) {
    val colors = BelkaTheme.colors
    Dialog(
        onDismissRequest = onDismiss,
        properties = DialogProperties(usePlatformDefaultWidth = false),
    ) {
        Surface(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp)
                .widthIn(max = 420.dp),
            shape = RoundedCornerShape(16.dp),
            color = colors.surface,
            border = BorderStroke(1.dp, colors.border),
        ) {
            Column(
                modifier = Modifier.padding(20.dp),
                verticalArrangement = Arrangement.spacedBy(14.dp),
            ) {
                Text(
                    text = state.title,
                    color = colors.text,
                    fontSize = 18.sp,
                    fontWeight = FontWeight.SemiBold,
                )
                Text(
                    text = state.message,
                    color = colors.text,
                    fontSize = 14.5.sp,
                )
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.End),
                ) {
                    BelkaSecondaryButton(onClick = onDismiss) {
                        Text(state.cancelLabel)
                    }
                    BelkaPrimaryButton(onClick = state.onConfirm) {
                        Text(state.okLabel)
                    }
                }
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ShareDialog(
    entries: List<Entry>,
    onDismiss: () -> Unit,
    onShare: suspend (ShareOptions) -> Unit,
) {
    val colors = BelkaTheme.colors
    val scope = rememberCoroutineScope()
    var password by remember(entries) { mutableStateOf("") }
    var expires by remember(entries) { mutableStateOf("") }
    var busy by remember(entries) { mutableStateOf(false) }
    var error by remember(entries) { mutableStateOf("") }
    var pickingDate by remember { mutableStateOf(false) }

    val subject = if (entries.size == 1) "«${entries[0].name}»" else "выбранных объектов (${entries.size})"

    Dialog(
        onDismissRequest = { if (!busy) onDismiss() },
        properties = DialogProperties(usePlatformDefaultWidth = false),
    ) {
        Surface(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 12.dp)
                .widthIn(max = 420.dp),
            shape = RoundedCornerShape(16.dp),
            color = colors.surface,
            border = BorderStroke(1.dp, colors.border),
        ) {
            Column(
                modifier = Modifier.padding(20.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Text(
                    text = "Ссылка для $subject",
                    color = colors.text,
                    fontSize = 18.sp,
                    fontWeight = FontWeight.SemiBold,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Text("Пароль", color = colors.muted, fontSize = 12.5.sp)
                    BelkaInputField(
                        value = password,
                        onValueChange = { password = it },
                        placeholder = "Без пароля",
                        enabled = !busy,
                    )
                }
                Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Text("Действует по", color = colors.muted, fontSize = 12.5.sp)
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .heightIn(min = 42.dp)
                            .clip(RoundedCornerShape(8.dp))
                            .background(colors.bg)
                            .border(1.dp, colors.border, RoundedCornerShape(8.dp))
                            .clickable(enabled = !busy) { pickingDate = true }
                            .padding(horizontal = 12.dp, vertical = 10.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Text(
                            text = expires.ifEmpty { "Бессрочно" },
                            color = if (expires.isEmpty()) colors.muted else colors.text,
                            fontSize = 15.sp,
                            modifier = Modifier.weight(1f),
                        )
                        if (expires.isNotEmpty()) {
                            Text(
                                text = "Сбросить",
                                color = colors.accent,
                                fontSize = 13.sp,
                                modifier = Modifier.clickable(enabled = !busy) { expires = "" },
                            )
                        }
                    }
                }
                Text(
                    text = "Пустые поля — без пароля и бессрочно. Если ссылка уже есть, она будет заменена новой: старая перестанет открываться.",
                    color = colors.muted,
                    fontSize = 12.5.sp,
                )
                if (error.isNotEmpty()) {
                    BelkaErrorBox(error)
                }
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.End),
                ) {
                    BelkaSecondaryButton(enabled = !busy, onClick = onDismiss) {
                        Text("Отмена")
                    }
                    BelkaPrimaryButton(
                        enabled = !busy,
                        onClick = {
                            scope.launch {
                                busy = true
                                error = ""
                                try {
                                    onShare(ShareOptions(password = password, expires = expires))
                                } catch (e: Exception) {
                                    error = e.message ?: e.toString()
                                } finally {
                                    busy = false
                                }
                            }
                        },
                    ) {
                        Text(if (busy) "Создание…" else "Создать ссылку")
                    }
                }
            }
        }
    }

    if (pickingDate) {
        val todayMillis = remember {
            LocalDate.now().atStartOfDay(ZoneOffset.UTC).toInstant().toEpochMilli()
        }
        val state = rememberDatePickerState(
            selectableDates = object : SelectableDates {
                override fun isSelectableDate(utcTimeMillis: Long): Boolean = utcTimeMillis >= todayMillis
            },
        )
        DatePickerDialog(
            onDismissRequest = { pickingDate = false },
            confirmButton = {
                TextButton(onClick = {
                    val millis = state.selectedDateMillis
                    if (millis != null) {
                        expires = Instant.ofEpochMilli(millis)
                            .atZone(ZoneOffset.UTC)
                            .toLocalDate()
                            .format(DateTimeFormatter.ISO_LOCAL_DATE)
                    }
                    pickingDate = false
                }) {
                    Text("Выбрать")
                }
            },
            dismissButton = {
                TextButton(onClick = { pickingDate = false }) {
                    Text("Отмена")
                }
            },
        ) {
            DatePicker(state = state)
        }
    }
}
