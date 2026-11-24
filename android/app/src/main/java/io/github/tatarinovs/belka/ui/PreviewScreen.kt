package io.github.tatarinovs.belka.ui

import android.content.res.Configuration
import android.graphics.Bitmap
import android.graphics.Color as AndroidColor
import android.graphics.pdf.PdfRenderer
import android.net.Uri
import android.os.ParcelFileDescriptor
import android.os.SystemClock
import android.view.LayoutInflater
import android.view.ViewGroup
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.gestures.detectTransformGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.systemBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.media3.common.MediaItem
import androidx.media3.common.PlaybackException
import androidx.media3.common.Player
import androidx.media3.common.util.UnstableApi
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.exoplayer.source.DefaultMediaSourceFactory
import androidx.media3.extractor.DefaultExtractorsFactory
import androidx.media3.ui.PlayerView
import coil3.compose.AsyncImage
import io.github.tatarinovs.belka.R
import io.github.tatarinovs.belka.core.AndroidPlatform
import io.github.tatarinovs.belka.core.BelkaCore
import io.github.tatarinovs.belka.core.Entry
import io.github.tatarinovs.belka.core.PREVIEW_CONFIRM_SIZE
import io.github.tatarinovs.belka.core.Preview
import io.github.tatarinovs.belka.core.PreviewKind
import io.github.tatarinovs.belka.core.TEXT_PREVIEW_MAX
import io.github.tatarinovs.belka.core.formatBytes
import io.github.tatarinovs.belka.core.previewKind
import io.github.tatarinovs.belka.ui.icons.BelkaIcon
import io.github.tatarinovs.belka.ui.theme.BelkaTheme
import java.io.File
import kotlin.math.abs
import kotlin.math.roundToInt
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext

@Composable
fun PreviewScreen(
    session: PreviewSession,
    onClose: () -> Unit,
) {
    val colors = BelkaTheme.colors
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val config = LocalConfiguration.current
    val landscape = config.orientation == Configuration.ORIENTATION_LANDSCAPE

    var index by remember(session) { mutableIntStateOf(session.start) }
    var preview by remember { mutableStateOf<Preview?>(null) }
    var streamSrc by remember { mutableStateOf("") }
    var loading by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf("") }
    var progressDone by remember { mutableStateOf(0L) }
    var progressTotal by remember { mutableStateOf(0L) }
    var stalled by remember { mutableStateOf("") }
    var confirmed by remember(session) { mutableStateOf(emptySet<String>()) }
    var unstreamable by remember(session) { mutableStateOf(emptySet<String>()) }
    var mediaFailed by remember { mutableStateOf(false) }
    var pdfError by remember { mutableStateOf("") }

    val entry = session.files.getOrNull(index)
    val kind = entry?.let { previewKind(it.name, it.size) }
    val isMedia = kind == PreviewKind.VIDEO || kind == PreviewKind.AUDIO
    val tooLarge = entry != null && (
        (kind == PreviewKind.TEXT && entry.size > TEXT_PREVIEW_MAX) ||
            (entry.size > PREVIEW_CONFIRM_SIZE &&
                !confirmed.contains(entry.path) &&
                kind != PreviewKind.PLAYER &&
                (!isMedia || unstreamable.contains(entry.path)))
        )
    val hasMediaSrc = streamSrc.isNotEmpty() || preview != null
    val canOpenExternally = preview != null ||
        (kind == PreviewKind.VIDEO && streamSrc.isNotEmpty()) ||
        kind == PreviewKind.PLAYER
    val immersive = landscape && kind == PreviewKind.VIDEO && hasMediaSrc && !mediaFailed

    DisposableEffect(Unit) {
        onDispose {
            scope.launch { runCatching { BelkaCore.previewCancel() } }
            AndroidPlatform.setImmersive(false)
        }
    }

    LaunchedEffect(immersive) {
        AndroidPlatform.setImmersive(immersive)
    }

    LaunchedEffect(entry?.path, tooLarge) {
        preview = null
        streamSrc = ""
        error = ""
        stalled = ""
        mediaFailed = false
        pdfError = ""
        if (entry == null || kind == null || tooLarge || kind == PreviewKind.PLAYER) {
            loading = false
            return@LaunchedEffect
        }
        loading = true
        progressDone = 0L
        progressTotal = entry.size
        if (isMedia && !unstreamable.contains(entry.path)) {
            val url = runCatching { BelkaCore.streamOpen(session.account.id, entry) }.getOrNull()
            if (!url.isNullOrEmpty()) {
                streamSrc = url
                loading = false
                return@LaunchedEffect
            }
            unstreamable = unstreamable + entry.path
            if (entry.size > PREVIEW_CONFIRM_SIZE && !confirmed.contains(entry.path)) {
                loading = false
                return@LaunchedEffect
            }
        }
        val started = SystemClock.elapsedRealtime()
        val poller = launch {
            while (true) {
                delay(200)
                val p = runCatching { BelkaCore.previewProgress() }.getOrNull() ?: continue
                progressDone = p.done
                progressTotal = if (p.total > 0) p.total else entry.size
                stalled = when {
                    p.attempt > 1 -> "Связь прервалась · попытка ${p.attempt} из ${p.max_attempts}"
                    p.done == 0L && SystemClock.elapsedRealtime() - started > 5000 -> "Ждём ответа сервера…"
                    else -> ""
                }
            }
        }
        try {
            preview = BelkaCore.previewFile(session.account.id, entry, kind == PreviewKind.TEXT)
        } catch (e: Exception) {
            val msg = e.message ?: e.toString()
            if (!msg.contains("Preview cancelled")) error = msg
        } finally {
            poller.cancel()
            loading = false
        }
    }

    fun go(step: Int) {
        val next = index + step
        if (next in session.files.indices) index = next
    }

    fun playExternally() {
        val item = entry ?: return
        scope.launch {
            try {
                val info = BelkaCore.streamOpenPlayer(session.account.id, item)
                AndroidPlatform.playUrl(context, info.url, info.mime, info.name)
            } catch (e: Exception) {
                error = e.message ?: e.toString()
            }
        }
    }

    fun openExternally() {
        if (kind == PreviewKind.PLAYER || (kind == PreviewKind.VIDEO && streamSrc.isNotEmpty())) {
            playExternally()
        } else {
            val path = preview?.path ?: return
            runCatching { AndroidPlatform.openFile(context, path) }.onFailure {
                error = it.message ?: it.toString()
            }
        }
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(if (immersive) Color.Black else colors.surface)
            .then(if (immersive) Modifier else Modifier.systemBarsPadding()),
    ) {
        if (!immersive) {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .height(56.dp)
                    .padding(horizontal = 4.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                BelkaToolButton(
                    icon = BelkaIcon.CLOSE,
                    contentDescription = "Закрыть",
                    onClick = onClose,
                )
                Column(modifier = Modifier.weight(1f)) {
                    Text(
                        text = entry?.name.orEmpty(),
                        color = colors.text,
                        fontSize = 16.sp,
                        fontWeight = FontWeight.SemiBold,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                    Text(
                        text = formatBytes(entry?.size ?: 0L),
                        color = colors.muted,
                        fontSize = 12.sp,
                    )
                }
                BelkaToolButton(
                    icon = BelkaIcon.EXTERNAL,
                    contentDescription = "Открыть в другом приложении",
                    enabled = canOpenExternally,
                    onClick = ::openExternally,
                )
            }
            HorizontalDivider(color = colors.border)
        }

        Box(
            modifier = Modifier
                .weight(1f)
                .fillMaxWidth()
                .background(if (immersive) Color.Black else colors.bg)
                .pointerInput(index, kind) {
                    if (kind == PreviewKind.IMAGE || kind == PreviewKind.VIDEO || kind == PreviewKind.AUDIO) return@pointerInput
                    var totalDx = 0f
                    detectHorizontalDragGestures(
                        onDragStart = { totalDx = 0f },
                        onHorizontalDrag = { _, dragAmount -> totalDx += dragAmount },
                        onDragEnd = {
                            if (abs(totalDx) > 120f) go(if (totalDx < 0) 1 else -1)
                        },
                    )
                },
            contentAlignment = Alignment.Center,
        ) {
            when {
                entry == null || kind == null -> {
                    PreviewMessage("Предпросмотр для файлов этого типа недоступен.")
                }
                kind == PreviewKind.TEXT && entry.size > TEXT_PREVIEW_MAX -> {
                    PreviewMessage("Текстовый файл слишком большой для предпросмотра.")
                }
                tooLarge -> {
                    Column(
                        modifier = Modifier.padding(24.dp),
                        horizontalAlignment = Alignment.CenterHorizontally,
                        verticalArrangement = Arrangement.spacedBy(12.dp),
                    ) {
                        Text(
                            text = "Для просмотра файл нужно скачать целиком: ${formatBytes(entry.size)}.",
                            color = colors.muted,
                            textAlign = TextAlign.Center,
                        )
                        BelkaPrimaryButton(onClick = { confirmed = confirmed + entry.path }) {
                            Text("Скачать и показать")
                        }
                    }
                }
                error.isNotEmpty() -> {
                    Text(
                        text = error,
                        color = colors.danger,
                        textAlign = TextAlign.Center,
                        modifier = Modifier.padding(24.dp),
                    )
                }
                kind == PreviewKind.PLAYER -> {
                    Column(
                        modifier = Modifier.padding(24.dp),
                        horizontalAlignment = Alignment.CenterHorizontally,
                        verticalArrangement = Arrangement.spacedBy(12.dp),
                    ) {
                        Text(
                            text = "Это видео встроенный плеер не воспроизводит: его покажет VLC или другой видеоплеер.",
                            color = colors.muted,
                            textAlign = TextAlign.Center,
                        )
                        BelkaPrimaryButton(onClick = ::playExternally) {
                            Text("Открыть в плеере")
                        }
                    }
                }
                isMedia && mediaFailed -> {
                    Column(
                        modifier = Modifier.padding(24.dp),
                        horizontalAlignment = Alignment.CenterHorizontally,
                        verticalArrangement = Arrangement.spacedBy(12.dp),
                    ) {
                        Text(
                            text = "Встроенный плеер не может воспроизвести этот файл.",
                            color = colors.muted,
                            textAlign = TextAlign.Center,
                        )
                        BelkaPrimaryButton(onClick = ::playExternally) {
                            Text("Открыть в плеере")
                        }
                    }
                }
                loading || !hasMediaSrc -> {
                    val pct = if (progressTotal > 0) {
                        ((progressDone.toFloat() / progressTotal.toFloat()) * 100f).coerceIn(0f, 100f)
                    } else {
                        0f
                    }
                    Column(
                        modifier = Modifier.padding(24.dp),
                        horizontalAlignment = Alignment.CenterHorizontally,
                        verticalArrangement = Arrangement.spacedBy(10.dp),
                    ) {
                        Text(text = "Загрузка… ${pct.toInt()}%", color = colors.muted)
                        Box(
                            modifier = Modifier
                                .width(240.dp)
                                .height(4.dp)
                                .clip(RoundedCornerShape(2.dp))
                                .background(colors.hover),
                        ) {
                            Box(
                                modifier = Modifier
                                    .fillMaxWidth(pct / 100f)
                                    .height(4.dp)
                                    .background(colors.accent),
                            )
                        }
                        if (stalled.isNotEmpty()) {
                            Text(text = stalled, color = colors.muted, fontSize = 12.5.sp)
                        }
                    }
                }
                kind == PreviewKind.IMAGE -> {
                    ZoomableImage(
                        filePath = preview!!.path,
                        contentDescription = entry.name,
                        onSwipe = { step -> go(step) },
                    )
                }
                kind == PreviewKind.VIDEO || kind == PreviewKind.AUDIO -> {
                    val uri = remember(streamSrc, preview?.path) {
                        if (streamSrc.isNotEmpty()) Uri.parse(streamSrc)
                        else Uri.fromFile(File(preview!!.path))
                    }
                    NativeMediaPlayer(
                        uri = uri,
                        isVideo = kind == PreviewKind.VIDEO,
                        onError = { mediaFailed = true },
                    )
                }
                kind == PreviewKind.PDF && pdfError.isEmpty() -> {
                    NativePdfViewer(
                        path = preview!!.path,
                        onFail = { pdfError = it },
                    )
                }
                kind == PreviewKind.PDF -> {
                    Column(
                        modifier = Modifier.padding(24.dp),
                        horizontalAlignment = Alignment.CenterHorizontally,
                        verticalArrangement = Arrangement.spacedBy(12.dp),
                    ) {
                        Text(
                            text = "Этот PDF не получается показать здесь: $pdfError",
                            color = colors.muted,
                            textAlign = TextAlign.Center,
                        )
                        BelkaPrimaryButton(onClick = ::openExternally) {
                            Text("Открыть в другом приложении")
                        }
                    }
                }
                kind == PreviewKind.TEXT -> {
                    val p = preview!!
                    Column(
                        modifier = Modifier
                            .fillMaxSize()
                            .verticalScroll(rememberScrollState())
                            .padding(horizontal = 16.dp, vertical = 12.dp),
                    ) {
                        if (p.truncated) {
                            Text(
                                text = "Показан первый мегабайт файла.",
                                color = colors.muted,
                                fontSize = 12.5.sp,
                                modifier = Modifier.padding(bottom = 8.dp),
                            )
                        }
                        SelectionContainer {
                            Text(
                                text = p.text.orEmpty(),
                                color = colors.text,
                                fontFamily = FontFamily.Monospace,
                                fontSize = 13.sp,
                                lineHeight = 18.sp,
                            )
                        }
                    }
                }
            }
        }

        if (!immersive && session.files.size > 1) {
            HorizontalDivider(color = colors.border)
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .background(colors.surface)
                    .padding(horizontal = 12.dp, vertical = 4.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.SpaceBetween,
            ) {
                BelkaToolButton(
                    icon = BelkaIcon.ARROW_LEFT,
                    contentDescription = "Предыдущий",
                    enabled = index > 0,
                    size = 48.dp,
                    iconSize = 24.dp,
                    onClick = { go(-1) },
                )
                Text(
                    text = "${index + 1} из ${session.files.size}",
                    color = colors.muted,
                    fontSize = 14.sp,
                )
                BelkaToolButton(
                    icon = BelkaIcon.ARROW_RIGHT,
                    contentDescription = "Следующий",
                    enabled = index < session.files.lastIndex,
                    size = 48.dp,
                    iconSize = 24.dp,
                    onClick = { go(1) },
                )
            }
        }
    }
}

@Composable
private fun PreviewMessage(text: String) {
    Text(
        text = text,
        color = BelkaTheme.colors.muted,
        textAlign = TextAlign.Center,
        modifier = Modifier.padding(24.dp),
    )
}

@Composable
private fun ZoomableImage(
    filePath: String,
    contentDescription: String,
    onSwipe: (Int) -> Unit,
) {
    var scale by remember(filePath) { mutableFloatStateOf(1f) }
    var offsetX by remember(filePath) { mutableFloatStateOf(0f) }
    var offsetY by remember(filePath) { mutableFloatStateOf(0f) }

    Box(
        modifier = Modifier
            .fillMaxSize()
            .pointerInput(filePath) {
                detectTapGestures(
                    onDoubleTap = {
                        if (scale > 1f) {
                            scale = 1f
                            offsetX = 0f
                            offsetY = 0f
                        } else {
                            scale = 2.5f
                        }
                    },
                )
            }
            .pointerInput(filePath) {
                detectTransformGestures { _, pan, zoom, _ ->
                    val nextScale = (scale * zoom).coerceIn(1f, 5f)
                    if (nextScale > 1f) {
                        scale = nextScale
                        offsetX += pan.x
                        offsetY += pan.y
                    } else {
                        scale = 1f
                        offsetX = 0f
                        offsetY = 0f
                    }
                }
            }
            .pointerInput(filePath) {
                var totalDx = 0f
                detectHorizontalDragGestures(
                    onDragStart = { totalDx = 0f },
                    onHorizontalDrag = { _, dragAmount ->
                        if (scale <= 1.05f) totalDx += dragAmount
                    },
                    onDragEnd = {
                        if (scale <= 1.05f && abs(totalDx) > 120f) {
                            onSwipe(if (totalDx < 0) 1 else -1)
                        }
                    },
                )
            },
        contentAlignment = Alignment.Center,
    ) {
        AsyncImage(
            model = File(filePath),
            contentDescription = contentDescription,
            contentScale = ContentScale.Fit,
            modifier = Modifier
                .fillMaxSize()
                .graphicsLayer(
                    scaleX = scale,
                    scaleY = scale,
                    translationX = offsetX,
                    translationY = offsetY,
                ),
        )
    }
}

@androidx.annotation.OptIn(UnstableApi::class)
@Composable
private fun NativeMediaPlayer(
    uri: Uri,
    isVideo: Boolean,
    onError: () -> Unit,
) {
    val context = LocalContext.current
    val player = remember(uri) {
        val extractors = DefaultExtractorsFactory().setConstantBitrateSeekingEnabled(true)
        ExoPlayer.Builder(context)
            .setMediaSourceFactory(DefaultMediaSourceFactory(context, extractors))
            .build()
            .apply {
                setMediaItem(MediaItem.fromUri(uri))
                playWhenReady = true
                prepare()
            }
    }

    DisposableEffect(player) {
        val listener = object : Player.Listener {
            override fun onPlayerError(error: PlaybackException) {
                onError()
            }
        }
        player.addListener(listener)
        onDispose {
            player.removeListener(listener)
            player.release()
        }
    }

    key(isVideo) {
        AndroidView(
            factory = { ctx ->
                val layoutRes = if (isVideo) {
                    R.layout.belka_video_player_view
                } else {
                    R.layout.belka_audio_player_view
                }
                (LayoutInflater.from(ctx).inflate(layoutRes, null) as PlayerView).apply {
                    layoutParams = ViewGroup.LayoutParams(
                        ViewGroup.LayoutParams.MATCH_PARENT,
                        ViewGroup.LayoutParams.MATCH_PARENT,
                    )
                    this.player = player
                    if (!isVideo) showController()
                }
            },
            update = { view ->
                if (view.player !== player) view.player = player
            },
            onRelease = { view ->
                view.player = null
            },
            modifier = if (isVideo) Modifier.fillMaxSize() else Modifier.fillMaxWidth().height(220.dp),
        )
    }
}

@Composable
private fun NativePdfViewer(
    path: String,
    onFail: (String) -> Unit,
) {
    val colors = BelkaTheme.colors
    val density = LocalDensity.current
    val pdfMutex = remember { Mutex() }
    var holder by remember(path) { mutableStateOf<Pair<ParcelFileDescriptor, PdfRenderer>?>(null) }
    var pageSizes by remember(path) { mutableStateOf<List<Pair<Int, Int>>>(emptyList()) }

    DisposableEffect(path) {
        onDispose {
            val current = holder
            holder = null
            if (current != null) {
                runCatching { current.second.close() }
                runCatching { current.first.close() }
            }
        }
    }

    LaunchedEffect(path) {
        withContext(Dispatchers.IO) {
            try {
                val fd = ParcelFileDescriptor.open(File(path), ParcelFileDescriptor.MODE_READ_ONLY)
                val renderer = try {
                    PdfRenderer(fd)
                } catch (e: Exception) {
                    fd.close()
                    throw e
                }
                val sizes = (0 until renderer.pageCount).map { i ->
                    renderer.openPage(i).use { p -> p.width to p.height }
                }
                holder = fd to renderer
                pageSizes = sizes
            } catch (e: Exception) {
                onFail(e.message ?: "Не удалось открыть PDF")
            }
        }
    }

    val renderer = holder?.second
    if (renderer == null) {
        Text("Открытие PDF…", color = colors.muted)
        return
    }

    BoxWithConstraints(modifier = Modifier.fillMaxSize()) {
        val boxMaxWidth = maxWidth
        val widthPx = with(density) { boxMaxWidth.toPx().roundToInt().coerceIn(200, 2000) }
        LazyColumn(
            modifier = Modifier.fillMaxSize().padding(horizontal = 8.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            items(pageSizes.size, key = { it }) { pageIndex ->
                val (pw, ph) = pageSizes[pageIndex]
                val aspect = if (pw > 0 && ph > 0) pw.toFloat() / ph.toFloat() else 0.7f
                var bitmap by remember(path, pageIndex, widthPx) { mutableStateOf<ImageBitmap?>(null) }

                LaunchedEffect(path, pageIndex, widthPx) {
                    bitmap = withContext(Dispatchers.IO) {
                        pdfMutex.withLock {
                            runCatching {
                                renderer.openPage(pageIndex).use { page ->
                                    val h = (widthPx.toLong() * page.height / page.width.coerceAtLeast(1))
                                        .toInt()
                                        .coerceAtLeast(1)
                                    val bmp = Bitmap.createBitmap(widthPx, h, Bitmap.Config.ARGB_8888)
                                    bmp.eraseColor(AndroidColor.WHITE)
                                    page.render(bmp, null, null, PdfRenderer.Page.RENDER_MODE_FOR_DISPLAY)
                                    bmp.asImageBitmap()
                                }
                            }.getOrNull()
                        }
                    }
                }

                Box(
                    modifier = Modifier
                        .fillMaxWidth()
                        .clip(RoundedCornerShape(4.dp))
                        .background(Color.White),
                    contentAlignment = Alignment.Center,
                ) {
                    val bmp = bitmap
                    if (bmp != null) {
                        Image(
                            bitmap = bmp,
                            contentDescription = "Страница ${pageIndex + 1}",
                            contentScale = ContentScale.FillWidth,
                            modifier = Modifier.fillMaxWidth(),
                        )
                    } else {
                        Box(
                            modifier = Modifier
                                .fillMaxWidth()
                                .height((boxMaxWidth / aspect)),
                        )
                    }
                }
            }
        }
    }
}
