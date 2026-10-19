package com.ztomer.multitop.android.pairing

import java.io.ByteArrayInputStream
import java.security.KeyFactory
import java.security.KeyStore
import java.security.cert.CertificateFactory
import java.security.cert.X509Certificate
import java.security.spec.PKCS8EncodedKeySpec
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
 *
 * `rust_server` is the other direction: a certificate and key the SERVER made
 * (`crates/multitop/src/serve_tls.rs`), PEM, beside the pairing string the server
 * printed for it (`rust_server_pairing.txt`). Both are written by the Rust test
 * `the_golden_pairing_string_is_what_the_server_prints`, which also fails when
 * the server would print anything else -- so neither half can drift alone.
 */
object Fixtures {
    const val PASSWORD = "fixture"
    const val SERVER_SHA256 = "259baf4ae4da56e9e86fd5275393ed5ec9e4b63102060918fd9e805e713c6885"
    const val OTHER_SHA256 = "160d60cf4e2e8fd260675de9fcc083191e8a14d621cf2ba4247fe3bd70560c4f"
    const val ISSUED_SHA256 = "5e44140709db97202edfa3a8b95cc9ada7f147dc652b65d70fecadf8ff3bf273"

    const val RUST_SERVER = "rust_server"

    fun keyStore(name: String): KeyStore = if (name == RUST_SERVER) pemKeyStore(name) else p12KeyStore(name)

    private fun p12KeyStore(name: String): KeyStore {
        val stream =
            requireNotNull(Fixtures::class.java.getResourceAsStream("/$name.p12")) {
                "missing test resource $name.p12"
            }
        return stream.use { input ->
            KeyStore.getInstance("PKCS12").apply { load(input, PASSWORD.toCharArray()) }
        }
    }

    fun text(resource: String): String =
        requireNotNull(Fixtures::class.java.getResourceAsStream("/$resource")) {
            "missing test resource $resource"
        }.use { it.readBytes().toString(Charsets.UTF_8) }

    /** The PEM the server writes -- a CERTIFICATE then a PKCS#8 PRIVATE KEY -- as a key store. */
    private fun pemKeyStore(name: String): KeyStore {
        val pem = text("$name.pem")
        val cert =
            CertificateFactory
                .getInstance("X.509")
                .generateCertificate(ByteArrayInputStream(pemBlock(pem, "CERTIFICATE"))) as X509Certificate
        val key = KeyFactory.getInstance("EC").generatePrivate(PKCS8EncodedKeySpec(pemBlock(pem, "PRIVATE KEY")))
        return KeyStore.getInstance("PKCS12").apply {
            load(null, null)
            setKeyEntry(name, key, PASSWORD.toCharArray(), arrayOf(cert))
        }
    }

    private fun pemBlock(
        pem: String,
        label: String,
    ): ByteArray {
        val body = pem.substringAfter("-----BEGIN $label-----").substringBefore("-----END $label-----")
        return Base64.getMimeDecoder().decode(body)
    }

    fun certificate(name: String): X509Certificate = keyStore(name).getCertificate(name) as X509Certificate

    /** The payload form of a certificate: unpadded base64url of its DER. */
    fun payloadCert(name: String): String = Base64.getUrlEncoder().withoutPadding().encodeToString(certificate(name).encoded)
}
