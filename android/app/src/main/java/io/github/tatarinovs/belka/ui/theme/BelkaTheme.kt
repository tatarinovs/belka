package io.github.tatarinovs.belka.ui.theme

import androidx.compose.foundation.background
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.ReadOnlyComposable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.TextUnit
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.tatarinovs.belka.core.ProviderKind

@Immutable
data class BelkaPalette(
    val isDark: Boolean,
    val bg: Color,
    val surface: Color,
    val sidebar: Color,
    val text: Color,
    val muted: Color,
    val border: Color,
    val hover: Color,
    val selected: Color,
    val selectedStrong: Color,
    val accent: Color,
    val accentText: Color,
    val success: Color,
    val danger: Color,
    val dangerBg: Color,
    val folder: Color,
    val crypt: Color,
    val warningBg: Color,
)

private val LightPalette = BelkaPalette(
    isDark = false,
    bg = Color(0xFFF6F7F9),
    surface = Color(0xFFFFFFFF),
    sidebar = Color(0xFFEEF0F3),
    text = Color(0xFF1C2026),
    muted = Color(0xFF5D6471),
    border = Color(0xFFDDE1E6),
    hover = Color(0xFFE7EAEF),
    selected = Color(0xFFDBE7FB),
    selectedStrong = Color(0xFFC4D8FA),
    accent = Color(0xFF2463D6),
    accentText = Color(0xFFFFFFFF),
    success = Color(0xFF1E8E3E),
    danger = Color(0xFFC4372B),
    dangerBg = Color(0xFFFDECEA),
    folder = Color(0xFFE0A43A),
    crypt = Color(0xFF455A64),
    warningBg = Color(0x30E0A43A),
)

private val DarkPalette = BelkaPalette(
    isDark = true,
    bg = Color(0xFF17191D),
    surface = Color(0xFF1E2126),
    sidebar = Color(0xFF15171A),
    text = Color(0xFFE4E7EC),
    muted = Color(0xFF8D95A2),
    border = Color(0xFF2E333A),
    hover = Color(0xFF272B31),
    selected = Color(0xFF1F3456),
    selectedStrong = Color(0xFF26416B),
    accent = Color(0xFF4A86F0),
    accentText = Color(0xFFFFFFFF),
    success = Color(0xFF4CC06A),
    danger = Color(0xFFF07167),
    dangerBg = Color(0xFF3A1F1D),
    folder = Color(0xFFE8B04F),
    crypt = Color(0xFF90A4AE),
    warningBg = Color(0x30E8B04F),
)

val LocalBelkaPalette = staticCompositionLocalOf { LightPalette }

object BelkaTheme {
    val colors: BelkaPalette
        @Composable
        @ReadOnlyComposable
        get() = LocalBelkaPalette.current
}

fun providerColor(kind: ProviderKind): Color = when (kind) {
    ProviderKind.YANDEX -> Color(0xFFE8492F)
    ProviderKind.MAILRU -> Color(0xFF1A73E8)
    ProviderKind.GOOGLE -> Color(0xFF1E8E3E)
    ProviderKind.WEBDAV -> Color(0xFF7C4DCC)
    ProviderKind.S3 -> Color(0xFFC77700)
    ProviderKind.SYNOLOGY -> Color(0xFF2F6F9F)
    ProviderKind.CRYPT -> Color(0xFF455A64)
}

@Composable
fun BelkaTheme(
    darkTheme: Boolean = isSystemInDarkTheme(),
    content: @Composable () -> Unit,
) {
    val palette = if (darkTheme) DarkPalette else LightPalette
    val colorScheme = if (darkTheme) {
        darkColorScheme(
            primary = palette.accent,
            onPrimary = palette.accentText,
            background = palette.bg,
            onBackground = palette.text,
            surface = palette.surface,
            onSurface = palette.text,
            surfaceVariant = palette.hover,
            onSurfaceVariant = palette.muted,
            outline = palette.border,
            outlineVariant = palette.border,
            error = palette.danger,
            onError = Color.White,
            errorContainer = palette.dangerBg,
            onErrorContainer = palette.danger,
        )
    } else {
        lightColorScheme(
            primary = palette.accent,
            onPrimary = palette.accentText,
            background = palette.bg,
            onBackground = palette.text,
            surface = palette.surface,
            onSurface = palette.text,
            surfaceVariant = palette.hover,
            onSurfaceVariant = palette.muted,
            outline = palette.border,
            outlineVariant = palette.border,
            error = palette.danger,
            onError = Color.White,
            errorContainer = palette.dangerBg,
            onErrorContainer = palette.danger,
        )
    }

    CompositionLocalProvider(LocalBelkaPalette provides palette) {
        MaterialTheme(
            colorScheme = colorScheme,
            content = content,
        )
    }
}

@Composable
fun ProviderBadge(
    kind: ProviderKind,
    short: String,
    modifier: Modifier = Modifier,
    size: Dp = 22.dp,
    cornerRadius: Dp = 6.dp,
    fontSize: TextUnit = 11.sp,
) {
    Box(
        modifier = modifier
            .size(size)
            .clip(RoundedCornerShape(cornerRadius))
            .background(providerColor(kind)),
        contentAlignment = Alignment.Center,
    ) {
        Text(
            text = short,
            color = Color.White,
            fontSize = fontSize,
            fontWeight = FontWeight.Bold,
            maxLines = 1,
        )
    }
}
