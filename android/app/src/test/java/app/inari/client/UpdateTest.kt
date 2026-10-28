package app.inari.client

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class UpdateTest {
    private val sum = "a".repeat(64)

    @Test
    fun aNewerVersionIsOfferedWithAnAbsoluteUrl() {
        val u = Updates.parse("""{"versionCode": 19, "versionName": "0.9.0", "url": "Inari-0.9.0.apk", "sha256": "$sum"}""",
            "https://apk.example:8998/")
        assertEquals("https://apk.example:8998/Inari-0.9.0.apk", u!!.url)
        assertEquals(u, Updates.newer(u, 18))
        assertNull(Updates.newer(u, 19))
    }

    @Test
    fun aMalformedOrUncheckedFileOffersNothing() {
        assertNull(Updates.parse("""{"versionCode": 19, "versionName": "0.9.0", "url": "x.apk", "sha256": "short"}"""))
        assertNull(Updates.parse("not json"))
    }
}
