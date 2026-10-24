package com.ztomer.multitop.android.api

import com.ztomer.multitop.android.pairing.Fixtures
import com.ztomer.multitop.android.pairing.Pairing
import com.ztomer.multitop.android.pairing.PairingPayload
import org.junit.jupiter.api.AfterEach
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertNull
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.BeforeEach
import org.junit.jupiter.api.Test
import org.junit.jupiter.api.fail
import org.junit.jupiter.api.io.TempDir
import java.io.ByteArrayInputStream
import java.io.File
import java.net.InetSocketAddress
import java.net.ServerSocket
import java.net.Socket
import java.security.cert.CertificateFactory
import java.security.cert.X509Certificate
import java.util.Base64
import java.util.concurrent.TimeUnit

/**
 * A2.5's proof: the app's own client ([ServerApi], over [com.ztomer.multitop.android.pairing.PinnedTls])
 * against a real `multitop --serve` this test starts, on a NON-loopback bind so
 * the server speaks TLS (loopback stays plain HTTP and would prove nothing).
 *
 * The binary is `MULTITOP_BIN`, which `tools/android_gate.sh` builds and sets.
 * Absent, this FAILS with how to fix it rather than skipping: a skipped
 * end-to-end test is the gate that passes without having looked.
 *
 * The one host the server watches is this machine (`127.0.0.1`, which the
 * server reads in-process): a remote host would need the cross-compiled agents
 * a CI build does not embed, and `--serve` rightly refuses to start without
 * them. So the test reaches nothing outside this machine, and it sees a real
 * reading arrive -- not read yet, then read.
 */
class ServeEndToEndTest {
    @TempDir
    lateinit var dir: File

    private lateinit var server: Process
    private lateinit var log: File
    private var port = 0

    @BeforeEach
    fun start() {
        val bin =
            System.getenv("MULTITOP_BIN")?.takeIf { File(it).canExecute() }
                ?: fail("MULTITOP_BIN is unset or not executable: run tools/android_gate.sh, which builds multitop and sets it")
        val config = File(dir, "config.toml")
        config.writeText("[[servers]]\nhost = \"$HOST\"\nport = 22\nuser = \"\"\nname = \"$NAME\"\n")
        port = ServerSocket(0).use { it.localPort }
        log = File(dir, "serve.log")
        server =
            ProcessBuilder(
                bin,
                "-c",
                config.path,
                "--serve",
                "0.0.0.0:$port",
                "--serve-token-file",
                File(dir, "serve-token").path,
            ).redirectInput(ProcessBuilder.Redirect.from(File("/dev/null")))
                .redirectErrorStream(true)
                .redirectOutput(log)
                .apply { environment()["MULTITOP_MOCK_KEYCHAIN"] = "1" }
                .start()
        waitFor("the server to listen on $port") { listening() }
    }

    @AfterEach
    fun stop() {
        server.destroy()
        if (!server.waitFor(STOP_S, TimeUnit.SECONDS)) {
            server.destroyForcibly().waitFor(STOP_S, TimeUnit.SECONDS)
        }
    }

    /** The pairing the phone would scan, made from what the server wrote beside its config. */
    private fun pairing(
        token: String = File(dir, "serve-token").readText().trim(),
        certificate: X509Certificate = servedCertificate(),
    ): Pairing {
        val made = Pairing("127.0.0.1", port, token, certificate)
        // Through the payload's own rules, as a scanned pairing goes.
        return (PairingPayload.parse(PairingPayload.format(made)) as PairingPayload.Parsed).pairing
    }

    @Test
    fun `the paired client reads health and hosts over pinned TLS`() {
        var last: Overview? = null
        waitFor("a reading of this machine to arrive") {
            val fetched = ServerApi(pairing()).overview()
            val read = fetched as? Fetch.Read ?: fail("expected a read, got $fetched; server log:\n${log.readText()}")
            val o = read.overview
            last = o
            // On EVERY read, before the reading arrives and after: the count is
            // the server's own and adds up, and an unread host has no score.
            assertEquals(1, o.health.total)
            assertEquals(o.health.total, o.health.healthy + o.health.breaching + o.health.unread)
            val row = o.hosts.single()
            if (!row.hasReading) assertNull(row.health, "a host not read yet has no health")
            row.hasReading
        }
        val o = last ?: fail("no read was made")
        val row = o.hosts.single()
        assertEquals(HOST, row.host)
        assertEquals(NAME, row.label)
        assertTrue(row.reachable)
        assertTrue(row.health in 0..MAX_HEALTH, "health ${row.health}")
        assertEquals(0, o.health.unread)
    }

    @Test
    fun `a client pinned to another certificate is refused before any request`() {
        val fetched = ServerApi(pairing(certificate = Fixtures.certificate("other"))).overview()
        val failed = fetched as? Fetch.Failed ?: fail("a client pinned elsewhere read the server: $fetched")
        assertTrue("did not present the paired certificate" in failed.reason, failed.reason)
    }

    @Test
    fun `a wrong token is refused and said to be the token`() {
        val fetched = ServerApi(pairing(token = "0".repeat(TOKEN_HEX))).overview()
        val failed = fetched as? Fetch.Failed ?: fail("a wrong token read the server: $fetched")
        assertTrue("refused this phone's token" in failed.reason, failed.reason)
    }

    private fun servedCertificate(): X509Certificate {
        val pem = File(dir, "serve-tls.pem").readText()
        val body = pem.substringAfter("-----BEGIN CERTIFICATE-----").substringBefore("-----END CERTIFICATE-----")
        val der = Base64.getMimeDecoder().decode(body)
        return CertificateFactory.getInstance("X.509").generateCertificate(ByteArrayInputStream(der)) as X509Certificate
    }

    private fun listening(): Boolean =
        runCatching { Socket().use { it.connect(InetSocketAddress("127.0.0.1", port), CONNECT_MS) } }.isSuccess

    private fun waitFor(
        what: String,
        ready: () -> Boolean,
    ) {
        val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(WAIT_S)
        while (System.nanoTime() < deadline) {
            if (!server.isAlive) fail("multitop --serve exited ${server.exitValue()} waiting for $what:\n${log.readText()}")
            if (ready()) return
            Thread.sleep(POLL_MS)
        }
        fail("timed out after ${WAIT_S}s waiting for $what; server log:\n${log.readText()}")
    }

    private companion object {
        const val HOST = "127.0.0.1"
        const val MAX_HEALTH = 100
        const val NAME = "e2e-host"
        const val TOKEN_HEX = 64
        const val WAIT_S = 30L
        const val STOP_S = 10L
        const val POLL_MS = 200L
        const val CONNECT_MS = 500
    }
}
