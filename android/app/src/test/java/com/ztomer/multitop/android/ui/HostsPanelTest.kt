package com.ztomer.multitop.android.ui

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import com.ztomer.multitop.android.api.HealthSummary
import com.ztomer.multitop.android.api.HostRow
import com.ztomer.multitop.android.api.Overview
import com.ztomer.multitop.android.pairing.Fixtures
import com.ztomer.multitop.android.pairing.PairingPayload
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

/** One test per state of the hosts panel, and the words that carry each row's state. */
@RunWith(RobolectricTestRunner::class)
class HostsPanelTest {
    @get:Rule
    val compose = createComposeRule()

    private val read = HostRow("192.168.0.33", "beelink", 22, true, 87, false, true)
    private val hot = HostRow("10.0.0.2", null, 22, true, 40, true, true)
    private val unread = HostRow("10.0.0.3", "nas", 22, false, null, false, true)
    private val gone = HostRow("10.0.0.4", "pihole", 22, false, null, false, false)
    private val overview = Overview(HealthSummary(4, 1, 1, 2), listOf(read, hot, unread, gone))
    private var refreshed = 0

    private fun show(state: HostsState) = compose.setContent { HostsPanel(state) { refreshed++ } }

    @Test
    fun aReadShowsTheServersCountAndEveryHost() {
        show(HostsState.Read(overview))
        compose.onNodeWithText("⚠ 1 healthy, 1 breaching, 2 not read yet of 4 hosts").assertIsDisplayed()
        compose.onNodeWithText("✓ beelink: health 87/100").assertIsDisplayed()
        compose.onNodeWithText("⚠ 10.0.0.2: breaching, health 40/100").assertIsDisplayed()
        compose.onNodeWithText("· nas: not read yet").assertIsDisplayed()
        compose.onNodeWithText("✗ pihole: unreachable").assertIsDisplayed()
        compose.onNodeWithText(HostsCopy.REFRESH).performClick()
        assertEquals(1, refreshed)
    }

    @Test
    fun readingSaysSoAndCannotBeStartedTwice() {
        show(HostsState.Reading(null))
        compose.onNodeWithText(HostsCopy.READING).assertIsDisplayed()
        compose.onNodeWithText(HostsCopy.REFRESH).assertIsNotEnabled()
    }

    @Test
    fun aFailureSaysWhyAndShowsTheLastReadAsTheLastRead() {
        show(HostsState.Failed("could not reach 192.168.1.20:8443: timeout", overview))
        compose.onNodeWithText("✗ Could not read the server: could not reach 192.168.1.20:8443: timeout").assertIsDisplayed()
        compose.onNodeWithText(HostsCopy.LAST_READ).assertIsDisplayed()
        compose.onNodeWithText("✓ beelink: health 87/100").assertIsDisplayed()
    }

    @Test
    fun thePairedScreenCarriesThePanel() {
        val printed = Fixtures.text("rust_server_pairing.txt").trim()
        val pairing = (PairingPayload.parse(printed) as PairingPayload.Parsed).pairing
        val actions = PairingActions({}, {}, {}, {})
        compose.setContent {
            PairingScreen(PairingState.Paired(pairing), actions, paired = { HostsPanel(HostsState.Read(overview)) {} }) {}
        }
        compose.onNodeWithText("✓ beelink: health 87/100").assertExists()
    }
}
