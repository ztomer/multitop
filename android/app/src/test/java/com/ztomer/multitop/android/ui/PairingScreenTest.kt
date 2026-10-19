package com.ztomer.multitop.android.ui

import androidx.compose.material3.Text
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextInput
import com.ztomer.multitop.android.pairing.Fixtures
import com.ztomer.multitop.android.pairing.PairingPayload
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

/**
 * One test per state of the one screen, under Robolectric on the JVM. Each
 * asserts the words that carry the state (glyph and text, never colour alone)
 * and that every control on it does what it says.
 */
@RunWith(RobolectricTestRunner::class)
class PairingScreenTest {
    @get:Rule
    val compose = createComposeRule()

    private val printed = Fixtures.text("rust_server_pairing.txt").trim()
    private val pairing = (PairingPayload.parse(printed) as PairingPayload.Parsed).pairing
    private val calls = mutableListOf<String>()
    private val actions =
        PairingActions(
            begin = { calls += "begin" },
            submit = { calls += "submit:$it" },
            cancel = { calls += "cancel" },
            unpair = { calls += "unpair" },
        )

    private fun show(state: PairingState) = compose.setContent { PairingScreen(state, actions) { Text(SCANNER) } }

    @Test
    fun notPairedSaysSoAndOffersPairing() {
        show(PairingState.NotPaired)
        compose.onNodeWithText(Copy.NOT_PAIRED).assertIsDisplayed()
        compose.onNodeWithText(Copy.PAIR).performClick()
        assertEquals(listOf("begin"), calls)
    }

    @Test
    fun pairingShowsTheScannerAndSubmitsWhatIsPasted() {
        show(PairingState.InProgress)
        compose.onNodeWithText(Copy.PAIRING).assertIsDisplayed()
        compose.onNodeWithText(SCANNER).assertExists()
        compose.onNodeWithText(Copy.PASTE_LABEL).performScrollTo().performTextInput(printed)
        compose.onNodeWithText(Copy.PAIR_PASTED).performScrollTo().performClick()
        compose.onNodeWithText(Copy.CANCEL).performScrollTo().performClick()
        assertEquals(listOf("submit:$printed", "cancel"), calls)
    }

    @Test
    fun pairedNamesTheHostAndTheFingerprintAndUnpairs() {
        show(PairingState.Paired(pairing))
        compose.onNodeWithText("✓ Paired to 192.168.1.20:8443").assertIsDisplayed()
        compose.onNodeWithText(Copy.fingerprint(pairing.pin.hex)).assertExists()
        compose.onNodeWithText(Copy.UNPAIR).performClick()
        assertEquals(listOf("unpair"), calls)
    }

    @Test
    fun failedStatesTheReasonAndOffersRetryAndBack() {
        show(PairingState.Failed("sha256 does not match cert"))
        compose.onNodeWithText("✗ Pairing failed: sha256 does not match cert").assertIsDisplayed()
        compose.onNodeWithText(Copy.TRY_AGAIN).performClick()
        compose.onNodeWithText(Copy.BACK).performClick()
        assertEquals(listOf("begin", "cancel"), calls)
    }

    @Test
    fun theScanSlotSaysWhyThereIsNoCamera() {
        val words =
            mapOf(
                CameraAccess.Asking to Copy.CAMERA_ASKING,
                CameraAccess.Denied to Copy.CAMERA_DENIED,
                CameraAccess.Absent to Copy.CAMERA_ABSENT,
                CameraAccess.Granted to SCANNER,
            )
        var access by mutableStateOf(CameraAccess.Asking)
        compose.setContent { ScanSlot(access) { Text(SCANNER) } }
        for ((state, text) in words) {
            access = state
            compose.onNodeWithText(text).assertExists()
        }
    }

    @Test
    fun anIpv6HostIsShownBracketed() {
        assertEquals("✓ Paired to [fe80::1]:8443", Copy.paired(pairing.copy(host = "fe80::1")))
    }

    private companion object {
        const val SCANNER = "camera preview"
    }
}
