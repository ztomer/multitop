package com.ztomer.multitop.android.push

import android.app.Application
import android.app.NotificationManager
import android.content.pm.PackageManager
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.core.app.ApplicationProvider
import com.google.firebase.messaging.RemoteMessage
import com.ztomer.multitop.android.api.Sent
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.unifiedpush.android.connector.data.PublicKeySet
import org.unifiedpush.android.connector.data.PushEndpoint
import org.unifiedpush.android.connector.data.PushMessage

/**
 * The Android edges of A3: the notification, both services, and the panel.
 * Robolectric runs the target platform, where POST_NOTIFICATIONS exists; the
 * permission is named by its string, which lint does not hold to minSdk.
 */
@RunWith(RobolectricTestRunner::class)
class PushAndroidTest {
    @get:Rule
    val compose = createComposeRule()

    private val notify = "android.permission.POST_NOTIFICATIONS"
    private val app: Application = ApplicationProvider.getApplicationContext()
    private val wire = """{"v":1,"kind":"down","host":"10.0.0.5","title":"beelink is unreachable","body":"lost"}"""

    private fun posted() = shadowOf(app.getSystemService(NotificationManager::class.java)).allNotifications

    @Test
    fun anAlertIsDrawnOnlyWithThePermission() {
        val alert = PushJson.alert(wire)!!
        shadowOf(app).denyPermissions(notify)
        assertFalse(AlertNotifier.show(app, alert))
        assertTrue(posted().isEmpty())
        shadowOf(app).grantPermissions(notify)
        assertTrue(AlertNotifier.show(app, alert))
        val n = posted().single()
        assertEquals("beelink is unreachable", shadowOf(n).contentTitle)
        assertEquals(AlertNotifier.CHANNEL, n.channelId)
    }

    @Test
    fun unifiedPushDrawsOnlyWhatDecrypted() {
        shadowOf(app).grantPermissions(notify)
        val service = Robolectric.buildService(UpService::class.java).create().get()
        service.onMessage(PushMessage(wire.toByteArray(), false), Push.INSTANCE)
        assertTrue("a message anyone could have POSTed is dropped", posted().isEmpty())
        service.onMessage(PushMessage(wire.toByteArray(), true), Push.INSTANCE)
        assertEquals(1, posted().size)
        // No keys, nothing to register; and the other callbacks are quiet.
        service.onNewEndpoint(PushEndpoint("https://ntfy.example/upX?up=1", null, false), Push.INSTANCE)
        service.onNewEndpoint(PushEndpoint("https://ntfy.example/upX?up=1", PublicKeySet("BPub", "auth"), false), Push.INSTANCE)
        service.onUnregistered(Push.INSTANCE)
    }

    @Test
    fun fcmDrawsTheSamePayload() {
        shadowOf(app).grantPermissions(notify)
        val service = Robolectric.buildService(FcmService::class.java).create().get()
        service.onMessageReceived(RemoteMessage.Builder("x@fcm").addData("other", "1").build())
        assertTrue(posted().isEmpty())
        service.onMessageReceived(RemoteMessage.Builder("x@fcm").addData("alert", wire).build())
        assertEquals("beelink is unreachable", shadowOf(posted().single()).contentTitle)
        service.onRegistered("synthetic-fid")
    }

    @Test
    fun fcmRegistersByInstallationId() {
        // Without the flag, register() fails and Firebase falls back to the
        // deprecated onNewToken, which FcmService does not override -- and the
        // lint check that would demand it is disabled on the strength of this.
        val meta = app.packageManager.getApplicationInfo(app.packageName, PackageManager.GET_META_DATA).metaData
        assertTrue(meta.getBoolean("firebase_messaging_installation_id_enabled"))
    }

    @Test
    fun withNoDistributorAndNoFirebaseThePhoneHasNoRoute() {
        assertFalse(Push.fcmConfigured(app))
        assertEquals(PushChoice.None, Push.choice(app))
        Push.start(app)
        Push.register(app, PushJson.fcm("unpaired"))
    }

    @Test
    fun thePanelShowsTheRouteAndSendsATest() {
        var tests = 0
        val c =
            PushController(
                choice = { PushChoice.None },
                status = { Sent.Answered(200, """{"route":null}""") },
                test = {
                    tests++
                    Sent.Answered(409, "{}")
                },
                run = Runnable::run,
            )
        c.refresh()
        compose.setContent { PushPanel(c) }
        compose.onNodeWithText(PushCopy.NO_DISTRIBUTOR).assertIsDisplayed()
        compose.onNodeWithText(PushCopy.TEST).performClick()
        compose.onNodeWithText(PushCopy.NOTHING_REGISTERED).assertIsDisplayed()
        assertEquals(1, tests)
    }
}
