package io.github.tatarinovs.belka

/**
 * The Rust side of backups (`native.rs`, module `jni_api`). The library is
 * loaded here because WorkManager may start a backup in a process where the
 * page, which loads it otherwise, never ran.
 */
object BackupNative {
    init {
        System.loadLibrary("belka_gui_lib")
    }

    /** Runs a task to the end; returns its result as JSON. */
    @JvmStatic
    external fun run(dir: String, id: String): String

    /** The progress of a running task as JSON, or an empty string. */
    @JvmStatic
    external fun progress(id: String): String

    @JvmStatic
    external fun cancel(id: String)
}
