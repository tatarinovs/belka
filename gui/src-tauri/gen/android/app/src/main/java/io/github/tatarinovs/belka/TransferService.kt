package io.github.tatarinovs.belka

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationCompat
import androidx.core.content.ContextCompat

/**
 * A foreground service that only shows the transfer progress: while it runs,
 * Android does not freeze or kill the app, so the Rust side keeps transferring
 * with the screen off or another app open.
 */
class TransferService : Service() {
    companion object {
        private const val CHANNEL = "transfers"
        private const val NOTIFICATION_ID = 1

        @Volatile
        private var running = false

        @Volatile
        private var last = TransfersArgs()

        fun update(context: Context, args: TransfersArgs) {
            last = args
            if (args.active == 0) {
                if (running) context.stopService(Intent(context, TransferService::class.java))
                return
            }
            if (running) {
                // Restarting a foreground service from the background is not
                // allowed; the running one only gets a new notification.
                context.getSystemService(NotificationManager::class.java)
                    .notify(NOTIFICATION_ID, build(context, args))
            } else {
                ContextCompat.startForegroundService(context, Intent(context, TransferService::class.java))
            }
        }

        private fun ensureChannel(context: Context) {
            if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
            val manager = context.getSystemService(NotificationManager::class.java)
            if (manager.getNotificationChannel(CHANNEL) == null) {
                manager.createNotificationChannel(
                    NotificationChannel(CHANNEL, "Передачи файлов", NotificationManager.IMPORTANCE_LOW),
                )
            }
        }

        private fun build(context: Context, args: TransfersArgs): Notification {
            ensureChannel(context)
            val open = PendingIntent.getActivity(
                context,
                0,
                Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
                PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
            )
            val title = when {
                args.uploads == args.active -> "Загрузка в облако: ${args.active}"
                args.uploads == 0 -> "Скачивание: ${args.active}"
                else -> "Передача файлов: ${args.active}"
            }
            val builder = NotificationCompat.Builder(context, CHANNEL)
                .setSmallIcon(
                    if (args.uploads > 0) android.R.drawable.stat_sys_upload else android.R.drawable.stat_sys_download,
                )
                .setContentTitle(title)
                .setContentText(args.name)
                .setContentIntent(open)
                .setOngoing(true)
                .setOnlyAlertOnce(true)
                .setSilent(true)
            if (args.totalBytes > 0) {
                val permille = (args.doneBytes * 1000 / args.totalBytes).toInt().coerceIn(0, 1000)
                builder.setProgress(1000, permille, false)
            } else {
                builder.setProgress(0, 0, true)
            }
            return builder.build()
        }
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val notification = build(this, last)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            startForeground(NOTIFICATION_ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC)
        } else {
            startForeground(NOTIFICATION_ID, notification)
        }
        running = true
        // Everything finished while the service was starting.
        if (last.active == 0) stopSelf()
        return START_NOT_STICKY
    }

    override fun onDestroy() {
        running = false
        super.onDestroy()
    }

    /** Android 15 limits data sync services; the transfers go on until it kills the app. */
    override fun onTimeout(startId: Int, fgsType: Int) {
        stopSelf()
    }
}
