package com.ztomer.multitop.android.push

import org.json.JSONException
import org.json.JSONObject

/** One alert, as the server sends it on either route. */
data class Alert(
    val kind: String,
    val host: String,
    val title: String,
    val body: String,
)

/**
 * The wire, both directions: the registration this phone POSTs to
 * `/api/push/register`, and the alert payload the server pushes back
 * (`crates/multitop/src/push/mod.rs`, `Alert`, version 1).
 */
object PushJson {
    const val PAYLOAD_VERSION = 1
    private val KINDS = setOf("down", "degraded", "recovered", "test")

    fun unifiedPush(
        endpoint: String,
        p256dh: String,
        auth: String,
    ): String =
        JSONObject()
            .put("kind", "unifiedpush")
            .put("endpoint", endpoint)
            .put("p256dh", p256dh)
            .put("auth", auth)
            .toString()

    fun fcm(token: String): String = JSONObject().put("kind", "fcm").put("token", token).toString()

    /**
     * The alert, or null for anything this version does not draw: another
     * payload version, an unknown kind, a missing title. Never drawn half-read.
     */
    fun alert(text: String): Alert? =
        try {
            val o = JSONObject(text)
            val kind = o.optString("kind")
            val title = o.optString("title")
            if (o.optInt("v") != PAYLOAD_VERSION || kind !in KINDS || title.isEmpty()) {
                null
            } else {
                Alert(kind, o.optString("host"), title, o.optString("body"))
            }
        } catch (_: JSONException) {
            null
        }
}
