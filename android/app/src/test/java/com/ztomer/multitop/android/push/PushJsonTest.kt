package com.ztomer.multitop.android.push

import org.json.JSONObject
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertNull
import org.junit.jupiter.api.Test

class PushJsonTest {
    @Test
    fun `registrations are the shapes the server's Registration enum reads`() {
        val up = JSONObject(PushJson.unifiedPush("https://ntfy.example/upX?up=1", "BPub", "auth"))
        assertEquals("unifiedpush", up.getString("kind"))
        assertEquals("https://ntfy.example/upX?up=1", up.getString("endpoint"))
        assertEquals("BPub", up.getString("p256dh"))
        assertEquals("auth", up.getString("auth"))
        assertEquals(setOf("kind", "endpoint", "p256dh", "auth"), up.keys().asSequence().toSet())
        val fcm = JSONObject(PushJson.fcm("tok"))
        assertEquals(setOf("kind", "token"), fcm.keys().asSequence().toSet())
        assertEquals("fcm", fcm.getString("kind"))
    }

    @Test
    fun `the server's payload reads as an alert`() {
        // Byte for byte what crates/multitop/src/push serialises for a down host.
        val wire = """{"v":1,"kind":"down","host":"10.0.0.5","title":"beelink is unreachable","body":"lost"}"""
        assertEquals(Alert("down", "10.0.0.5", "beelink is unreachable", "lost"), PushJson.alert(wire))
    }

    @Test
    fun `anything else is refused rather than drawn half-read`() {
        assertNull(PushJson.alert("""{"v":2,"kind":"down","title":"t"}"""), "a newer version")
        assertNull(PushJson.alert("""{"v":1,"kind":"party","title":"t"}"""), "an unknown kind")
        assertNull(PushJson.alert("""{"v":1,"kind":"down"}"""), "no title")
        assertNull(PushJson.alert("not json"))
    }
}
