package com.ztomer.multitop.android.push

import android.content.Context
import com.google.firebase.FirebaseApp
import com.google.firebase.messaging.FirebaseMessaging
import com.ztomer.multitop.android.api.Sent
import com.ztomer.multitop.android.api.ServerApi
import com.ztomer.multitop.android.pairing.Pairing
import com.ztomer.multitop.android.store.AesGcmSealer
import com.ztomer.multitop.android.store.KeystoreKey
import com.ztomer.multitop.android.store.PairingStore
import org.unifiedpush.android.connector.UnifiedPush
import java.io.File
import kotlin.concurrent.thread

/**
 * The phone's half of A3: pick the route ([PushPlan]), register with it, and
 * hand what it returns (an endpoint and keys, or a token) to the paired server.
 * The services ([UpService], [FcmService]) call back into [register] whenever
 * the distributor or Firebase issues a new one.
 */
object Push {
    const val INSTANCE = "default"
    const val STORE_FILE = "pairing.sealed"

    /** True when this build carries a Firebase configuration (google-services.json at build time). */
    fun fcmConfigured(context: Context): Boolean = FirebaseApp.getApps(context).isNotEmpty()

    fun choice(context: Context): PushChoice =
        PushPlan.choose(
            UnifiedPush.getDistributors(context),
            UnifiedPush.getSavedDistributor(context),
            context.packageName,
            fcmConfigured(context),
        )

    fun store(context: Context): PairingStore = PairingStore(File(context.noBackupFilesDir, STORE_FILE), AesGcmSealer(KeystoreKey::get))

    private fun pairing(context: Context): Pairing? = (store(context).load() as? PairingStore.Loaded.Found)?.pairing

    /** Register this phone by its [choice]; the endpoint or token reaches the server asynchronously. */
    fun start(context: Context) {
        AlertNotifier.ensureChannel(context)
        when (val c = choice(context)) {
            is PushChoice.UnifiedPush -> {
                UnifiedPush.saveDistributor(context, c.distributor)
                UnifiedPush.register(context, INSTANCE, null, null)
            }

            PushChoice.Fcm -> {
                // Firebase 26: register() replaces the deprecated token getter;
                // the token arrives in FcmService.onRegistered.
                FirebaseMessaging.getInstance().register()
            }

            PushChoice.None -> {}
        }
    }

    /** Send a registration to the paired server, off the calling thread. Unpaired: nothing to tell. */
    fun register(
        context: Context,
        json: String,
    ) {
        val app = context.applicationContext
        thread(name = "push-register") { pairing(app)?.let { ServerApi(it).post("/api/push/register", json) } }
    }

    fun test(pairing: Pairing): Sent = ServerApi(pairing).post("/api/push/test", "{}")
}
