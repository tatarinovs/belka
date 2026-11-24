package io.github.tatarinovs.belka.core

import android.content.Context
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json

enum class PaneMode(val wire: String) {
    CLOUD("cloud"),
    LOCAL("local");

    companion object {
        fun fromWire(value: String?): PaneMode = if (value == "local") LOCAL else CLOUD
    }
}

enum class SortKey(val wire: String) {
    NAME("name"),
    MODIFIED("modified"),
    SIZE("size");

    companion object {
        fun fromWire(value: String?): SortKey = entries.firstOrNull { it.wire == value } ?: NAME
    }
}

class Prefs(context: Context) {
    private val prefs = context.getSharedPreferences("belka_mobile_ui", Context.MODE_PRIVATE)
    private val json = Json { ignoreUnknownKeys = true }

    var mode: PaneMode
        get() = PaneMode.fromWire(prefs.getString("mode", null))
        set(value) = prefs.edit().putString("mode", value.wire).apply()

    var accountId: String?
        get() = prefs.getString("account", null)
        set(value) = prefs.edit().putString("account", value).apply()

    var localPath: String
        get() = prefs.getString("localPath", null) ?: ANDROID_DOWNLOADS
        set(value) = prefs.edit().putString("localPath", value).apply()

    var sortKey: SortKey
        get() = SortKey.fromWire(prefs.getString("sort", null))
        set(value) = prefs.edit().putString("sort", value.wire).apply()

    var updateCheckedAt: Long
        get() = prefs.getLong("updateChecked", 0L)
        set(value) = prefs.edit().putLong("updateChecked", value).apply()

    fun getCloudPaths(): MutableMap<String, String> {
        val raw = prefs.getString("cloudPaths", null) ?: return mutableMapOf()
        return runCatching { json.decodeFromString<Map<String, String>>(raw).toMutableMap() }
            .getOrDefault(mutableMapOf())
    }

    fun setCloudPaths(paths: Map<String, String>) {
        prefs.edit().putString("cloudPaths", json.encodeToString(paths)).apply()
    }

    companion object {
        const val ANDROID_DOWNLOADS = "/storage/emulated/0/Download"
    }
}
