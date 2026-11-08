package com.ztomer.multitop.android.push

import com.google.firebase.messaging.FirebaseMessagingService
import com.google.firebase.messaging.RemoteMessage

/**
 * FCM's callbacks: the fallback route, carrying the same payload under `alert`.
 *
 * Firebase Messaging 25.1+ registers by Firebase installation ID (FID): with
 * `firebase_messaging_installation_id_enabled` in the manifest, every
 * registration and refresh arrives in [onRegistered] and the deprecated
 * `onNewToken` is never called, so it is not overridden. Android lint's
 * MissingFirebaseInstanceTokenRefresh predates this and is disabled, with its
 * reason, in app/build.gradle.kts (docs/DECISIONS.md, D1).
 */
class FcmService : FirebaseMessagingService() {
    /** The FID, on registration and whenever it changes; the server sends to it. */
    override fun onRegistered(installationId: String) {
        Push.register(this, PushJson.fcm(installationId))
    }

    override fun onMessageReceived(message: RemoteMessage) {
        message.data["alert"]?.let(PushJson::alert)?.let { AlertNotifier.show(this, it) }
    }
}
