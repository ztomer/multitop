package com.ztomer.multitop.android.pairing

import java.security.KeyStore
import java.security.cert.X509Certificate
import java.util.Base64

/**
 * Two self-signed EC P-256 certificates (`server`, `other`), each with its key,
 * made once with `openssl req -x509`, and a third (`issued`) that `server`'s key
 * SIGNED -- the certificate a bare trust-anchor check would wrongly accept. All
 * are PKCS#12 under src/test/resources: fixtures, not secrets; nothing outside
 * these tests trusts them.
 *
 * The fingerprints are what `openssl x509 -noout -fingerprint -sha256` printed
 * for each, lower-cased with the colons removed. They are written down rather
 * than computed here so the tests also hold this code to openssl's notion of a
 * certificate's SHA-256 -- the one the server half (A2-TLS) will print.
 */
object Fixtures {
    const val PASSWORD = "fixture"
    const val SERVER_SHA256 = "259baf4ae4da56e9e86fd5275393ed5ec9e4b63102060918fd9e805e713c6885"
    const val OTHER_SHA256 = "160d60cf4e2e8fd260675de9fcc083191e8a14d621cf2ba4247fe3bd70560c4f"
    const val ISSUED_SHA256 = "5e44140709db97202edfa3a8b95cc9ada7f147dc652b65d70fecadf8ff3bf273"

    fun keyStore(name: String): KeyStore {
        val stream =
            requireNotNull(Fixtures::class.java.getResourceAsStream("/$name.p12")) {
                "missing test resource $name.p12"
            }
        return stream.use { input ->
            KeyStore.getInstance("PKCS12").apply { load(input, PASSWORD.toCharArray()) }
        }
    }

    fun certificate(name: String): X509Certificate = keyStore(name).getCertificate(name) as X509Certificate

    /** The payload form of a certificate: unpadded base64url of its DER. */
    fun payloadCert(name: String): String = Base64.getUrlEncoder().withoutPadding().encodeToString(certificate(name).encoded)
}
