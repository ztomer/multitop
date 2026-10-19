package com.ztomer.multitop.android.store

import com.ztomer.multitop.android.pairing.Fixtures
import com.ztomer.multitop.android.pairing.PairingPayload
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertFalse
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test
import org.junit.jupiter.api.io.TempDir
import java.io.File
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey

/** The store over the real cipher with a software key in place of the Keystore's. */
class PairingStoreTest {
    @TempDir
    lateinit var dir: File

    private val printed = Fixtures.text("rust_server_pairing.txt").trim()
    private val pairing = (PairingPayload.parse(printed) as PairingPayload.Parsed).pairing
    private val key = newKey()

    private fun newKey(): SecretKey = KeyGenerator.getInstance("AES").apply { init(AES_BITS) }.generateKey()

    private fun file() = File(dir, "pairing.sealed")

    private fun store(with: SecretKey = key) = PairingStore(file(), AesGcmSealer { with })

    @Test
    fun `the canonical form of the server's pairing string is the string itself`() {
        assertEquals(printed, PairingPayload.format(pairing))
    }

    @Test
    fun `a saved pairing loads back equal, in a fresh store over the same key`() {
        store().save(pairing)
        assertEquals(PairingStore.Loaded.Found(pairing), store().load())
    }

    @Test
    fun `nothing of the pairing is on disk in the clear`() {
        store().save(pairing)
        val bytes = file().readBytes().toString(Charsets.ISO_8859_1)
        for (secret in listOf(pairing.token, pairing.pin.hex, "multitop://", pairing.host)) {
            assertFalse(bytes.contains(secret), "found '$secret' in the clear")
        }
        assertFalse(File(dir, "pairing.sealed.partial").exists(), "the partial write was left behind")
    }

    @Test
    fun `unpair clears it, and a store with nothing saved loads empty`() {
        assertEquals(PairingStore.Loaded.Empty, store().load())
        store().save(pairing)
        assertTrue(store().clear())
        assertFalse(file().exists())
        assertEquals(PairingStore.Loaded.Empty, store().load())
        assertTrue(store().clear(), "clearing nothing is not a failure")
    }

    @Test
    fun `a different key -- a reinstall, a wiped Keystore -- reads as unreadable, not as a pairing`() {
        store().save(pairing)
        val loaded = store(newKey()).load()
        assertTrue(loaded is PairingStore.Loaded.Unreadable, "$loaded")
    }

    @Test
    fun `a flipped bit is refused by the cipher's tag`() {
        store().save(pairing)
        val bytes = file().readBytes()
        bytes[bytes.size - 1] = (bytes[bytes.size - 1].toInt() xor 1).toByte()
        file().writeBytes(bytes)
        assertTrue(store().load() is PairingStore.Loaded.Unreadable)
    }

    @Test
    fun `a file that is not a sealed record is unreadable`() {
        file().writeBytes(ByteArray(0))
        assertTrue(store().load() is PairingStore.Loaded.Unreadable)
        file().writeBytes(byteArrayOf(2, 0, 0))
        assertTrue(store().load() is PairingStore.Loaded.Unreadable)
    }

    @Test
    fun `a sealed string the pairing rules refuse is unreadable, with the rule's reason`() {
        file().writeBytes(AesGcmSealer { key }.seal("multitop://pair?v=2".toByteArray()))
        val loaded = store().load()
        assertTrue(loaded is PairingStore.Loaded.Unreadable && loaded.reason.contains("field"), "$loaded")
    }

    @Test
    fun `a save that cannot write says so`() {
        val blocked = PairingStore(File(File(dir, "missing"), "pairing.sealed"), AesGcmSealer { key })
        org.junit.jupiter.api
            .assertThrows<java.io.IOException> { blocked.save(pairing) }
    }

    private companion object {
        const val AES_BITS = 256
    }
}
