package com.ztomer.multitop.android.pairing

import java.security.MessageDigest
import java.security.cert.X509Certificate

/**
 * The SHA-256 of one certificate's DER encoding: the whole of what the client
 * trusts (docs/roadmap.md, A2-TLS). The server's certificate is self-signed, so
 * there is no CA to ask; the pin taken at pairing IS the trust anchor, the way
 * an ssh `known_hosts` line is.
 *
 * It pins the certificate, not its public key: the server reuses one
 * certificate across restarts, and a new certificate is a new pairing.
 */
class CertPin private constructor(
    private val digest: ByteArray,
) {
    /** 64 lowercase hex digits, the form the pairing payload carries. */
    val hex: String get() = digest.joinToString("") { "%02x".format(it) }

    /** Constant-time: a comparison that stops at the first differing byte leaks how much matched. */
    fun matches(cert: X509Certificate): Boolean = MessageDigest.isEqual(digest, sha256(cert.encoded))

    override fun equals(other: Any?): Boolean = other is CertPin && MessageDigest.isEqual(digest, other.digest)

    override fun hashCode(): Int = digest.contentHashCode()

    override fun toString(): String = "CertPin(sha256=$hex)"

    companion object {
        const val HEX_LEN = 64
        private val HEX = Regex("[0-9a-f]{$HEX_LEN}")

        fun of(cert: X509Certificate): CertPin = CertPin(sha256(cert.encoded))

        /** The pin from its payload form, or null when [hex] is not exactly 64 lowercase hex digits. */
        fun fromHex(hex: String): CertPin? {
            if (!HEX.matches(hex)) return null
            return CertPin(ByteArray(HEX_LEN / 2) { i -> hex.substring(i * 2, i * 2 + 2).toInt(HEX_RADIX).toByte() })
        }

        private const val HEX_RADIX = 16
    }
}

internal fun sha256(bytes: ByteArray): ByteArray = MessageDigest.getInstance("SHA-256").digest(bytes)
