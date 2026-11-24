package io.github.tatarinovs.belka

import io.github.tatarinovs.belka.core.PreviewKind
import io.github.tatarinovs.belka.core.previewKind
import org.junit.Assert.assertEquals
import org.junit.Test

class FormatTest {
    @Test
    fun previewKindClassifiesVideoAudioAndOtherFormats() {
        for (ext in listOf("mp4", "mkv", "avi", "m2ts", "mts", "flv", "mpg", "mpeg", "vob", "3gp")) {
            assertEquals("expected VIDEO for .$ext", PreviewKind.VIDEO, previewKind("movie.$ext", 1234))
        }
        assertEquals(PreviewKind.TEXT, previewKind("App.ts", 1024))
        assertEquals(PreviewKind.VIDEO, previewKind("stream.ts", 2L * 1024L * 1024L))
        assertEquals(PreviewKind.PLAYER, previewKind("legacy.wmv", 1234))
        for (ext in listOf("mp3", "m4a", "flac", "opus")) {
            assertEquals("expected AUDIO for .$ext", PreviewKind.AUDIO, previewKind("track.$ext", 1234))
        }
        assertEquals(PreviewKind.PDF, previewKind("doc.pdf", 1234))
        assertEquals(PreviewKind.IMAGE, previewKind("photo.PNG", 1234))
        assertEquals(PreviewKind.IMAGE, previewKind("iphone.heic", 1234))
        assertEquals(PreviewKind.IMAGE, previewKind("iphone.HEIF", 1234))
    }
}
