package io.github.tatarinovs.belka.core

fun interface UrlOpener {
    fun openUrl(url: String)
}

/**
 * Direct JNI entry points into `libbelka_android.so`.
 * WorkManager may run [backupRun] in a fresh process before Application.onCreate
 * finishes or without UI, so the library is loaded in this object's initializer.
 */
object BelkaNative {
    init {
        System.loadLibrary("belka_android")
    }

    @JvmStatic
    external fun init(configDir: String, cacheDir: String)

    @JvmStatic
    external fun call(method: String, argsJson: String): String

    @JvmStatic
    external fun oauthSignIn(argsJson: String, opener: UrlOpener): String

    @JvmStatic
    external fun backupRun(dir: String, id: String): String

    @JvmStatic
    external fun backupProgress(id: String): String

    @JvmStatic
    external fun backupCancel(id: String)
}
