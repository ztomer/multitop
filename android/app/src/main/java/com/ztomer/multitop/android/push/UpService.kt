package com.ztomer.multitop.android.push

import org.unifiedpush.android.connector.FailedReason
import org.unifiedpush.android.connector.PushService
import org.unifiedpush.android.connector.data.PushEndpoint
import org.unifiedpush.android.connector.data.PushMessage

/**
 * UnifiedPush's callbacks. An endpoint is handed to the server with its Web
 * Push keys; a message is drawn only if the connector DECRYPTED it -- an
 * endpoint is a URL anyone who learns it can POST to, and only the server
 * holds the key that makes a message decrypt.
 */
class UpService : PushService() {
    override fun onNewEndpoint(
        endpoint: PushEndpoint,
        instance: String,
    ) {
        val keys = endpoint.pubKeySet ?: return
        Push.register(this, PushJson.unifiedPush(endpoint.url, keys.pubKey, keys.auth))
    }

    override fun onMessage(
        message: PushMessage,
        instance: String,
    ) {
        if (!message.decrypted) return
        PushJson.alert(message.content.toString(Charsets.UTF_8))?.let { AlertNotifier.show(this, it) }
    }

    override fun onRegistrationFailed(
        reason: FailedReason,
        instance: String,
    ) = Unit

    override fun onUnregistered(instance: String) = Unit
}
