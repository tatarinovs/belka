package io.github.tatarinovs.belka.core

import android.Manifest
import android.app.Activity
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.provider.Settings
import android.webkit.MimeTypeMap
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import androidx.core.content.FileProvider
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import io.github.tatarinovs.belka.TransferService
import java.io.File
import java.lang.ref.WeakReference

object AndroidPlatform {
    private var currentActivity: WeakReference<Activity>? = null
    private var askedNotifications = false

    fun attachActivity(activity: Activity) {
        currentActivity = WeakReference(activity)
    }

    fun detachActivity(activity: Activity) {
        if (currentActivity?.get() === activity) {
            currentActivity = null
        }
    }

    fun hasStorageAccess(context: Context): Boolean =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            Environment.isExternalStorageManager()
        } else {
            ContextCompat.checkSelfPermission(context, Manifest.permission.WRITE_EXTERNAL_STORAGE) ==
                PackageManager.PERMISSION_GRANTED
        }

    fun requestStorageAccess(context: Context) {
        val activity = currentActivity?.get()
        val host: Context = activity ?: context
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            val intent = Intent(
                Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION,
                Uri.parse("package:${context.packageName}"),
            )
            if (activity == null) intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            try {
                host.startActivity(intent)
            } catch (_: Exception) {
                val fallback = Intent(Settings.ACTION_MANAGE_ALL_FILES_ACCESS_PERMISSION)
                if (activity == null) fallback.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                host.startActivity(fallback)
            }
        } else if (activity != null) {
            ActivityCompat.requestPermissions(
                activity,
                arrayOf(
                    Manifest.permission.READ_EXTERNAL_STORAGE,
                    Manifest.permission.WRITE_EXTERNAL_STORAGE,
                ),
                1,
            )
        }
    }

    fun askNotifications() {
        if (askedNotifications || Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) return
        val activity = currentActivity?.get() ?: return
        askedNotifications = true
        if (ContextCompat.checkSelfPermission(activity, Manifest.permission.POST_NOTIFICATIONS) !=
            PackageManager.PERMISSION_GRANTED
        ) {
            activity.runOnUiThread {
                ActivityCompat.requestPermissions(
                    activity,
                    arrayOf(Manifest.permission.POST_NOTIFICATIONS),
                    2,
                )
            }
        }
    }

    fun openFile(context: Context, path: String) {
        val activity = currentActivity?.get()
        val host: Context = activity ?: context
        val file = File(path)
        val type = MimeTypeMap.getSingleton().getMimeTypeFromExtension(file.extension.lowercase()) ?: "*/*"
        val uri = FileProvider.getUriForFile(context, "${context.packageName}.fileprovider", file)
        val view = Intent(Intent.ACTION_VIEW)
            .setDataAndType(uri, type)
            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        val chooser = Intent.createChooser(view, file.name)
        if (activity == null) chooser.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        try {
            host.startActivity(chooser)
        } catch (_: Exception) {
            throw BelkaException("Нет приложения для открытия «${file.name}»")
        }
    }

    fun playUrl(context: Context, url: String, mime: String, title: String) {
        val activity = currentActivity?.get()
        val host: Context = activity ?: context
        val view = Intent(Intent.ACTION_VIEW)
            .setDataAndType(Uri.parse(url), mime)
            .putExtra("title", title)
        val chooser = Intent.createChooser(view, title)
        if (activity == null) chooser.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        try {
            TransferService.playing(context.applicationContext, title)
            host.startActivity(chooser)
        } catch (_: Exception) {
            throw BelkaException("Нет приложения, которое может воспроизвести это видео")
        }
    }

    fun openUrl(context: Context, url: String) {
        val activity = currentActivity?.get()
        val host: Context = activity ?: context
        val intent = Intent(Intent.ACTION_VIEW, Uri.parse(url))
        if (activity == null) intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        host.startActivity(intent)
    }

    fun copyToClipboard(context: Context, text: String) {
        val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        clipboard.setPrimaryClip(ClipData.newPlainText("Belka", text))
    }

    fun setImmersive(on: Boolean) {
        val activity = currentActivity?.get() ?: return
        activity.runOnUiThread {
            val bars = WindowCompat.getInsetsController(activity.window, activity.window.decorView)
            if (on) {
                bars.systemBarsBehavior =
                    WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
                bars.hide(WindowInsetsCompat.Type.systemBars())
            } else {
                bars.show(WindowInsetsCompat.Type.systemBars())
            }
        }
    }
}
