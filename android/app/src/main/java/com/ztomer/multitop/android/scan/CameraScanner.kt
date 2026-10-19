package com.ztomer.multitop.android.scan

import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.LocalLifecycleOwner
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean

/**
 * The live viewfinder: CameraX's preview, and an analysis stream whose every
 * frame's luminance goes to [QrDecoder]. The first code read is handed to
 * [onDecoded] once, on the main thread; the camera is released when this
 * leaves the composition.
 */
@Composable
fun CameraScanner(
    onDecoded: (String) -> Unit,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val owner = LocalLifecycleOwner.current
    val deliver by rememberUpdatedState(onDecoded)
    val view = androidx.compose.runtime.remember { PreviewView(context) }
    DisposableEffect(owner) {
        val worker = Executors.newSingleThreadExecutor()
        val main = ContextCompat.getMainExecutor(context)
        val done = AtomicBoolean(false)
        val future = ProcessCameraProvider.getInstance(context)
        future.addListener({
            val provider = future.get()
            val preview = Preview.Builder().build().also { it.surfaceProvider = view.surfaceProvider }
            val analysis =
                ImageAnalysis
                    .Builder()
                    .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
                    .build()
            analysis.setAnalyzer(worker) { image ->
                image.use {
                    val plane = it.planes[0]
                    val luma = QrDecoder.packed(plane.buffer, plane.rowStride, it.width, it.height)
                    val text = QrDecoder.decode(luma, it.width, it.height)
                    if (text != null && done.compareAndSet(false, true)) main.execute { deliver(text) }
                }
            }
            provider.unbindAll()
            provider.bindToLifecycle(owner, CameraSelector.DEFAULT_BACK_CAMERA, preview, analysis)
        }, main)
        onDispose {
            future.addListener({ future.get().unbindAll() }, main)
            worker.shutdown()
        }
    }
    AndroidView(factory = { view }, modifier = modifier)
}
