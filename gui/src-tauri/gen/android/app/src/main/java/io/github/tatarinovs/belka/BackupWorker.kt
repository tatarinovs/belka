package io.github.tatarinovs.belka

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.media.MediaScannerConnection
import android.os.Build
import androidx.core.app.NotificationCompat
import androidx.work.Constraints
import androidx.work.CoroutineWorker
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.ExistingWorkPolicy
import androidx.work.ForegroundInfo
import androidx.work.NetworkType
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.OutOfQuotaPolicy
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import androidx.work.workDataOf
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject

/**
 * Runs one backup task: on its schedule as periodic work, or once when the
 * user starts it. The work itself is done by Rust; this keeps the process
 * alive under a progress notification and reports the result.
 */
class BackupWorker(context: Context, params: WorkerParameters) : CoroutineWorker(context, params) {
    companion object {
        private const val KEY_ID = "id"
        private const val KEY_NAME = "name"
        private const val CHANNEL = "backups"
        private const val PREFS = "backups"
        private const val SCHEDULED = "scheduled"

        private fun periodic(id: String) = "backup-$id"

        private fun once(id: String) = "backup-now-$id"

        /** Replaces the schedule: tasks not in `tasks` stop running. */
        fun schedule(context: Context, tasks: Array<ScheduledBackup>) {
            val work = WorkManager.getInstance(context)
            val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            val wanted = tasks.map { it.id }.toSet()
            for (id in prefs.getStringSet(SCHEDULED, emptySet()).orEmpty() - wanted) {
                work.cancelUniqueWork(periodic(id))
            }
            for (task in tasks) {
                val constraints = Constraints.Builder()
                    .setRequiredNetworkType(if (task.wifiOnly) NetworkType.UNMETERED else NetworkType.CONNECTED)
                    .setRequiresCharging(task.chargingOnly)
                    .build()
                val request = PeriodicWorkRequestBuilder<BackupWorker>(
                    task.intervalHours.coerceAtLeast(1).toLong(),
                    TimeUnit.HOURS,
                )
                    .setConstraints(constraints)
                    .setInputData(workDataOf(KEY_ID to task.id, KEY_NAME to task.name))
                    .build()
                work.enqueueUniquePeriodicWork(periodic(task.id), ExistingPeriodicWorkPolicy.UPDATE, request)
            }
            prefs.edit().putStringSet(SCHEDULED, wanted).apply()
        }

        /** Starts a task now on any network: the user asked for it. */
        fun runNow(context: Context, id: String, name: String) {
            val request = OneTimeWorkRequestBuilder<BackupWorker>()
                .setConstraints(Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build())
                .setExpedited(OutOfQuotaPolicy.RUN_AS_NON_EXPEDITED_WORK_REQUEST)
                .setInputData(workDataOf(KEY_ID to id, KEY_NAME to name))
                .build()
            WorkManager.getInstance(context).enqueueUniqueWork(once(id), ExistingWorkPolicy.KEEP, request)
        }

        private fun ensureChannel(context: Context) {
            if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
            val manager = context.getSystemService(NotificationManager::class.java)
            if (manager.getNotificationChannel(CHANNEL) == null) {
                manager.createNotificationChannel(
                    NotificationChannel(CHANNEL, "Автобэкап", NotificationManager.IMPORTANCE_LOW),
                )
            }
        }
    }

    private val id = inputData.getString(KEY_ID).orEmpty()
    private val name = inputData.getString(KEY_NAME).orEmpty()

    /** Each task has its own notification, so parallel runs do not mix. */
    private val notificationId = 1000 + (id.hashCode() and 0xffff)

    override suspend fun doWork(): Result {
        if (id.isEmpty()) return Result.failure()
        try {
            setForeground(foregroundInfo(""))
        } catch (e: Exception) {
            // Android 12+ refuses a foreground service to work started in the
            // background; the run goes on until the system stops it, and the
            // next one continues from there.
        }
        val dir = applicationContext.dataDir.absolutePath
        val json = coroutineScope {
            val ticker = launch {
                try {
                    while (true) {
                        delay(1000)
                        val progress = BackupNative.progress(id)
                        if (progress.isNotEmpty()) notify(build(progress))
                    }
                } catch (e: CancellationException) {
                    // WorkManager stops the work: a constraint is no longer met.
                    if (isStopped) BackupNative.cancel(id)
                    throw e
                }
            }
            val result = withContext(Dispatchers.IO) { BackupNative.run(dir, id) }
            ticker.cancel()
            result
        }
        report(JSONObject(json))
        return Result.success()
    }

    override suspend fun getForegroundInfo(): ForegroundInfo = foregroundInfo("")

    private fun foregroundInfo(progress: String): ForegroundInfo =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            ForegroundInfo(notificationId, build(progress), ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC)
        } else {
            ForegroundInfo(notificationId, build(progress))
        }

    private fun openApp(): PendingIntent = PendingIntent.getActivity(
        applicationContext,
        0,
        Intent(applicationContext, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )

    /** The ongoing notification; `progress` is the JSON from Rust or empty. */
    private fun build(progress: String): Notification {
        ensureChannel(applicationContext)
        val builder = NotificationCompat.Builder(applicationContext, CHANNEL)
            .setSmallIcon(android.R.drawable.stat_sys_upload)
            .setContentTitle("Автобэкап «$name»")
            .setContentIntent(openApp())
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setSilent(true)
        val live = progress.takeIf { it.isNotEmpty() }?.let { JSONObject(it) }
        if (live == null || live.optBoolean("scanning")) {
            builder.setContentText("Поиск новых файлов…").setProgress(0, 0, true)
        } else {
            val total = live.optLong("files_total")
            val current = live.optString("current")
            builder.setContentText(
                "${live.optLong("files_done")} из $total" + if (current.isNotEmpty()) " · $current" else "",
            )
            val bytes = live.optLong("bytes_total")
            if (bytes > 0) {
                val permille = (live.optLong("bytes_done") * 1000 / bytes).toInt().coerceIn(0, 1000)
                builder.setProgress(1000, permille, false)
            }
        }
        return builder.build()
    }

    private fun notify(notification: Notification) {
        try {
            applicationContext.getSystemService(NotificationManager::class.java).notify(notificationId, notification)
        } catch (e: SecurityException) {
            // Notifications are not allowed.
        }
    }

    /** A notice for the result: something was uploaded, or the run failed. */
    private fun report(result: JSONObject) {
        if (result.optBoolean("busy")) return
        forgetFreed(result.optJSONArray("freed_paths"))
        val uploaded = result.optLong("uploaded")
        val failed = result.optLong("failed")
        val freed = result.optLong("freed")
        val error = result.optString("error").takeIf { !result.isNull("error") && it.isNotEmpty() }
        if (result.optBoolean("cancelled") || (uploaded == 0L && failed == 0L && freed == 0L && error == null)) {
            return
        }
        val firstFailure = result.optJSONArray("failures")?.optJSONArray(0)
            ?.let { "${it.optString(0)}: ${it.optString(1)}" }
        val lines = mutableListOf<String>()
        if (error != null) lines.add(error)
        if (uploaded > 0) lines.add("Загружено новых файлов: $uploaded")
        if (failed > 0) lines.add("Не удалось загрузить: $failed" + (firstFailure?.let { " ($it)" } ?: ""))
        if (freed > 0) lines.add("Освобождено места: удалено $freed старых файлов, они есть в облаке")
        val text = lines.joinToString("\n")
        val title = if (error != null || failed > 0) "Автобэкап «$name» с ошибками" else "Автобэкап «$name» выполнен"
        ensureChannel(applicationContext)
        val notification = NotificationCompat.Builder(applicationContext, CHANNEL)
            .setSmallIcon(android.R.drawable.stat_sys_upload_done)
            .setContentTitle(title)
            .setContentText(lines.first())
            .setStyle(NotificationCompat.BigTextStyle().bigText(text))
            .setContentIntent(openApp())
            .setAutoCancel(true)
            .build()
        // The ongoing notification goes away with the work; this one stays
        // under another id.
        try {
            applicationContext.getSystemService(NotificationManager::class.java)
                .notify(notificationId + 0x10000, notification)
        } catch (e: SecurityException) {
            // Notifications are not allowed.
        }
    }

    /** The gallery keeps showing deleted photos until the media scanner looks at them. */
    private fun forgetFreed(paths: JSONArray?) {
        if (paths == null || paths.length() == 0) return
        val list = Array(paths.length()) { paths.optString(it) }
        MediaScannerConnection.scanFile(applicationContext, list, null, null)
    }
}
