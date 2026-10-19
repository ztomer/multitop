package com.ztomer.multitop.android.scan

import com.google.zxing.BarcodeFormat
import com.google.zxing.BinaryBitmap
import com.google.zxing.DecodeHintType
import com.google.zxing.MultiFormatReader
import com.google.zxing.PlanarYUVLuminanceSource
import com.google.zxing.ReaderException
import com.google.zxing.common.HybridBinarizer
import java.nio.ByteBuffer

/**
 * Finds a QR code in one camera frame's luminance and returns its text.
 *
 * Both polarities: `multitop --serve` draws its code with terminal glyphs, so
 * on a dark terminal the modules are LIGHT on a dark ground -- the inverse of a
 * printed code, which a stock QR reader does not find. ZXing retries an
 * inverted frame when asked ([DecodeHintType.ALSO_INVERTED], honoured by
 * [MultiFormatReader]); `QrDecoderTest` holds this to the code the server
 * really draws, painted the way a dark and a light terminal show it.
 */
object QrDecoder {
    private val HINTS: Map<DecodeHintType, Any> =
        mapOf(
            DecodeHintType.POSSIBLE_FORMATS to listOf(BarcodeFormat.QR_CODE),
            // ZXing tests this key for PRESENCE, not its value: `to false` still inverts.
            DecodeHintType.ALSO_INVERTED to true,
            DecodeHintType.TRY_HARDER to true,
        )

    /** [luma] is [width] x [height] bytes, one per pixel, rows packed. */
    fun decode(
        luma: ByteArray,
        width: Int,
        height: Int,
    ): String? {
        val source = PlanarYUVLuminanceSource(luma, width, height, 0, 0, width, height, false)
        val reader = MultiFormatReader().apply { setHints(HINTS) }
        return try {
            reader.decodeWithState(BinaryBitmap(HybridBinarizer(source))).text
        } catch (_: ReaderException) {
            null
        }
    }

    /**
     * A camera plane's rows packed tight: a Y plane's rows are [rowStride] bytes
     * apart, which may exceed [width], and its last row may stop at [width].
     */
    fun packed(
        plane: ByteBuffer,
        rowStride: Int,
        width: Int,
        height: Int,
    ): ByteArray {
        val out = ByteArray(width * height)
        val src = plane.duplicate()
        for (row in 0 until height) {
            src.position(row * rowStride)
            src.get(out, row * width, width)
        }
        return out
    }
}
