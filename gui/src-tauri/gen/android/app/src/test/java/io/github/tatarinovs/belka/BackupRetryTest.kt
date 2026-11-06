package io.github.tatarinovs.belka

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class BackupRetryTest {
    @Test fun failedRunsRetryWithinTheInterval() {
        assertTrue(shouldRetryBackup(1, false, false, false, 0))
        assertTrue(shouldRetryBackup(0, true, false, false, 2))
    }
    @Test fun retriesAreBounded() {
        assertFalse(shouldRetryBackup(1, true, false, false, 3))
    }
    @Test fun successBusyAndCancellationDoNotRetry() {
        assertFalse(shouldRetryBackup(0, false, false, false, 0))
        assertFalse(shouldRetryBackup(1, true, true, false, 0))
        assertFalse(shouldRetryBackup(1, true, false, true, 0))
    }
}
