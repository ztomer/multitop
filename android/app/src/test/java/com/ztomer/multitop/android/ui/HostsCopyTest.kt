package com.ztomer.multitop.android.ui

import com.ztomer.multitop.android.api.HealthSummary
import com.ztomer.multitop.android.api.HostRow
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Test

/** The panel's states are told apart by glyph and words, never by colour alone (house UI rule 5). */
class HostsCopyTest {
    private fun row(
        reading: Boolean,
        breaching: Boolean,
        reachable: Boolean,
    ) = HostRow("h", null, 22, reading, if (reading) 50 else null, breaching, reachable)

    @Test
    fun `every row state has its own glyph`() {
        val glyphs =
            listOf(
                row(reading = true, breaching = false, reachable = true),
                row(reading = true, breaching = true, reachable = true),
                row(reading = false, breaching = false, reachable = true),
                row(reading = false, breaching = false, reachable = false),
            ).map { HostsCopy.row(it).substringBefore(' ') }
        assertEquals(listOf("✓", "⚠", "·", "✗"), glyphs)
    }

    @Test
    fun `an unread host is never summed as well`() {
        assertEquals("· 0 healthy, 1 not read yet of 1 host", HostsCopy.summary(HealthSummary(1, 0, 0, 1)))
        assertEquals("✓ 2 healthy of 2 hosts", HostsCopy.summary(HealthSummary(2, 2, 0, 0)))
        assertEquals(HostsCopy.NO_HOSTS, HostsCopy.summary(HealthSummary(0, 0, 0, 0)))
    }
}
