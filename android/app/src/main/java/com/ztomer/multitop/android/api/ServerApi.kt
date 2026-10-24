package com.ztomer.multitop.android.api

import com.ztomer.multitop.android.pairing.Pairing
import com.ztomer.multitop.android.pairing.PinnedTls
import java.io.IOException
import java.net.HttpURLConnection
import java.net.URI
import javax.net.ssl.HttpsURLConnection
import javax.net.ssl.SSLException

/** The outcome of one read: the overview, or why there is none, in words a person can act on. */
sealed interface Fetch {
    data class Read(
        val overview: Overview,
    ) : Fetch

    data class Failed(
        val reason: String,
    ) : Fetch
}

/**
 * The read-only API of the paired `multitop --serve`: `/api/health` and
 * `/api/hosts`, over [PinnedTls] and nothing else. The socket factory trusts
 * only the pinned certificate and the verifier requires the leaf to BE it; the
 * host name is not checked, because the LAN address may change and the pin may
 * not. There is no plain-HTTP path to fall back to (the manifest forbids
 * cleartext as well).
 */
class ServerApi(
    private val pairing: Pairing,
    private val timeoutMs: Int = TIMEOUT_MS,
) {
    private val tls by lazy { PinnedTls.context(pairing.certificate).socketFactory }

    /** Both endpoints, or the first reason either could not be read. */
    fun overview(): Fetch =
        try {
            val health = OverviewJson.health(get("/api/health"))
            val hosts = OverviewJson.hosts(get("/api/hosts"))
            Fetch.Read(Overview(health, hosts))
        } catch (e: Refused) {
            Fetch.Failed(e.message.orEmpty())
        } catch (e: OverviewJson.Malformed) {
            Fetch.Failed("the server answered something this app cannot read (${e.message})")
        } catch (e: SSLException) {
            // A handshake the pinned trust store refused, or a leaf the pin refused.
            Fetch.Failed(
                "the server at $where did not present the paired certificate " +
                    "(${e.javaClass.simpleName}): if it was re-keyed, pair again",
            )
        } catch (e: IOException) {
            Fetch.Failed("could not reach $where: ${e.message ?: e.javaClass.simpleName}")
        }

    private val where: String
        get() = "${bracketed(pairing.host)}:${pairing.port}"

    private class Refused(
        message: String,
    ) : IOException(message)

    private fun get(path: String): String {
        val url = URI("https", null, bracketed(pairing.host), pairing.port, path, null, null).toURL()
        val connection = url.openConnection() as? HttpsURLConnection ?: throw IOException("$url is not HTTPS")
        connection.sslSocketFactory = tls
        connection.hostnameVerifier = PinnedTls.verifier(pairing.pin)
        connection.connectTimeout = timeoutMs
        connection.readTimeout = timeoutMs
        connection.instanceFollowRedirects = false
        connection.useCaches = false
        connection.setRequestProperty("Authorization", "Bearer ${pairing.token}")
        connection.setRequestProperty("Accept", "application/json")
        try {
            return when (val code = connection.responseCode) {
                HttpURLConnection.HTTP_OK -> {
                    connection.inputStream.use { it.readBytes().toString(Charsets.UTF_8) }
                }

                HttpURLConnection.HTTP_UNAUTHORIZED -> {
                    throw Refused("the server at $where refused this phone's token: pair again")
                }

                else -> {
                    throw Refused("the server at $where answered $path with HTTP $code")
                }
            }
        } finally {
            connection.disconnect()
        }
    }

    private companion object {
        const val TIMEOUT_MS = 10_000

        fun bracketed(host: String): String = if (':' in host) "[$host]" else host
    }
}
