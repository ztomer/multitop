package com.ztomer.multitop.android.pairing

import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test

class PairingPayloadTest {
    private val token = "ab".repeat(TOKEN_HEX_LEN / 2)
    private val cert = Fixtures.payloadCert("server")
    private val good =
        "multitop://pair?v=1&addr=192.168.1.20:8443&token=$token&sha256=${Fixtures.SERVER_SHA256}&cert=$cert"

    private fun refused(text: String): PairingPayload.Refused {
        val result = PairingPayload.parse(text)
        assertTrue(result is PairingPayload.Refused, "accepted a payload it must refuse: $text")
        return result as PairingPayload.Refused
    }

    @Test
    fun `a well-formed payload yields the address, token and pin`() {
        val result = PairingPayload.parse(good)
        assertTrue(result is PairingPayload.Parsed, "refused a good payload: $result")
        val pairing = (result as PairingPayload.Parsed).pairing
        assertEquals("192.168.1.20", pairing.host)
        assertEquals(PORT, pairing.port)
        assertEquals(token, pairing.token)
        assertEquals(Fixtures.SERVER_SHA256, pairing.pin.hex)
        assertEquals(Fixtures.certificate("server"), pairing.certificate)
    }

    @Test
    fun `a fingerprint that disagrees with the certificate is refused`() {
        val result = refused(good.replace(Fixtures.SERVER_SHA256, Fixtures.OTHER_SHA256))
        assertTrue(result.reason.contains("does not match"), result.reason)
    }

    @Test
    fun `a certificate that is not one, or not base64url, is refused`() {
        refused(good.replace(cert, cert.drop(CHOP)))
        refused(good.replace(cert, "$cert=="))
        refused(good.replace(cert, "$cert+"))
    }

    @Test
    fun `the token never reaches toString`() {
        val pairing = (PairingPayload.parse(good) as PairingPayload.Parsed).pairing
        assertTrue(token !in pairing.toString())
    }

    @Test
    fun `a bracketed IPv6 address keeps its host without the brackets`() {
        val result = PairingPayload.parse(good.replace("192.168.1.20", "[fd00::20]"))
        assertEquals("fd00::20", (result as PairingPayload.Parsed).pairing.host)
    }

    @Test
    fun `surrounding whitespace from a paste is tolerated, nothing else is`() {
        assertTrue(PairingPayload.parse("  $good\n") is PairingPayload.Parsed)
    }

    @Test
    fun `a payload without a pin is refused -- no pin, no pairing`() {
        refused(good.replace(Regex("&sha256=[^&]*"), ""))
    }

    @Test
    fun `each required field is required`() {
        for (field in listOf("v", "addr", "token", "sha256", "cert")) {
            val without = good.replace(Regex("[?&]$field=[^&]*"), "").replace("pair&", "pair?")
            refused(without)
        }
    }

    @Test
    fun `a field given twice is refused rather than one of them picked`() {
        refused("$good&sha256=${Fixtures.OTHER_SHA256}")
    }

    @Test
    fun `an unknown field or version is refused, not ignored`() {
        refused("$good&insecure=1")
        refused(good.replace("v=1", "v=2"))
    }

    @Test
    fun `the scheme and path are exact`() {
        refused(good.replace("multitop://", "https://"))
        refused(good.replace("//pair?", "//pairing?"))
    }

    @Test
    fun `a pin that is not 64 lowercase hex digits is refused`() {
        refused(good.replace(Fixtures.SERVER_SHA256, Fixtures.SERVER_SHA256.dropLast(2)))
        refused(good.replace(Fixtures.SERVER_SHA256, Fixtures.SERVER_SHA256.uppercase()))
        refused(good.replace(Fixtures.SERVER_SHA256, Fixtures.SERVER_SHA256.chunked(2).joinToString(":")))
    }

    @Test
    fun `a token that is not 64 lowercase hex digits is refused`() {
        refused(good.replace(token, token.drop(2)))
        refused(good.replace(token, "zz" + token.drop(2)))
    }

    @Test
    fun `an address needs a host and a port in range`() {
        refused(good.replace("192.168.1.20:8443", "192.168.1.20"))
        refused(good.replace("192.168.1.20:8443", ":8443"))
        refused(good.replace(":8443", ":0"))
        refused(good.replace(":8443", ":65536"))
        refused(good.replace(":8443", ":84x3"))
        refused(good.replace("192.168.1.20", "host name"))
    }

    @Test
    fun `a refusal says why`() {
        assertTrue(refused(good.replace(Regex("&sha256=[^&]*"), "")).reason.contains("sha256"))
    }

    private companion object {
        const val TOKEN_HEX_LEN = 64
        const val PORT = 8443
        const val CHOP = 40
    }
}
