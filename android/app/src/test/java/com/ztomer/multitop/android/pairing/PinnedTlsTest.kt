package com.ztomer.multitop.android.pairing

import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertFalse
import org.junit.jupiter.api.Test
import org.junit.jupiter.api.assertThrows
import java.net.InetAddress
import java.security.cert.CertificateException
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import javax.net.ssl.KeyManagerFactory
import javax.net.ssl.SSLContext
import javax.net.ssl.SSLHandshakeException
import javax.net.ssl.SSLPeerUnverifiedException
import javax.net.ssl.SSLServerSocket
import javax.net.ssl.SSLSocket

class PinnedTlsTest {
    private val server = Fixtures.certificate("server")
    private val other = Fixtures.certificate("other")
    private val issued = Fixtures.certificate("issued")
    private val serverPin = CertPin.of(server)

    @Test
    fun `the pin is the certificate's SHA-256 as openssl prints it`() {
        assertEquals(Fixtures.SERVER_SHA256, CertPin.of(server).hex)
        assertEquals(Fixtures.OTHER_SHA256, CertPin.of(other).hex)
        assertEquals(Fixtures.ISSUED_SHA256, CertPin.of(issued).hex)
        assertEquals(serverPin, CertPin.fromHex(Fixtures.SERVER_SHA256))
    }

    @Test
    fun `the pinned certificate is accepted by the trust manager`() {
        PinnedTls.trustManager(server).checkServerTrusted(arrayOf(server), AUTH)
    }

    @Test
    fun `a different certificate is refused by the trust manager`() {
        assertThrows<CertificateException> {
            PinnedTls.trustManager(server).checkServerTrusted(arrayOf(other), AUTH)
        }
    }

    @Test
    fun `a real TLS session succeeds against the pinned certificate`() {
        assertEquals("pong", exchange(serverKey = "server", trusted = "server", pinned = "server"))
    }

    @Test
    fun `a real TLS session with a different certificate is refused`() {
        assertThrows<SSLHandshakeException> { exchange(serverKey = "other", trusted = "server", pinned = "server") }
    }

    @Test
    fun `no pin refuses every session, even one the platform accepted`() {
        // A context cannot be built without a certificate (the type says so), so
        // the only unpinned piece is the verifier, and it must refuse everything.
        for (key in listOf("server", "other")) {
            assertThrows<SSLPeerUnverifiedException> { exchange(serverKey = key, trusted = key, pinned = null) }
        }
    }

    @Test
    fun `a certificate the pinned key signed passes the platform but not the pin`() {
        // The platform trust manager accepts it -- the pinned certificate is a
        // trust anchor, and an anchor vouches for what it issues ...
        PinnedTls.trustManager(server).checkServerTrusted(arrayOf(issued, server), AUTH)
        // ... so the session is refused by the pin, which names one leaf.
        assertThrows<SSLPeerUnverifiedException> { exchange(serverKey = "issued", trusted = "server", pinned = "server") }
    }

    @Test
    fun `the verifier refuses a session that has no peer`() {
        val unconnected = SSLContext.getDefault().createSSLEngine().session
        assertFalse(PinnedTls.verifier(serverPin).verify("host", unconnected))
    }

    /**
     * One loopback TLS exchange the way the client will make it: the server
     * presents [serverKey]'s certificate; the client's trust store holds only
     * [trusted], and the session is then held to [pinned]'s fingerprint.
     */
    private fun exchange(
        serverKey: String,
        trusted: String,
        pinned: String?,
    ): String {
        val kmf = KeyManagerFactory.getInstance(KeyManagerFactory.getDefaultAlgorithm())
        kmf.init(Fixtures.keyStore(serverKey), Fixtures.PASSWORD.toCharArray())
        val serverCtx = SSLContext.getInstance("TLS").apply { init(kmf.keyManagers, null, null) }
        val loopback = InetAddress.getLoopbackAddress()
        val listener = serverCtx.serverSocketFactory.createServerSocket(0, 1, loopback) as SSLServerSocket
        val pool = Executors.newSingleThreadExecutor()
        try {
            pool.submit {
                runCatching {
                    (listener.accept() as SSLSocket).use { s ->
                        s.outputStream.write("pong\n".toByteArray())
                        s.outputStream.flush()
                    }
                }
            }
            val factory = PinnedTls.context(Fixtures.certificate(trusted)).socketFactory
            val socket = factory.createSocket(loopback, listener.localPort) as SSLSocket
            val verifier = PinnedTls.verifier(pinned?.let { CertPin.of(Fixtures.certificate(it)) })
            return socket.use { s ->
                s.soTimeout = TIMEOUT_MS
                s.startHandshake()
                if (!verifier.verify(loopback.hostAddress, s.session)) {
                    throw SSLPeerUnverifiedException("leaf is not the pinned certificate")
                }
                s.inputStream.bufferedReader().readLine()
            }
        } finally {
            listener.close()
            pool.shutdownNow()
            pool.awaitTermination(TIMEOUT_MS.toLong(), TimeUnit.MILLISECONDS)
        }
    }

    private companion object {
        const val AUTH = "ECDHE_ECDSA"
        const val TIMEOUT_MS = 5_000
    }
}
