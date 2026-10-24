package io.github.tatarinovs.belka

import android.Manifest
import android.app.Activity
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.graphics.Color
import android.graphics.pdf.PdfRenderer
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.os.ParcelFileDescriptor
import android.provider.Settings
import android.webkit.MimeTypeMap
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import androidx.core.content.FileProvider
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.io.File
import java.io.FileOutputStream
import java.io.IOException
import java.util.concurrent.Executors

@InvokeArg
class OpenFileArgs {
    var path: String = ""
}

@InvokeArg
class TransfersArgs {
    var active: Int = 0
    var uploads: Int = 0
    var name: String = ""
    var doneBytes: Long = 0
    var totalBytes: Long = 0
    /** The file an external player is streaming, or "". */
    var player: String = ""
}

@InvokeArg
class ScheduledBackup {
    var id: String = ""
    var name: String = ""
    var intervalHours: Int = 24
    var wifiOnly: Boolean = true
    var chargingOnly: Boolean = false
}

@InvokeArg
class ScheduleArgs {
    var tasks: Array<ScheduledBackup> = arrayOf()
}

@InvokeArg
class RunBackupArgs {
    var id: String = ""
    var name: String = ""
}

@InvokeArg
class PlayUrlArgs {
    var url: String = ""
    var mime: String = ""
    var title: String = ""
}

@InvokeArg
class ImmersiveArgs {
    var on: Boolean = false
}

@InvokeArg
class PdfArgs {
    var path: String = ""
    var page: Int = 0
    var width: Int = 0
    var out: String = ""
}

/**
 * What the Rust side needs from Android: storage access, the transfer
 * service, backups, the full screen for videos and PDF pages.
 */
@TauriPlugin
class BelkaPlugin(private val activity: Activity) : Plugin(activity) {
    private var askedNotifications = false

    /** PdfRenderer is not thread-safe, and rendering must not hold up the UI. */
    private val pdfThread = Executors.newSingleThreadExecutor()
    /** The PDF shown in the preview and its path; used on [pdfThread] only. */
    private var pdf: Pair<String, PdfRenderer>? = null

    private fun hasStorageAccess(): Boolean =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            Environment.isExternalStorageManager()
        } else {
            ContextCompat.checkSelfPermission(activity, Manifest.permission.WRITE_EXTERNAL_STORAGE) ==
                PackageManager.PERMISSION_GRANTED
        }

    @Command
    fun storageAccess(invoke: Invoke) {
        val ret = JSObject()
        ret.put("granted", hasStorageAccess())
        invoke.resolve(ret)
    }

    /** Opens the "All files access" page of the app, or asks for the old permissions. */
    @Command
    fun requestStorageAccess(invoke: Invoke) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            try {
                activity.startActivity(
                    Intent(
                        Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION,
                        Uri.parse("package:${activity.packageName}"),
                    ),
                )
            } catch (e: Exception) {
                activity.startActivity(Intent(Settings.ACTION_MANAGE_ALL_FILES_ACCESS_PERMISSION))
            }
        } else {
            ActivityCompat.requestPermissions(
                activity,
                arrayOf(
                    Manifest.permission.READ_EXTERNAL_STORAGE,
                    Manifest.permission.WRITE_EXTERNAL_STORAGE,
                ),
                1,
            )
        }
        invoke.resolve()
    }

    /** Opens a file in the app the user picks for its type. */
    @Command
    fun openFile(invoke: Invoke) {
        val file = File(invoke.parseArgs(OpenFileArgs::class.java).path)
        val type = MimeTypeMap.getSingleton().getMimeTypeFromExtension(file.extension.lowercase()) ?: "*/*"
        try {
            val uri = FileProvider.getUriForFile(activity, "${activity.packageName}.fileprovider", file)
            val view = Intent(Intent.ACTION_VIEW)
                .setDataAndType(uri, type)
                .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            activity.startActivity(Intent.createChooser(view, file.name))
            invoke.resolve()
        } catch (e: Exception) {
            invoke.reject("No app can open '${file.name}'")
        }
    }

    /**
     * Offers the players for a stream of the app's media server. VLC and MX
     * Player show `title` instead of the URL.
     */
    @Command
    fun playUrl(invoke: Invoke) {
        val args = invoke.parseArgs(PlayUrlArgs::class.java)
        val view = Intent(Intent.ACTION_VIEW)
            .setDataAndType(Uri.parse(args.url), args.mime)
            .putExtra("title", args.title)
        try {
            // No asking for notifications here: the question would cover the
            // choice of the player, and the service runs without them.
            TransferService.playing(activity.applicationContext, args.title)
            activity.startActivity(Intent.createChooser(view, args.title))
            invoke.resolve()
        } catch (e: Exception) {
            invoke.reject("No app can play the video")
        }
    }

    /** Keeps the process alive with a notification while anything is transferred. */
    @Command
    fun transfers(invoke: Invoke) {
        val args = invoke.parseArgs(TransfersArgs::class.java)
        if (args.active > 0) askNotifications()
        TransferService.update(activity.applicationContext, args)
        invoke.resolve()
    }

    /** Replaces the WorkManager schedule of backups with the enabled tasks. */
    @Command
    fun scheduleBackups(invoke: Invoke) {
        val args = invoke.parseArgs(ScheduleArgs::class.java)
        if (args.tasks.isNotEmpty()) askNotifications()
        BackupWorker.schedule(activity.applicationContext, args.tasks)
        invoke.resolve()
    }

    @Command
    fun runBackup(invoke: Invoke) {
        val args = invoke.parseArgs(RunBackupArgs::class.java)
        askNotifications()
        BackupWorker.runNow(activity.applicationContext, args.id, args.name)
        invoke.resolve()
    }

    /**
     * Hides the status and navigation bars, as for a video across the whole
     * screen; a swipe from the edge shows them for a moment.
     */
    @Command
    fun immersive(invoke: Invoke) {
        val on = invoke.parseArgs(ImmersiveArgs::class.java).on
        activity.runOnUiThread {
            val bars = WindowCompat.getInsetsController(activity.window, activity.window.decorView)
            if (on) {
                bars.systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
                bars.hide(WindowInsetsCompat.Type.systemBars())
            } else {
                bars.show(WindowInsetsCompat.Type.systemBars())
            }
        }
        invoke.resolve()
    }

    private fun pdfRenderer(path: String): PdfRenderer {
        pdf?.let { (open, renderer) ->
            if (open == path) return renderer
            renderer.close()
            pdf = null
        }
        val fd = ParcelFileDescriptor.open(File(path), ParcelFileDescriptor.MODE_READ_ONLY)
        val renderer = try {
            PdfRenderer(fd)
        } catch (e: Exception) {
            fd.close()
            throw e
        }
        pdf = path to renderer
        return renderer
    }

    /** The pages of a PDF with their sizes in points. Fails for a PDF with a password. */
    @Command
    fun pdfInfo(invoke: Invoke) {
        val args = invoke.parseArgs(PdfArgs::class.java)
        pdfThread.execute {
            try {
                val renderer = pdfRenderer(args.path)
                val pages = JSArray()
                for (i in 0 until renderer.pageCount) {
                    renderer.openPage(i).use { page ->
                        val size = JSObject()
                        size.put("width", page.width)
                        size.put("height", page.height)
                        pages.put(size)
                    }
                }
                val ret = JSObject()
                ret.put("pages", pages)
                invoke.resolve(ret)
            } catch (e: Exception) {
                invoke.reject(e.message ?: "Cannot open the PDF")
            }
        }
    }

    /** Renders a page of a PDF `width` pixels wide into a PNG file, once. */
    @Command
    fun pdfPage(invoke: Invoke) {
        val args = invoke.parseArgs(PdfArgs::class.java)
        pdfThread.execute {
            try {
                val out = File(args.out)
                if (!out.exists()) {
                    pdfRenderer(args.path).openPage(args.page).use { page ->
                        val height = (args.width.toLong() * page.height / page.width).toInt().coerceAtLeast(1)
                        val bitmap = Bitmap.createBitmap(args.width, height, Bitmap.Config.ARGB_8888)
                        // Pages are drawn on transparency, while paper is white.
                        bitmap.eraseColor(Color.WHITE)
                        page.render(bitmap, null, null, PdfRenderer.Page.RENDER_MODE_FOR_DISPLAY)
                        val part = File("${out.path}.part")
                        FileOutputStream(part).use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
                        bitmap.recycle()
                        if (!part.renameTo(out)) throw IOException("Cannot save '${out.name}'")
                    }
                }
                invoke.resolve()
            } catch (e: Exception) {
                invoke.reject(e.message ?: "Cannot show the page")
            }
        }
    }

    /** Android 13+ shows the progress notification only with this permission. */
    private fun askNotifications() {
        if (askedNotifications || Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) return
        askedNotifications = true
        if (ContextCompat.checkSelfPermission(activity, Manifest.permission.POST_NOTIFICATIONS) !=
            PackageManager.PERMISSION_GRANTED
        ) {
            activity.runOnUiThread {
                ActivityCompat.requestPermissions(activity, arrayOf(Manifest.permission.POST_NOTIFICATIONS), 2)
            }
        }
    }
}
