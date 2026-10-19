package com.ztomer.multitop.android.pairing

import java.security.KeyStore
import java.security.cert.X509Certificate
import javax.net.ssl.HostnameVerifier
import javax.net.ssl.SSLContext
import javax.net.ssl.SSLSession
import javax.net.ssl.TrustManagerFactory
import javax.net.ssl.X509TrustManager

/**
 * TLS that trusts exactly one server certificate -- the one pinned at pairing --
 * and nothing else: not the system CAs, not a user-installed CA.
 *
 * Two layers, and neither is a hand-written `X509TrustManager` (Android lint's
 * CustomX509TrustManager: a custom one is the classic way to disable validation
 * by accident). The PLATFORM trust manager does the validating, over a trust
 * store that holds only the pinned certificate. There is no unpinned form: a
 * context cannot be built without a certificate, so an unpaired app has no way
 * to open a connection, and [verifier] without a pin refuses every session.
 * The platform alone would also accept a
 * certificate the pinned key had SIGNED -- a trust anchor vouches for what it
 * issues -- so [verifier] then requires the presented leaf to BE the pinned
 * certificate, byte for byte by its SHA-256.
 *
 * Validity dates are not the gate: the pin is the trust decision, as with an ssh
 * host key, and a self-signed certificate's dates are whatever its maker wrote.
 */
object PinnedTls {
    /** The platform's own X.509 trust manager over a store holding only [pinned]. */
    fun trustManager(pinned: X509Certificate): X509TrustManager {
        val store = KeyStore.getInstance(KeyStore.getDefaultType()).apply { load(null, null) }
        store.setCertificateEntry(ALIAS, pinned)
        val factory = TrustManagerFactory.getInstance(TrustManagerFactory.getDefaultAlgorithm())
        factory.init(store)
        return factory.trustManagers.filterIsInstance<X509TrustManager>().single()
    }

    /** An [SSLContext] whose only trust is [trustManager] over [pinned]. */
    fun context(pinned: X509Certificate): SSLContext =
        SSLContext.getInstance("TLS").apply {
            init(null, arrayOf(trustManager(pinned)), null)
        }

    /**
     * Accepts a session only when its leaf certificate is exactly [pin]. It
     * ignores the host name on purpose: the server is reached by a LAN address
     * that DHCP may change, and the pin -- not the name -- is what identifies it.
     */
    fun verifier(pin: CertPin?): HostnameVerifier = HostnameVerifier { _, session -> leafMatches(pin, session) }

    private fun leafMatches(
        pin: CertPin?,
        session: SSLSession,
    ): Boolean {
        val leaf = runCatching { session.peerCertificates.firstOrNull() }.getOrNull()
        return pin != null && leaf is X509Certificate && pin.matches(leaf)
    }

    private const val ALIAS = "multitop-pinned"
}
