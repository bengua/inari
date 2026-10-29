package app.inari.client

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class InariHostTest {
    private val base = "https://core.example.net:8462"

    @Test fun `the same origin stays in the view`() {
        assertTrue(InariHost.sameOrigin(base, "$base/threads/thr_1?a=1"))
        assertTrue(InariHost.sameOrigin(base, "https://CORE.example.net:8462/"))
    }

    @Test fun `another host, port or scheme leaves for the system browser`() {
        assertFalse(InariHost.sameOrigin(base, "https://evil.example/"))
        assertFalse(InariHost.sameOrigin(base, "https://core.example.net:9443/"))
        assertFalse(InariHost.sameOrigin(base, "https://core.example.net/"))
        assertFalse(InariHost.sameOrigin(base, "http://core.example.net:8462/"))
        assertFalse(InariHost.sameOrigin(base, "https://core.example.net.evil.example:8462/"))
        assertFalse(InariHost.sameOrigin(base, "https://user@evil.example:8462@core.example.net:8462/"))
    }

    @Test fun `a cleartext or malformed base never matches`() {
        assertFalse(InariHost.sameOrigin("http://core:8462", "http://core:8462/"))
        assertFalse(InariHost.sameOrigin("not a url", base))
        assertFalse(InariHost.sameOrigin(base, "not a url"))
    }

    @Test fun `a failed load says what to check, by cause`() {
        assertEquals(R.string.inari_hint_network, InariHost.hint("net::ERR_NAME_NOT_RESOLVED"))
        assertEquals(R.string.inari_hint_network, InariHost.hint("net::ERR_INTERNET_DISCONNECTED"))
        assertEquals(R.string.inari_hint_core, InariHost.hint("net::ERR_CONNECTION_REFUSED"))
        assertEquals(R.string.inari_hint_core, InariHost.hint("net::ERR_TIMED_OUT"))
        assertEquals(R.string.inari_hint_certificate, InariHost.hint("certificate"))
        assertEquals(R.string.inari_hint_other, InariHost.hint("net::ERR_SOMETHING_ELSE"))
    }

    @Test fun `the page asks to share a text, and only that address does`() {
        // as the page writes it: encodeURIComponent, which keeps "+" as %2B and a space as %20
        assertEquals("Bonjour à toi + 2\n> cité", InariHost.sharedText("inari-share:Bonjour%20%C3%A0%20toi%20%2B%202%0A%3E%20cit%C3%A9"))
        assertNull(InariHost.sharedText("inari-share:"))
        assertNull(InariHost.sharedText("inari-share:%E0%A4%A"))
        assertNull(InariHost.sharedText("https://core.example.net/inari-share:x"))
    }
}
