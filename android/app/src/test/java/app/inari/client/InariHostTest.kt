package app.inari.client

import org.junit.Assert.assertFalse
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
}
