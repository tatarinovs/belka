package io.github.tatarinovs.belka

import android.app.Application
import io.github.tatarinovs.belka.core.BelkaCore
import io.github.tatarinovs.belka.core.BelkaNative
import io.github.tatarinovs.belka.core.TransferActivity
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

class BelkaApp : Application() {
    private val appScope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    override fun onCreate() {
        super.onCreate()
        // Keep the exact same config directory used by the Tauri build so
        // existing accounts, secrets.json, and backup tasks survive upgrades.
        BelkaNative.init(dataDir.absolutePath, cacheDir.absolutePath)

        appScope.launch {
            runCatching {
                val tasks = BelkaCore.backupList()
                BackupWorker.schedule(applicationContext, tasks)
            }
        }

        appScope.launch {
            var last: TransferActivity? = null
            while (true) {
                delay(1000)
                val activity = runCatching { BelkaCore.transferActivityBlocking() }.getOrNull() ?: continue
                if (activity != last) {
                    TransferService.update(applicationContext, activity)
                    last = activity
                }
            }
        }
    }
}
