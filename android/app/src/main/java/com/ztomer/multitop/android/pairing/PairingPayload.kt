package com.ztomer.multitop.android.pairing

import java.io.ByteArrayInputStream
import java.security.cert.CertificateException
import java.security.cert.CertificateFactory
import java.security.cert.X509Certificate
import java.util.Base64

/** What pairing yields: where the server is, how to authenticate, and the one certificate to trust. */
data class Pairing(
    val host: String,
    val port: Int,
    val token: String,
    val certificate: X509Certificate,
) {
    /** The certificate's SHA-256: what the server prints and the app shows for a human to compare. */
    val pin: CertPin get() = CertPin.of(certificate)

    /** The token is a credential: never in a log line or a crash report. */
    override fun toString(): String = "Pairing(host=$host, port=$port, pin=$pin)"
}

/**
 * The pairing payload, one string (or the QR code of it) printed by
 * `multitop --serve` on a non-loopback bind:
 *
 *     multitop://pair?v=1&addr=<host>:<port>&token=<64 hex>&sha256=<64 hex>&cert=<base64url DER>
 *
 * `host` is an IPv4 address, a DNS name, or an IPv6 address in brackets;
 * `token` is the `--serve` bearer token (256 bits, hex); `cert` is the server's
 * self-signed certificate (unpadded base64url of its DER, ~530 characters for a
 * P-256 one: a dense but ordinary QR code); `sha256` is that certificate's
 * fingerprint, lowercase hex without colons, and must agree with it.
 *
 * Why the certificate travels and not only its fingerprint: the client trusts
 * through the PLATFORM trust manager over a store holding the certificate
 * ([PinnedTls]), and a fingerprint alone could only be enforced by a hand-written
 * trust manager. The fingerprint stays, as the short form a human compares
 * against the one `multitop --serve` prints.
 *
 * Strict by design: every field exactly once, nothing unknown, one canonical
 * spelling of each. A payload that is almost right is refused with the reason,
 * never repaired -- a guessed pin is no pin.
 */
object PairingPayload {
    sealed interface Result

    data class Parsed(
        val pairing: Pairing,
    ) : Result

    data class Refused(
        val reason: String,
    ) : Result

    const val PREFIX = "multitop://pair?"
    const val VERSION = "1"
    private val FIELDS = setOf("v", "addr", "token", "sha256", "cert")
    private val TOKEN = Regex("[0-9a-f]{64}")
    private val NAME = Regex("[A-Za-z0-9.-]+")
    private val IPV6 = Regex("\\[([0-9A-Fa-f:.]+)]")
    private const val MAX_PORT = 65_535

    /** Unpadded base64url, bounded: a P-256 certificate is ~530 characters, an RSA-4096 one under 2,000. */
    private val BASE64URL = Regex("[A-Za-z0-9_-]{1,4096}")

    fun parse(text: String): Result {
        val payload = text.trim()
        if (!payload.startsWith(PREFIX)) return Refused("not a multitop pairing string (expected $PREFIX...)")
        val fields = mutableMapOf<String, String>()
        for (part in payload.removePrefix(PREFIX).split('&')) {
            val key = part.substringBefore('=', missingDelimiterValue = "")
            if (key !in FIELDS) return Refused("unknown or malformed field: ${part.substringBefore('=')}")
            if (fields.put(key, part.substringAfter('=')) != null) return Refused("field given twice: $key")
        }
        FIELDS.firstOrNull { it !in fields }?.let { return Refused("missing field: $it") }
        return fromFields(fields)
    }

    private fun fromFields(fields: Map<String, String>): Result {
        val version = fields.getValue("v")
        val token = fields.getValue("token")
        val pin = CertPin.fromHex(fields.getValue("sha256"))
        val address = address(fields.getValue("addr"))
        val cert = certificate(fields.getValue("cert"))
        return when {
            version != VERSION -> Refused("unsupported pairing version $version (this app reads $VERSION)")
            !TOKEN.matches(token) -> Refused("token must be 64 lowercase hex digits")
            pin == null -> Refused("sha256 must be 64 lowercase hex digits, without colons")
            address == null -> Refused("addr must be <host>:<port> with a port from 1 to $MAX_PORT")
            cert == null -> Refused("cert is not an X.509 certificate in unpadded base64url")
            !pin.matches(cert) -> Refused("sha256 does not match cert: the pairing string is damaged or altered")
            else -> Parsed(Pairing(address.first, address.second, token, cert))
        }
    }

    private fun certificate(text: String): X509Certificate? {
        if (!BASE64URL.matches(text)) return null
        val der = Base64.getUrlDecoder().decode(text)
        return try {
            CertificateFactory.getInstance("X.509").generateCertificate(ByteArrayInputStream(der)) as? X509Certificate
        } catch (_: CertificateException) {
            null
        }
    }

    private fun address(addr: String): Pair<String, Int>? {
        val host = addr.substringBeforeLast(':', missingDelimiterValue = "")
        val port = addr.substringAfterLast(':').toIntOrNull()?.takeIf { it in 1..MAX_PORT }
        val bare = IPV6.matchEntire(host)?.groupValues?.get(1) ?: host.takeIf { NAME.matches(it) }
        return if (bare == null || port == null) null else bare to port
    }
}
