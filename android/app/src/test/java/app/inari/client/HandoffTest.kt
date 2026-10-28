package app.inari.client

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class HandoffTest {
    @Test
    fun aSharedTextOrLinkGoesToTheComposerAsText() {
        assertEquals(Handoff(Handoff.Kind.TEXT, "Kyutai TTS — https://kyutai.org/tts/."),
            Handoff.of("android.intent.action.SEND", "Kyutai TTS — https://kyutai.org/tts/."))
        assertEquals(Handoff(Handoff.Kind.TEXT, "Der Zug fällt aus."),
            Handoff.of("android.intent.action.PROCESS_TEXT", "  Der Zug fällt aus.  "))
    }

    @Test
    fun nothingElseIsAHandoff() {
        assertNull(Handoff.of("android.intent.action.MAIN", "hello"))
        assertNull(Handoff.of("android.intent.action.SEND", "   "))
        assertNull(Handoff.of("android.intent.action.SEND", null))
    }

    @Test
    fun aSharedPdfOrPhotoIsAFile() {
        val pdf = Handoff.fileOf("android.intent.action.SEND", "content://x/1", "application/pdf")
        assertEquals(Handoff(Handoff.Kind.FILE, "", "content://x/1", "application/pdf"), pdf)
        assertEquals("image/jpeg", Handoff.fileOf("android.intent.action.SEND", "content://x/2", "IMAGE/JPEG")!!.mime)
        assertNull(Handoff.fileOf("android.intent.action.SEND", "content://x/3", "video/mp4"))
        assertNull(Handoff.fileOf("android.intent.action.SEND", null, "application/pdf"))
        assertNull(Handoff.fileOf("android.intent.action.VIEW", "content://x/1", "application/pdf"))
    }
}
