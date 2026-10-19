package com.ztomer.multitop.android.store

import java.nio.ByteBuffer
import java.security.GeneralSecurityException
import javax.crypto.Cipher
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * Encrypts what the app keeps on disk. The seam between the store and the key:
 * on the phone the key lives in the Android Keystore ([KeystoreKey]) and never
 * leaves it; the JVM tests hand [AesGcmSealer] a software key and so exercise
 * the same cipher, the same framing and the same tamper check.
 */
interface Sealer {
    fun seal(plain: ByteArray): ByteArray

    /** The plaintext, or a [GeneralSecurityException] when [sealed] was not made by this key or was altered. */
    fun open(sealed: ByteArray): ByteArray
}

/**
 * AES-256-GCM, a fresh IV per seal chosen by the cipher's provider (the
 * Keystore insists on that), framed as `[iv length][iv][ciphertext + tag]`.
 * GCM authenticates: a flipped bit or a different key fails [open], it does not
 * decrypt to garbage.
 */
class AesGcmSealer(
    private val key: () -> SecretKey,
) : Sealer {
    override fun seal(plain: ByteArray): ByteArray {
        val cipher = Cipher.getInstance(TRANSFORMATION).apply { init(Cipher.ENCRYPT_MODE, key()) }
        val iv = cipher.iv
        val body = cipher.doFinal(plain)
        return ByteBuffer
            .allocate(1 + iv.size + body.size)
            .put(iv.size.toByte())
            .put(iv)
            .put(body)
            .array()
    }

    override fun open(sealed: ByteArray): ByteArray {
        val ivLen = sealed.firstOrNull()?.toInt() ?: throw GeneralSecurityException("empty")
        if (ivLen !in MIN_IV..MAX_IV || sealed.size < 1 + ivLen + TAG_BYTES) {
            throw GeneralSecurityException("not a sealed record")
        }
        val iv = sealed.copyOfRange(1, 1 + ivLen)
        val cipher =
            Cipher.getInstance(TRANSFORMATION).apply {
                init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(TAG_BYTES * Byte.SIZE_BITS, iv))
            }
        return cipher.doFinal(sealed, 1 + ivLen, sealed.size - 1 - ivLen)
    }

    private companion object {
        const val TRANSFORMATION = "AES/GCM/NoPadding"
        const val TAG_BYTES = 16
        const val MIN_IV = 12
        const val MAX_IV = 16
    }
}
