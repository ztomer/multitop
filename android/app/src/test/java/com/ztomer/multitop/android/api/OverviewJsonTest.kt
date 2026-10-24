package com.ztomer.multitop.android.api

import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertNull
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test
import org.junit.jupiter.api.assertThrows

/** The two endpoints' bodies, in the exact shape `crates/multitop/src/server.rs` writes them. */
class OverviewJsonTest {
    @Test
    fun `health is the server's own count`() {
        assertEquals(
            HealthSummary(total = 3, healthy = 1, breaching = 1, unread = 1),
            OverviewJson.health("""{"breaching":1,"healthy":1,"total":3,"unread":1}"""),
        )
    }

    @Test
    fun `a host row reads every field, and an unread host has no health`() {
        val rows =
            OverviewJson.hosts(
                """[{"breaching":false,"has_snapshot":true,"health":87,"host":"192.168.0.33","name":"beelink",""" +
                    """"port":22,"reachable":true,"user":""},""" +
                    """{"breaching":false,"has_snapshot":false,"health":null,"host":"nas","name":null,""" +
                    """"port":2222,"reachable":false,"user":"root"}]""",
            )
        assertEquals(HostRow("192.168.0.33", "beelink", 22, true, 87, false, true), rows[0])
        assertEquals("beelink", rows[0].label)
        assertNull(rows[1].health)
        assertEquals("nas", rows[1].label)
    }

    @Test
    fun `a missing field is refused by name, never defaulted`() {
        // The server before A2.5 sent no `unread`; a reader that defaulted it to
        // 0 would show every unread host as healthy again.
        val e = assertThrows<OverviewJson.Malformed> { OverviewJson.health("""{"breaching":0,"healthy":1,"total":1}""") }
        assertTrue("unread" in e.message.orEmpty(), e.message)
        val wrong = assertThrows<OverviewJson.Malformed> { OverviewJson.hosts("""[{"host":1}]""") }
        assertTrue("host" in wrong.message.orEmpty(), wrong.message)
        assertThrows<OverviewJson.Malformed> { OverviewJson.hosts("not json") }
    }
}
