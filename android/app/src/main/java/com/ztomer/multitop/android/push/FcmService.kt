package com.ztomer.multitop.android.push

import com.google.firebase.messaging.FirebaseMessagingService
import com.google.firebase.messaging.RemoteMessage

/** FCM's callbacks: the fallback route, carrying the same payload under `alert`. */
class FcmService : FirebaseMessagingService() {
    /** Firebase 26's registration callback (it replaced the deprecated `onNewToken`). */
    override fun onRegistered(token: String) {
        Push.register(this, PushJson.fcm(token))
    }

    /**
     * Firebase 26 still calls this on its legacy token path (it invokes both
     * callbacks), and AGP's lint check MissingFirebaseInstanceTokenRefresh
     * still demands it; the override carries the deprecation rather than
     * hiding it, and does the same as [onRegistered].
     */
    @Deprecated("Firebase 26 replaced it with onRegistered", ReplaceWith("onRegistered(token)"))
    override fun onNewToken(token: String) {
        Push.register(this, PushJson.fcm(token))
    }

    override fun onMessageReceived(message: RemoteMessage) {
        message.data["alert"]?.let(PushJson::alert)?.let { AlertNotifier.show(this, it) }
    }
}
