package com.ztomer.multitop.android.push

import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Test

class PushPlanTest {
    private val me = "com.ztomer.multitop.android"
    private val ntfy = "io.heckel.ntfy"

    @Test
    fun `an installed distributor wins over FCM`() {
        assertEquals(PushChoice.UnifiedPush(ntfy), PushPlan.choose(listOf(ntfy), null, me, fcmConfigured = true))
    }

    @Test
    fun `the saved distributor is kept while it is installed, and dropped when it is not`() {
        val other = "org.example.distributor"
        assertEquals(PushChoice.UnifiedPush(other), PushPlan.choose(listOf(ntfy, other), other, me, false))
        assertEquals(PushChoice.UnifiedPush(ntfy), PushPlan.choose(listOf(ntfy), other, me, false))
    }

    @Test
    fun `this app is never its own distributor`() {
        assertEquals(PushChoice.Fcm, PushPlan.choose(listOf(me), me, me, fcmConfigured = true))
    }

    @Test
    fun `FCM only without a distributor, and nothing without either`() {
        assertEquals(PushChoice.Fcm, PushPlan.choose(emptyList(), null, me, fcmConfigured = true))
        assertEquals(PushChoice.None, PushPlan.choose(emptyList(), null, me, fcmConfigured = false))
    }
}
