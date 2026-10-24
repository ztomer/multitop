package com.ztomer.multitop.android.ui

import com.ztomer.multitop.android.api.Fetch
import com.ztomer.multitop.android.api.HealthSummary
import com.ztomer.multitop.android.api.Overview
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Test

class HostsControllerTest {
    private val one = Overview(HealthSummary(1, 1, 0, 0), emptyList())
    private val two = Overview(HealthSummary(2, 2, 0, 0), emptyList())

    @Test
    fun `a read shows what it found, and a failure keeps the last overview it had`() {
        val answers = ArrayDeque(listOf(Fetch.Read(one), Fetch.Failed("could not reach x")))
        val hosts = HostsController({ answers.removeFirst() }, Runnable::run)
        assertEquals(HostsState.Reading(null), hosts.state)
        hosts.refresh()
        assertEquals(HostsState.Read(one), hosts.state)
        hosts.refresh()
        assertEquals(HostsState.Failed("could not reach x", one), hosts.state)
    }

    @Test
    fun `last wins - an answer to a superseded read is dropped`() {
        val queued = mutableListOf<Runnable>()
        var answer: Fetch = Fetch.Read(two)
        val hosts = HostsController({ answer }, queued::add)
        hosts.refresh()
        hosts.refresh()
        assertEquals(HostsState.Reading(null), hosts.state)
        // The newer read lands first; the older one arrives after and must not show.
        queued[1].run()
        assertEquals(HostsState.Read(two), hosts.state)
        answer = Fetch.Read(one)
        queued[0].run()
        assertEquals(HostsState.Read(two), hosts.state)
    }
}
