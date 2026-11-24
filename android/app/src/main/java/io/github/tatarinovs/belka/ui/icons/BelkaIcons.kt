package io.github.tatarinovs.belka.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.PathParser
import androidx.compose.ui.unit.dp

/** Exact vector icons ported from `gui/src/lib/components/Icon.svelte`. */
enum class BelkaIcon(val pathData: String, val filled: Boolean = false, val strokeWidth: Float = 1.8f) {
    FOLDER("M3 6.5A1.5 1.5 0 0 1 4.5 5h4l2 2h9A1.5 1.5 0 0 1 21 8.5v9a1.5 1.5 0 0 1-1.5 1.5h-15A1.5 1.5 0 0 1 3 17.5z", filled = true),
    FILE("M6 3h8l5 5v12a1 1 0 0 1-1 1H6a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1zm8 0v5h5"),
    UP("M12 19V5m-6 6 6-6 6 6"),
    DOWN("M12 5v14m-6-6 6 6 6-6"),
    GRIP("M6 9h12M6 15h12"),
    MENU("M4 6h16M4 12h16M4 18h16"),
    REFRESH("M20 12a8 8 0 1 1-2.34-5.66M20 4v4h-4"),
    PLUS("M12 5v14M5 12h14"),
    TRASH("M4 7h16M9 7V4h6v3m-8 0 1 13h8l1-13"),
    CHEVRON("m9 6 6 6-6 6"),
    CLOUD("M7 18a4 4 0 0 1-.6-7.95A6 6 0 0 1 18 9a4.5 4.5 0 0 1-.5 9z"),
    UPLOAD("M12 16V4m-5 5 5-5 5 5M5 20h14"),
    COPY("M9 9h11v11H9zM15 9V4H4v11h5"),
    DOWNLOAD("M12 4v12m-5-5 5 5 5-5M5 20h14"),
    CLOSE("M6 6l12 12M18 6 6 18"),
    RETRY("M4 12a8 8 0 1 0 2.34-5.66M4 4v4h4"),
    REVEAL("M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2zm9 3v6m-3-3 3 3 3-3"),
    CHEVRON_DOWN("m6 9 6 6 6-6"),
    DRIVE("M3 13h18v5a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1zm0 0 2.5-7.2A1 1 0 0 1 6.4 5h11.2a1 1 0 0 1 .9.8L21 13M17 16h.01"),
    PLACE("M3 10.5 12 3l9 7.5V20a1 1 0 0 1-1 1h-5v-6H9v6H4a1 1 0 0 1-1-1z"),
    ARROW_RIGHT("M5 12h14m-6-6 6 6-6 6"),
    ARROW_LEFT("M19 12H5m6-6-6 6 6 6"),
    EYE("M2 12s3.6-7 10-7 10 7 10 7-3.6 7-10 7S2 12 2 12zm10 3a3 3 0 1 0 0-6 3 3 0 0 0 0 6z"),
    LOCK("M7 11V8a5 5 0 0 1 10 0v3M5 11h14v10H5z"),
    EYE_OFF("M3 3l18 18M10.6 5.1A10 10 0 0 1 12 5c6.4 0 10 7 10 7a17 17 0 0 1-3.2 4.1M6.6 6.6A17 17 0 0 0 2 12s3.6 7 10 7a9.7 9.7 0 0 0 5.4-1.6M9.9 9.9a3 3 0 0 0 4.2 4.2"),
    SETTINGS("M4 6h9m4 0h3M4 12h3m4 0h9M4 18h11m4 0h1M13 4v4M7 10v4M15 16v4"),
    LINK("M10 14a4 4 0 0 0 5.66 0l3-3a4 4 0 0 0-5.66-5.66l-1 1M14 10a4 4 0 0 0-5.66 0l-3 3a4 4 0 0 0 5.66 5.66l1-1"),
    MORE("M12 5h0.01M12 12h0.01M12 19h0.01", strokeWidth = 3f),
    CHECK("m5 12.5 4.5 4.5L19 7"),
    PHONE("M8 3h8a1 1 0 0 1 1 1v16a1 1 0 0 1-1 1H8a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1zm3 15h2"),
    SORT("M7 4v16m-3-3 3 3 3-3M14 6h6M14 12h4M14 18h2"),
    CHECK_ALL("M4 4h16v16H4zm4 8.5 3 3 5-6"),
    EXTERNAL("M14 4h6v6m0-6-9 9m7 1v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h5"),
    CLOCK("M12 7v5l3 2m6-2a9 9 0 1 1-18 0 9 9 0 0 1 18 0z"),
    INFO("M12 11v5m0-8h0.01M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0z"),
    PLAY("M8 5v14l11-7z"),
    STOP("M7 7h10v10H7z"),
    EDIT("m15 5 4 4M4 20l4-1 12-12a2.8 2.8 0 0 0-4-4L4 15z");

    val vector: ImageVector by lazy {
        ImageVector.Builder(
            name = name,
            defaultWidth = 24.dp,
            defaultHeight = 24.dp,
            viewportWidth = 24f,
            viewportHeight = 24f,
        ).addPath(
            pathData = PathParser().parsePathString(pathData).toNodes(),
            fill = if (filled) SolidColor(Color.White) else null,
            stroke = if (filled) null else SolidColor(Color.White),
            strokeLineWidth = strokeWidth,
            strokeLineCap = StrokeCap.Round,
            strokeLineJoin = StrokeJoin.Round,
        ).build()
    }
}
