package com.ztomer.multitop.android.store

import com.ztomer.multitop.android.pairing.Pairing
import com.ztomer.multitop.android.pairing.PairingPayload
import java.io.File
import java.io.IOException
import java.security.GeneralSecurityException

/**
 * The one pairing this phone holds: server address, bearer token and pinned
 * certificate, as the canonical pairing string ([PairingPayload.format]),
 * sealed by [sealer] into one file. Nothing is written in the clear, and the
 * file lives in `noBackupFilesDir`, which the manifest's backup rules also
 * exclude.
 *
 * Reading re-parses the string by the pairing rules, so what is stored is held
 * to exactly what a fresh scan is.
 */
class PairingStore(
    private val file: File,
    private val sealer: Sealer,
) {
    sealed interface Loaded {
        data object Empty : Loaded

        data class Found(
            val pairing: Pairing,
        ) : Loaded

        data class Unreadable(
            val reason: String,
        ) : Loaded
    }

    /** Replaces any stored pairing; written beside and renamed over, so a crash leaves the old one or the new. */
    @Throws(IOException::class, GeneralSecurityException::class)
    fun save(pairing: Pairing) {
        val sealed = sealer.seal(PairingPayload.format(pairing).toByteArray(Charsets.UTF_8))
        val partial = File(file.parentFile, "${file.name}.partial")
        partial.writeBytes(sealed)
        if (!partial.renameTo(file)) {
            partial.delete()
            throw IOException("could not replace ${file.name}")
        }
    }

    fun load(): Loaded {
        if (!file.exists()) return Loaded.Empty
        val text =
            try {
                sealer.open(file.readBytes()).toString(Charsets.UTF_8)
            } catch (e: GeneralSecurityException) {
                return Loaded.Unreadable("it cannot be decrypted with this phone's key (${e.javaClass.simpleName})")
            } catch (e: IOException) {
                return Loaded.Unreadable("it cannot be read (${e.message})")
            }
        return when (val parsed = PairingPayload.parse(text)) {
            is PairingPayload.Parsed -> Loaded.Found(parsed.pairing)
            is PairingPayload.Refused -> Loaded.Unreadable(parsed.reason)
        }
    }

    /** Forgets the pairing. True when nothing is left on disk. */
    fun clear(): Boolean = !file.exists() || file.delete()
}
