package com.ztomer.multitop.android.push

import com.ztomer.multitop.android.api.Sent
import org.json.JSONException
import org.json.JSONObject

/**
 * The push panel's words, in one place, so the tests assert what the person
 * reads. Alarm (✗) is kept for a delivery that failed; a step not taken yet
 * is a status (·) that names the step.
 */
object PushCopy {
    const val TEST = "Send a test alert"
    const val READING = "→ Reading the alert route"
    const val REGISTERING = "→ Registering this phone for alerts"
    const val NO_DISTRIBUTOR = "· Alerts need a push distributor: install ntfy (UnifiedPush) on this phone"
    const val NOTHING_REGISTERED = "· The server has no route to this phone yet"
    const val FCM_NO_ACCOUNT =
        "· This phone registered for FCM, but the server has no fcm-service-account.json beside its config"

    private fun via(route: String): String =
        when (route) {
            "unifiedpush" -> "UnifiedPush (ntfy)"
            "fcm" -> "FCM"
            else -> route
        }

    /** The route the server will use, from `GET /api/push` and what this phone chose. */
    fun status(
        answer: Sent,
        choice: PushChoice,
    ): String {
        val o =
            when (answer) {
                is Sent.Failed -> return "✗ Could not read the alert route: ${answer.reason}"
                is Sent.Answered -> parse(answer.body) ?: return "✗ The server answered something this app cannot read"
            }
        val route = o.optString("route").takeUnless { o.isNull("route") || it.isEmpty() }
        return when {
            route != null -> "✓ Alerts reach this phone through ${via(route)}"
            choice == PushChoice.None -> NO_DISTRIBUTOR
            o.optBoolean("fcm") && !o.optBoolean("fcm_ready") -> FCM_NO_ACCOUNT
            else -> REGISTERING
        }
    }

    /** What `POST /api/push/test` did. */
    fun tested(answer: Sent): String =
        when (answer) {
            is Sent.Failed -> {
                "✗ The test alert was not sent: ${answer.reason}"
            }

            is Sent.Answered -> {
                val o = parse(answer.body)
                when {
                    answer.code == HTTP_OK -> "✓ Test alert sent through ${via(o?.optString("via").orEmpty())}"
                    answer.code == HTTP_CONFLICT -> NOTHING_REGISTERED
                    else -> "✗ The test alert was not delivered: ${failures(o) ?: "HTTP ${answer.code}"}"
                }
            }
        }

    private fun failures(o: JSONObject?): String? {
        val list = o?.optJSONArray("failed") ?: return null
        return (0 until list.length())
            .mapNotNull { list.optJSONArray(it) }
            .joinToString("; ") { "${via(it.optString(0))}: ${it.optString(1)}" }
            .ifEmpty { null }
    }

    private fun parse(body: String): JSONObject? =
        try {
            JSONObject(body)
        } catch (_: JSONException) {
            null
        }

    private const val HTTP_OK = 200
    private const val HTTP_CONFLICT = 409
}
