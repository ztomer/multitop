package com.ztomer.multitop.android.ui

import com.ztomer.multitop.android.pairing.Fixtures
import com.ztomer.multitop.android.pairing.PairingPayload
import com.ztomer.multitop.android.store.AesGcmSealer
import com.ztomer.multitop.android.store.PairingStore
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test
import org.junit.jupiter.api.io.TempDir
import java.io.File
import javax.crypto.KeyGenerator

class PairingControllerTest {
    @TempDir
    lateinit var dir: File

    private val printed = Fixtures.text("rust_server_pairing.txt").trim()
    private val pairing = (PairingPayload.parse(printed) as PairingPayload.Parsed).pairing
    private val key = KeyGenerator.getInstance("AES").apply { init(AES_BITS) }.generateKey()

    private fun store() = PairingStore(File(dir, "pairing.sealed"), AesGcmSealer { key })

    @Test
    fun `with nothing stored it starts not paired`() {
        assertEquals(PairingState.NotPaired, PairingController(store()).state)
    }

    @Test
    fun `a good string pairs, is stored, and a restart starts paired`() {
        val controller = PairingController(store())
        controller.begin()
        assertEquals(PairingState.InProgress, controller.state)
        controller.submit(printed)
        assertEquals(PairingState.Paired(pairing), controller.state)
        assertEquals(PairingState.Paired(pairing), PairingController(store()).state)
    }

    @Test
    fun `a wrong fingerprint is refused with the reason, and nothing is stored`() {
        val wrong = printed.replace("sha256=${pairing.pin.hex}", "sha256=${Fixtures.OTHER_SHA256}")
        val controller = PairingController(store())
        controller.begin()
        controller.submit(wrong)
        val state = controller.state
        assertTrue(state is PairingState.Failed, "$state")
        assertTrue((state as PairingState.Failed).reason.contains("sha256 does not match cert"), state.reason)
        assertEquals(PairingStore.Loaded.Empty, store().load())
    }

    @Test
    fun `unpair forgets the server on disk too`() {
        val controller = PairingController(store())
        controller.submit(printed)
        controller.unpair()
        assertEquals(PairingState.NotPaired, controller.state)
        assertEquals(PairingStore.Loaded.Empty, store().load())
    }

    @Test
    fun `cancel returns to what is stored`() {
        val controller = PairingController(store())
        controller.begin()
        controller.cancel()
        assertEquals(PairingState.NotPaired, controller.state)
        controller.submit(printed)
        controller.submit("not a pairing string")
        controller.cancel()
        assertEquals(PairingState.Paired(pairing), controller.state, "a failed re-pair does not unpair")
    }

    @Test
    fun `an unreadable saved pairing is reported and forgotten`() {
        File(dir, "pairing.sealed").writeBytes(byteArrayOf(1, 2, 3))
        val state = PairingController(store()).state
        assertTrue(state is PairingState.Failed && state.reason.contains("unreadable"), "$state")
        assertEquals(PairingStore.Loaded.Empty, store().load())
    }

    @Test
    fun `a pairing that cannot be saved is a failure that says so, not a pairing`() {
        val controller = PairingController(PairingStore(File(File(dir, "missing"), "p"), AesGcmSealer { key }))
        controller.submit(printed)
        val state = controller.state
        assertTrue(state is PairingState.Failed && state.reason.contains("could not be saved"), "$state")
    }

    private companion object {
        const val AES_BITS = 256
    }
}
