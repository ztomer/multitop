package com.ztomer.multitop.android.ui

import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable

/** Whether the scanner can run, and if not, why -- each says so in words; none is a fault. */
enum class CameraAccess { Granted, Asking, Denied, Absent }

/** The scanner's square: the live camera when allowed, otherwise the reason it is not there. */
@Composable
fun ScanSlot(
    access: CameraAccess,
    content: @Composable () -> Unit,
) {
    when (access) {
        CameraAccess.Granted -> content()
        CameraAccess.Asking -> Text(Copy.CAMERA_ASKING, style = MaterialTheme.typography.bodyLarge)
        CameraAccess.Denied -> Text(Copy.CAMERA_DENIED, style = MaterialTheme.typography.bodyLarge)
        CameraAccess.Absent -> Text(Copy.CAMERA_ABSENT, style = MaterialTheme.typography.bodyLarge)
    }
}
