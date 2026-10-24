package com.ztomer.multitop.android.api

import org.json.JSONArray
import org.json.JSONException
import org.json.JSONObject

/** One row of `/api/hosts`: what the server knows about one host it watches. */
data class HostRow(
    val host: String,
    /** The config's `name = "..."`, when the host has one. */
    val name: String?,
    val port: Int,
    /** Whether the server has read this host at all. */
    val hasReading: Boolean,
    /** 0-100; null until the host has been read (the server sends no score for nothing measured). */
    val health: Int?,
    val breaching: Boolean,
    val reachable: Boolean,
) {
    /** What a person calls the host: its name, else its address. */
    val label: String get() = name ?: host
}

/** `/api/health`: the server's own count, never re-derived here. */
data class HealthSummary(
    val total: Int,
    val healthy: Int,
    val breaching: Int,
    val unread: Int,
)

/** Both read-only endpoints, read together. */
data class Overview(
    val health: HealthSummary,
    val hosts: List<HostRow>,
)

/**
 * The two endpoints' JSON, read strictly: a field that is missing or of the
 * wrong type is refused with its name ([Malformed]), never defaulted -- a
 * defaulted `health` is exactly the made-up score the server stopped sending.
 */
object OverviewJson {
    class Malformed(
        message: String,
    ) : Exception(message)

    fun health(text: String): HealthSummary =
        read("/api/health") {
            val o = JSONObject(text)
            HealthSummary(
                total = o.int("total"),
                healthy = o.int("healthy"),
                breaching = o.int("breaching"),
                unread = o.int("unread"),
            )
        }

    fun hosts(text: String): List<HostRow> =
        read("/api/hosts") {
            val array = JSONArray(text)
            (0 until array.length()).map { i -> row(array.getJSONObject(i)) }
        }

    private fun row(o: JSONObject): HostRow =
        HostRow(
            host = o.string("host"),
            name = if (o.isNull("name")) null else o.string("name"),
            port = o.int("port"),
            hasReading = o.bool("has_snapshot"),
            health = if (o.isNull("health")) null else o.int("health"),
            breaching = o.bool("breaching"),
            reachable = o.bool("reachable"),
        )

    private fun <T> read(
        endpoint: String,
        parse: () -> T,
    ): T =
        try {
            parse()
        } catch (e: JSONException) {
            throw Malformed("$endpoint: ${e.message}")
        }

    private fun JSONObject.need(key: String): Any {
        if (!has(key)) throw JSONException("missing field $key")
        return get(key)
    }

    private fun JSONObject.int(key: String): Int = need(key) as? Int ?: throw JSONException("$key is not a whole number")

    private fun JSONObject.bool(key: String): Boolean = need(key) as? Boolean ?: throw JSONException("$key is not true or false")

    private fun JSONObject.string(key: String): String = need(key) as? String ?: throw JSONException("$key is not a string")
}
