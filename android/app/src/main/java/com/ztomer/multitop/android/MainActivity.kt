package com.ztomer.multitop.android

import android.Manifest
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.ztomer.multitop.android.api.ServerApi
import com.ztomer.multitop.android.pairing.Pairing
import com.ztomer.multitop.android.push.Push
import com.ztomer.multitop.android.push.PushController
import com.ztomer.multitop.android.push.PushPanel
import com.ztomer.multitop.android.scan.CameraScanner
import com.ztomer.multitop.android.ui.CameraAccess
import com.ztomer.multitop.android.ui.HostsController
import com.ztomer.multitop.android.ui.HostsPanel
import com.ztomer.multitop.android.ui.PairingActions
import com.ztomer.multitop.android.ui.PairingController
import com.ztomer.multitop.android.ui.PairingScreen
import com.ztomer.multitop.android.ui.ScanSlot
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors

/**
 * Wires the pairing screen to the real things: the store sealed by the
 * Keystore key, in `noBackupFilesDir`, the camera behind its permission, and
 * the paired server's read-only API on one background thread. The screens
 * themselves (PairingScreen, HostsPanel) are tested per state without any.
 */
class MainActivity : ComponentActivity() {
    private val network: ExecutorService = Executors.newSingleThreadExecutor()

    override fun onDestroy() {
        network.shutdownNow()
        super.onDestroy()
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        val store = Push.store(this)
        val controller = PairingController(store)
        val hasCamera = packageManager.hasSystemFeature(PackageManager.FEATURE_CAMERA_ANY)
        setContent {
            MaterialTheme(colorScheme = if (isSystemInDarkTheme()) darkColorScheme() else lightColorScheme()) {
                var access by remember { mutableStateOf(cameraAccess(hasCamera)) }
                val ask =
                    rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
                        access = if (granted) CameraAccess.Granted else CameraAccess.Denied
                    }
                val actions =
                    PairingActions(
                        begin = {
                            controller.begin()
                            if (access == CameraAccess.Denied) {
                                access = CameraAccess.Asking
                                ask.launch(Manifest.permission.CAMERA)
                            }
                        },
                        submit = controller::submit,
                        cancel = controller::cancel,
                        unpair = controller::unpair,
                    )
                PairingScreen(controller.state, actions, paired = { Hosts(it) }) {
                    ScanSlot(access) { CameraScanner(controller::submit, Modifier.fillMaxSize()) }
                }
            }
        }
    }

    /**
     * The paired server's hosts, read when the pairing is shown and on demand;
     * and A3's alert route, registered with the server each time it is shown.
     */
    @Composable
    private fun Hosts(pairing: Pairing) {
        val hosts = remember(pairing) { HostsController(ServerApi(pairing)::overview, network::execute) }
        val push =
            remember(pairing) {
                PushController({ Push.choice(this@MainActivity) }, ServerApi(pairing)::pushStatus, { Push.test(pairing) }, network::execute)
            }
        val notifications = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { }
        LaunchedEffect(hosts) {
            hosts.refresh()
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
                checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
            ) {
                notifications.launch(Manifest.permission.POST_NOTIFICATIONS)
            }
            Push.start(this@MainActivity)
            push.refresh()
        }
        Column(verticalArrangement = Arrangement.spacedBy(SECTION_GAP.dp)) {
            HostsPanel(hosts.state, hosts::refresh)
            PushPanel(push)
        }
    }

    private fun cameraAccess(hasCamera: Boolean): CameraAccess =
        when {
            !hasCamera -> CameraAccess.Absent
            checkSelfPermission(Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED -> CameraAccess.Granted
            else -> CameraAccess.Denied
        }

    private companion object {
        const val SECTION_GAP = 24
    }
}
