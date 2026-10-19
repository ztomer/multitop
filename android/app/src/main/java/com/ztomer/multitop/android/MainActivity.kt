package com.ztomer.multitop.android

import android.Manifest
import android.content.pm.PackageManager
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import com.ztomer.multitop.android.scan.CameraScanner
import com.ztomer.multitop.android.store.AesGcmSealer
import com.ztomer.multitop.android.store.KeystoreKey
import com.ztomer.multitop.android.store.PairingStore
import com.ztomer.multitop.android.ui.CameraAccess
import com.ztomer.multitop.android.ui.PairingActions
import com.ztomer.multitop.android.ui.PairingController
import com.ztomer.multitop.android.ui.PairingScreen
import com.ztomer.multitop.android.ui.ScanSlot
import java.io.File

/**
 * Wires the pairing screen to the real things: the store sealed by the
 * Keystore key, in `noBackupFilesDir`, and the camera behind its permission.
 * The screen itself (PairingScreen) is tested per state without either.
 */
class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        val store = PairingStore(File(noBackupFilesDir, STORE_FILE), AesGcmSealer(KeystoreKey::get))
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
                PairingScreen(controller.state, actions) {
                    ScanSlot(access) { CameraScanner(controller::submit, Modifier.fillMaxSize()) }
                }
            }
        }
    }

    private fun cameraAccess(hasCamera: Boolean): CameraAccess =
        when {
            !hasCamera -> CameraAccess.Absent
            checkSelfPermission(Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED -> CameraAccess.Granted
            else -> CameraAccess.Denied
        }

    private companion object {
        const val STORE_FILE = "pairing.sealed"
    }
}
