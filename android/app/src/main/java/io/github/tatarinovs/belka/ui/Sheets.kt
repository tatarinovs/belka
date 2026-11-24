package io.github.tatarinovs.belka.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.tatarinovs.belka.core.providerDef
import io.github.tatarinovs.belka.ui.icons.BelkaIcon
import io.github.tatarinovs.belka.ui.theme.BelkaTheme
import io.github.tatarinovs.belka.ui.theme.ProviderBadge

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ActionBottomSheet(
    menu: SheetMenu,
    onDismiss: () -> Unit,
) {
    val colors = BelkaTheme.colors
    val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)

    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = sheetState,
        containerColor = colors.surface,
        dragHandle = null,
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .verticalScroll(rememberScrollState())
                .padding(top = 8.dp, bottom = 16.dp),
        ) {
            if (!menu.title.isNullOrEmpty()) {
                Text(
                    text = menu.title,
                    color = colors.muted,
                    fontSize = 13.sp,
                    modifier = Modifier.padding(horizontal = 20.dp, vertical = 10.dp),
                )
            }
            menu.items.forEach { item ->
                if (item.isSeparator) {
                    HorizontalDivider(
                        color = colors.border,
                        modifier = Modifier.padding(vertical = 6.dp),
                    )
                } else {
                    val alpha = if (item.disabled) 0.4f else 1f
                    val textColor = when {
                        item.danger -> colors.danger
                        else -> colors.text
                    }.copy(alpha = alpha)

                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .heightIn(min = 52.dp)
                            .clickable(enabled = !item.disabled) {
                                onDismiss()
                                item.action()
                            }
                            .padding(horizontal = 20.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(16.dp),
                    ) {
                        Box(modifier = Modifier.size(24.dp), contentAlignment = Alignment.Center) {
                            when {
                                item.badge != null -> {
                                    ProviderBadge(
                                        kind = item.badge.kind,
                                        short = providerDef(item.badge.kind).short,
                                        size = 22.dp,
                                        cornerRadius = 6.dp,
                                        fontSize = 11.sp,
                                    )
                                }
                                item.icon != null -> {
                                    Icon(
                                        imageVector = item.icon.vector,
                                        contentDescription = null,
                                        tint = if (item.danger) textColor else colors.muted.copy(alpha = alpha),
                                        modifier = Modifier.size(20.dp),
                                    )
                                }
                            }
                        }
                        Text(
                            text = item.label,
                            color = textColor,
                            fontSize = 15.5.sp,
                            modifier = Modifier.weight(1f),
                        )
                        if (item.checked) {
                            Icon(
                                imageVector = BelkaIcon.CHECK.vector,
                                contentDescription = null,
                                tint = colors.accent,
                                modifier = Modifier.size(20.dp),
                            )
                        }
                    }
                }
            }
        }
    }
}
