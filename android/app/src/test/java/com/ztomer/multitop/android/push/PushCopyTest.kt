package com.ztomer.multitop.android.push

import com.ztomer.multitop.android.api.Sent
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertFalse
import org.junit.jupiter.api.Assertions.assertNull
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test

class PushCopyTest {
    private val ntfy = PushChoice.UnifiedPush("io.heckel.ntfy")

    private fun status(
        body: String,
        choice: PushChoice = ntfy,
    ) = PushCopy.status(Sent.Answered(200, body), choice)

    @Test
    fun `the route the server reports is said, by name`() {
        assertEquals(
            "✓ Alerts reach this phone through UnifiedPush (ntfy)",
            status("""{"unifiedpush":true,"fcm":false,"fcm_ready":false,"route":"unifiedpush"}"""),
        )
        assertEquals(
            "✓ Alerts reach this phone through FCM",
            status("""{"unifiedpush":false,"fcm":true,"fcm_ready":true,"route":"fcm"}""", PushChoice.Fcm),
        )
    }

    @Test
    fun `a missing step is a status naming the step, never an alarm`() {
        val none = """{"unifiedpush":false,"fcm":false,"fcm_ready":false,"route":null}"""
        assertEquals(PushCopy.NO_DISTRIBUTOR, status(none, PushChoice.None))
        assertEquals(PushCopy.REGISTERING, status(none))
        assertEquals(
            PushCopy.FCM_NO_ACCOUNT,
            status("""{"unifiedpush":false,"fcm":true,"fcm_ready":false,"route":null}""", PushChoice.Fcm),
        )
        for (s in listOf(PushCopy.NO_DISTRIBUTOR, PushCopy.REGISTERING, PushCopy.FCM_NO_ACCOUNT, PushCopy.NOTHING_REGISTERED)) {
            assertFalse(s.startsWith("✗") || s.startsWith("⚠"), s)
        }
    }

    @Test
    fun `a failed read is the one fault`() {
        assertTrue(PushCopy.status(Sent.Failed("could not reach x"), ntfy).startsWith("✗"))
        assertTrue(status("not json").startsWith("✗"))
    }

    @Test
    fun `the test result says which route took it, or why none did`() {
        assertEquals(
            "✓ Test alert sent through UnifiedPush (ntfy)",
            PushCopy.tested(Sent.Answered(200, """{"via":"unifiedpush","failed":[]}""")),
        )
        assertEquals(PushCopy.NOTHING_REGISTERED, PushCopy.tested(Sent.Answered(409, """{"via":null,"failed":[]}""")))
        assertEquals(
            "✗ The test alert was not delivered: UnifiedPush (ntfy): distributor answered 500; FCM: FCM answered 500",
            PushCopy.tested(
                Sent.Answered(
                    502,
                    """{"via":null,"failed":[["unifiedpush","distributor answered 500"],["fcm","FCM answered 500"]]}""",
                ),
            ),
        )
        assertEquals("✗ The test alert was not delivered: HTTP 500", PushCopy.tested(Sent.Answered(500, "")))
        assertEquals("✗ The test alert was not sent: down", PushCopy.tested(Sent.Failed("down")))
    }

    @Test
    fun `the controller shows the route, then the test, and refuses a second tap in flight`() {
        val queued = mutableListOf<Runnable>()
        var tests = 0
        val c =
            PushController(
                choice = { ntfy },
                status = { Sent.Answered(200, """{"route":"unifiedpush"}""") },
                test = {
                    tests++
                    Sent.Answered(200, """{"via":"unifiedpush","failed":[]}""")
                },
                run = queued::add,
            )
        assertEquals(PushCopy.READING, c.route)
        c.refresh()
        queued.removeAt(0).run()
        assertEquals("✓ Alerts reach this phone through UnifiedPush (ntfy)", c.route)
        assertNull(c.result)
        c.sendTest()
        c.sendTest()
        assertTrue(c.testing)
        assertEquals(1, queued.size, "a second tap while testing is not a second alert")
        queued.removeAt(0).run()
        assertFalse(c.testing)
        assertEquals(1, tests)
        assertEquals("✓ Test alert sent through UnifiedPush (ntfy)", c.result)
    }
}
