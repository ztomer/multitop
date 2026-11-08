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

/** The outcome of one write: the server's status and body, or why it was not reached. */
sealed interface Sent {
    data class Answered(
        val code: Int,
        val body: String,
    ) : Sent

    data class Failed(
        val reason: String,
    ) : Sent
}

/**
 * The API of the paired `multitop --serve`: `/api/health` and
 * `/api/hosts`, and A3's push routes, over [PinnedTls] and nothing else. The socket factory trusts
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

    /**
     * A3 push: one POST to the paired server, answered with what it said.
     * Never throws: a failure is [Sent.Failed] in words a person can act on.
     */
    fun post(
        path: String,
        json: String,
    ): Sent =
        try {
            val (code, body) = exchange("POST", path, json)
            Sent.Answered(code, body)
        } catch (e: Refused) {
            Sent.Failed(e.message.orEmpty())
        } catch (e: SSLException) {
            Sent.Failed("the server at $where did not present the paired certificate (${e.javaClass.simpleName})")
        } catch (e: IOException) {
            Sent.Failed("could not reach $where: ${e.message ?: e.javaClass.simpleName}")
        }

    /** The A3 push status (`GET /api/push`), never throwing. */
    fun pushStatus(): Sent =
        try {
            Sent.Answered(HttpURLConnection.HTTP_OK, get("/api/push"))
        } catch (e: IOException) {
            Sent.Failed(e.message ?: e.javaClass.simpleName)
        }

    private fun get(path: String): String {
        val (code, body) = exchange("GET", path, null)
        if (code != HttpURLConnection.HTTP_OK) throw Refused("the server at $where answered $path with HTTP $code")
        return body
    }

    private fun exchange(
        method: String,
        path: String,
        json: String?,
    ): Pair<Int, String> {
        val url = URI("https", null, bracketed(pairing.host), pairing.port, path, null, null).toURL()
        val connection = url.openConnection() as? HttpsURLConnection ?: throw IOException("$url is not HTTPS")
        connection.sslSocketFactory = tls
        connection.hostnameVerifier = PinnedTls.verifier(pairing.pin)
        connection.connectTimeout = timeoutMs
        connection.readTimeout = timeoutMs
        connection.instanceFollowRedirects = false
        connection.useCaches = false
        connection.requestMethod = method
        connection.setRequestProperty("Authorization", "Bearer ${pairing.token}")
        connection.setRequestProperty("Accept", "application/json")
        try {
            if (json != null) {
                connection.doOutput = true
                connection.setRequestProperty("Content-Type", "application/json")
                connection.outputStream.use { it.write(json.toByteArray(Charsets.UTF_8)) }
            }
            val code = connection.responseCode
            if (code == HttpURLConnection.HTTP_UNAUTHORIZED) {
                throw Refused("the server at $where refused this phone's token: pair again")
            }
            val stream = if (code < HttpURLConnection.HTTP_BAD_REQUEST) connection.inputStream else connection.errorStream
            val body = stream?.use { it.readBytes().toString(Charsets.UTF_8) }.orEmpty()
            return code to body
        } finally {
            connection.disconnect()
        }
    }

    private companion object {
        const val TIMEOUT_MS = 10_000

        fun bracketed(host: String): String = if (':' in host) "[$host]" else host
    }
}
