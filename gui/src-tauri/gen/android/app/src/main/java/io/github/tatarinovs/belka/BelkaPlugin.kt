package io.github.tatarinovs.belka

import android.Manifest
import android.app.Activity
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
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.io.File

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

/** What the Rust side needs from Android: storage access, the transfer service and backups. */
@TauriPlugin
class BelkaPlugin(private val activity: Activity) : Plugin(activity) {
    private var askedNotifications = false

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
