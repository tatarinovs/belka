package io.github.tatarinovs.belka

/** Bounded retries within a schedule interval; cancellation and a second
 * worker finding the task busy are successful no-ops. */
internal fun shouldRetryBackup(failed: Long, error: Boolean, busy: Boolean,
                              cancelled: Boolean, attempt: Int): Boolean =
    attempt < 3 && !busy && !cancelled && (failed > 0 || error)
