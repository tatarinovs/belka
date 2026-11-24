package io.github.tatarinovs.belka.ui

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Checkbox
import androidx.compose.material3.CheckboxDefaults
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
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
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.tatarinovs.belka.core.Quota
import io.github.tatarinovs.belka.core.explainError
import io.github.tatarinovs.belka.core.formatBytes
import io.github.tatarinovs.belka.ui.icons.BelkaIcon
import io.github.tatarinovs.belka.ui.theme.BelkaTheme
import kotlin.math.roundToInt

@Composable
fun BelkaToolButton(
    icon: BelkaIcon,
    contentDescription: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    size: Dp = 44.dp,
    iconSize: Dp = 22.dp,
    tint: Color? = null,
) {
    val colors = BelkaTheme.colors
    IconButton(
        onClick = onClick,
        enabled = enabled,
        modifier = modifier.size(size),
    ) {
        Icon(
            imageVector = icon.vector,
            contentDescription = contentDescription,
            tint = (tint ?: colors.text).copy(alpha = if (enabled) 1f else 0.35f),
            modifier = Modifier.size(iconSize),
        )
    }
}

@Composable
fun BelkaPrimaryButton(
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    height: Dp = 42.dp,
    content: @Composable RowScope.() -> Unit,
) {
    val colors = BelkaTheme.colors
    Button(
        onClick = onClick,
        enabled = enabled,
        shape = CircleShape,
        colors = ButtonDefaults.buttonColors(
            containerColor = colors.accent,
            contentColor = colors.accentText,
            disabledContainerColor = colors.accent.copy(alpha = 0.5f),
            disabledContentColor = colors.accentText.copy(alpha = 0.7f),
        ),
        contentPadding = PaddingValues(horizontal = 16.dp, vertical = 0.dp),
        modifier = modifier.height(height),
        content = content,
    )
}

@Composable
fun BelkaSecondaryButton(
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    danger: Boolean = false,
    height: Dp = 42.dp,
    content: @Composable RowScope.() -> Unit,
) {
    val colors = BelkaTheme.colors
    OutlinedButton(
        onClick = onClick,
        enabled = enabled,
        shape = CircleShape,
        border = BorderStroke(1.dp, colors.border),
        colors = ButtonDefaults.outlinedButtonColors(
            containerColor = colors.surface,
            contentColor = if (danger) colors.danger else colors.text,
            disabledContentColor = (if (danger) colors.danger else colors.text).copy(alpha = 0.45f),
        ),
        contentPadding = PaddingValues(horizontal = 16.dp, vertical = 0.dp),
        modifier = modifier.height(height),
        content = content,
    )
}

@Composable
fun BelkaInputField(
    value: String,
    onValueChange: (String) -> Unit,
    modifier: Modifier = Modifier,
    placeholder: String = "",
    enabled: Boolean = true,
    isPassword: Boolean = false,
    keyboardType: KeyboardType = KeyboardType.Text,
    keyboardActions: KeyboardActions = KeyboardActions.Default,
    singleLine: Boolean = true,
) {
    val colors = BelkaTheme.colors
    var focused by remember { mutableStateOf(false) }
    var visible by remember { mutableStateOf(false) }

    val transformation = if (isPassword && !visible) {
        PasswordVisualTransformation()
    } else {
        VisualTransformation.None
    }

    Row(
        modifier = modifier
            .fillMaxWidth()
            .heightIn(min = 42.dp)
            .clip(RoundedCornerShape(8.dp))
            .background(colors.bg)
            .border(
                width = 1.dp,
                color = if (focused) colors.accent else colors.border,
                shape = RoundedCornerShape(8.dp),
            )
            .padding(start = 12.dp, end = if (isPassword) 4.dp else 12.dp, top = 8.dp, bottom = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(modifier = Modifier.weight(1f)) {
            if (value.isEmpty() && placeholder.isNotEmpty()) {
                Text(
                    text = placeholder,
                    color = colors.muted,
                    fontSize = 15.sp,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            BasicTextField(
                value = value,
                onValueChange = onValueChange,
                enabled = enabled,
                singleLine = singleLine,
                visualTransformation = transformation,
                keyboardOptions = KeyboardOptions(
                    keyboardType = if (isPassword) KeyboardType.Password else keyboardType,
                    autoCorrectEnabled = false,
                ),
                keyboardActions = keyboardActions,
                textStyle = TextStyle(
                    color = if (enabled) colors.text else colors.muted,
                    fontSize = 15.sp,
                ),
                cursorBrush = SolidColor(colors.accent),
                modifier = Modifier
                    .fillMaxWidth()
                    .onFocusChanged { focused = it.isFocused },
            )
        }
        if (isPassword) {
            IconButton(
                onClick = { visible = !visible },
                enabled = enabled,
                modifier = Modifier.size(32.dp),
            ) {
                Icon(
                    imageVector = (if (visible) BelkaIcon.EYE_OFF else BelkaIcon.EYE).vector,
                    contentDescription = if (visible) "Скрыть" else "Показать",
                    tint = colors.muted,
                    modifier = Modifier.size(18.dp),
                )
            }
        }
    }
}

@Composable
fun <T> BelkaDropdown(
    label: String,
    selectedText: String,
    options: List<Pair<T, String>>,
    onSelect: (T) -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
) {
    val colors = BelkaTheme.colors
    var expanded by remember { mutableStateOf(false) }
    Column(modifier = modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Text(text = label, color = colors.muted, fontSize = 12.5.sp)
        Box {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .heightIn(min = 42.dp)
                    .clip(RoundedCornerShape(8.dp))
                    .background(colors.bg)
                    .border(1.dp, colors.border, RoundedCornerShape(8.dp))
                    .clickable(enabled = enabled) { expanded = true }
                    .padding(horizontal = 12.dp, vertical = 10.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    text = selectedText,
                    color = if (enabled) colors.text else colors.muted,
                    fontSize = 15.sp,
                    modifier = Modifier.weight(1f),
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Icon(
                    imageVector = BelkaIcon.CHEVRON_DOWN.vector,
                    contentDescription = null,
                    tint = colors.muted,
                    modifier = Modifier.size(16.dp),
                )
            }
            DropdownMenu(
                expanded = expanded,
                onDismissRequest = { expanded = false },
                containerColor = colors.surface,
            ) {
                options.forEach { (item, title) ->
                    DropdownMenuItem(
                        text = { Text(title, color = colors.text, fontSize = 15.sp) },
                        onClick = {
                            expanded = false
                            onSelect(item)
                        },
                    )
                }
            }
        }
    }
}

@Composable
fun BelkaCheckboxRow(
    checked: Boolean,
    onCheckedChange: (Boolean) -> Unit,
    label: String,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
) {
    val colors = BelkaTheme.colors
    Row(
        modifier = modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(8.dp))
            .clickable(
                enabled = enabled,
                interactionSource = remember { MutableInteractionSource() },
                indication = null,
            ) { onCheckedChange(!checked) }
            .padding(vertical = 2.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Checkbox(
            checked = checked,
            onCheckedChange = if (enabled) onCheckedChange else null,
            enabled = enabled,
            colors = CheckboxDefaults.colors(
                checkedColor = colors.accent,
                checkmarkColor = colors.accentText,
                uncheckedColor = colors.muted,
            ),
        )
        Text(
            text = label,
            color = if (enabled) colors.text else colors.muted,
            fontSize = 14.5.sp,
            modifier = Modifier.weight(1f),
        )
    }
}

@Composable
fun BelkaErrorBox(
    rawError: String,
    modifier: Modifier = Modifier,
) {
    val colors = BelkaTheme.colors
    val explained = remember(rawError) { explainError(rawError) }
    var showDetail by remember(rawError) { mutableStateOf(false) }

    Column(
        modifier = modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(8.dp))
            .background(colors.dangerBg)
            .padding(horizontal = 12.dp, vertical = 10.dp),
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        Text(
            text = explained.summary,
            color = colors.danger,
            fontSize = 13.5.sp,
        )
        if (!explained.detail.isNullOrEmpty()) {
            Text(
                text = if (showDetail) "Скрыть подробности" else "Подробнее",
                color = colors.danger.copy(alpha = 0.85f),
                fontSize = 12.sp,
                fontWeight = FontWeight.Medium,
                modifier = Modifier.clickable { showDetail = !showDetail },
            )
            AnimatedVisibility(visible = showDetail) {
                Text(
                    text = explained.detail,
                    color = colors.danger.copy(alpha = 0.9f),
                    fontSize = 12.sp,
                )
            }
        }
    }
}

@Composable
fun QuotaMeterBar(
    quota: Quota?,
    modifier: Modifier = Modifier,
) {
    if (quota == null) return
    val colors = BelkaTheme.colors
    val total = quota.total
    val used = quota.used
    val free = quota.free
    val ratio = if (total != null && total > 0 && used != null) {
        (used.toFloat() / total.toFloat()).coerceIn(0f, 1f)
    } else {
        null
    }
    val low = ratio != null && ratio > 0.9f
    val text = when {
        free != null && total != null -> "Свободно ${formatBytes(free)} из ${formatBytes(total)}"
        free != null -> "Свободно ${formatBytes(free)}"
        used != null -> "Занято ${formatBytes(used)}"
        else -> ""
    }
    if (text.isEmpty()) return

    Row(
        modifier = modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        if (ratio != null) {
            Box(
                modifier = Modifier
                    .width(64.dp)
                    .height(6.dp)
                    .clip(RoundedCornerShape(3.dp))
                    .background(colors.border),
            ) {
                Box(
                    modifier = Modifier
                        .fillMaxWidth(ratio)
                        .height(6.dp)
                        .clip(RoundedCornerShape(3.dp))
                        .background(if (low) colors.danger else colors.accent),
                )
            }
        }
        Text(
            text = text,
            color = if (low) colors.danger else colors.muted,
            fontSize = 13.sp,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
            modifier = Modifier.weight(1f),
        )
        if (ratio != null && used != null && total != null) {
            Text(
                text = "${(ratio * 100).roundToInt()}%",
                color = if (low) colors.danger else colors.muted,
                fontSize = 12.sp,
            )
        }
    }
}

@Composable
fun ToastBanner(
    toast: ToastMessage,
    onDismiss: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val colors = BelkaTheme.colors
    val bg = if (toast.isError) colors.danger else colors.text
    val fg = if (toast.isError) Color.White else colors.surface

    Row(
        modifier = modifier
            .fillMaxWidth()
            .padding(horizontal = 12.dp, vertical = 8.dp)
            .clip(RoundedCornerShape(10.dp))
            .background(bg)
            .padding(start = 14.dp, end = 4.dp, top = 6.dp, bottom = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Text(
            text = toast.text,
            color = fg,
            fontSize = 14.sp,
            modifier = Modifier.weight(1f),
        )
        IconButton(onClick = onDismiss, modifier = Modifier.size(36.dp)) {
            Icon(
                imageVector = BelkaIcon.CLOSE.vector,
                contentDescription = "Скрыть",
                tint = fg,
                modifier = Modifier.size(16.dp),
            )
        }
    }
}
