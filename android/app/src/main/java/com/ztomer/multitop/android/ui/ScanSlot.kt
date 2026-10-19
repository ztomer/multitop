package com.ztomer.multitop.android.ui

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds

/** Whether the scanner can run, and if not, why -- each says so in words; none is a fault. */
enum class CameraAccess { Granted, Asking, Denied, Absent }

/** The scanner's square: the live camera when allowed, otherwise the reason it is not there. */
@Composable
fun ScanSlot(
    access: CameraAccess,
    content: @Composable () -> Unit,
) {
    when (access) {
        // The viewfinder takes a square, clipped: the preview scales to fill and
        // would otherwise draw over the paste field. A reason takes only its lines.
        CameraAccess.Granted -> Box(Modifier.fillMaxWidth().aspectRatio(1f).clipToBounds()) { content() }

        CameraAccess.Asking -> Text(Copy.CAMERA_ASKING, style = MaterialTheme.typography.bodyLarge)

        CameraAccess.Denied -> Text(Copy.CAMERA_DENIED, style = MaterialTheme.typography.bodyLarge)

        CameraAccess.Absent -> Text(Copy.CAMERA_ABSENT, style = MaterialTheme.typography.bodyLarge)
    }
}
