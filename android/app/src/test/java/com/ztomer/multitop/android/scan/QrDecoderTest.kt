package com.ztomer.multitop.android.scan

import com.google.zxing.BinaryBitmap
import com.google.zxing.MultiFormatReader
import com.google.zxing.NotFoundException
import com.google.zxing.PlanarYUVLuminanceSource
import com.google.zxing.common.HybridBinarizer
import com.ztomer.multitop.android.pairing.Fixtures
import org.junit.jupiter.api.Assertions.assertArrayEquals
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertNull
import org.junit.jupiter.api.Test
import org.junit.jupiter.api.assertThrows
import java.nio.ByteBuffer

/**
 * The scanner against the QR code `multitop --serve` really draws
 * (`rust_server_pairing_qr.txt`, written and checked by the Rust test
 * `the_golden_pairing_qr_is_what_the_server_draws`), painted as a terminal
 * shows it: each character cell is one module wide and two tall, and its glyph
 * (`█ ▀ ▄`) is drawn in the terminal's foreground colour on its background.
 */
class QrDecoderTest {
    private val printed = Fixtures.text("rust_server_pairing.txt").trim()
    private val drawn = Fixtures.text("rust_server_pairing_qr.txt").trimEnd('\n').lines()

    private class Frame(
        val luma: ByteArray,
        val width: Int,
        val height: Int,
    )

    /** The terminal's text as a camera sees it, [SCALE] pixels per module. */
    private fun paint(
        foreground: Int,
        background: Int,
    ): Frame {
        val cols = drawn.maxOf { it.length }
        val width = cols * SCALE
        val height = drawn.size * 2 * SCALE
        val luma = ByteArray(width * height) { background.toByte() }
        drawn.forEachIndexed { row, line ->
            line.forEachIndexed { col, glyph ->
                val (upper, lower) =
                    when (glyph) {
                        '█' -> true to true
                        '▀' -> true to false
                        '▄' -> false to true
                        ' ' -> false to false
                        else -> error("unexpected glyph '$glyph' in the server's QR code")
                    }
                if (upper) fill(luma, width, col, row * 2, foreground)
                if (lower) fill(luma, width, col, row * 2 + 1, foreground)
            }
        }
        return Frame(luma, width, height)
    }

    private fun fill(
        luma: ByteArray,
        width: Int,
        moduleX: Int,
        moduleY: Int,
        value: Int,
    ) {
        for (y in moduleY * SCALE until (moduleY + 1) * SCALE) {
            for (x in moduleX * SCALE until (moduleX + 1) * SCALE) luma[y * width + x] = value.toByte()
        }
    }

    @Test
    fun `a light terminal's code -- dark modules on light -- decodes to the pairing string`() {
        val frame = paint(foreground = DARK, background = LIGHT)
        assertEquals(printed, QrDecoder.decode(frame.luma, frame.width, frame.height))
    }

    @Test
    fun `a dark terminal's code -- light modules on dark, inverted -- decodes to the pairing string`() {
        val frame = paint(foreground = LIGHT, background = DARK)
        assertEquals(printed, QrDecoder.decode(frame.luma, frame.width, frame.height))
    }

    @Test
    fun `the dark terminal's frame really is inverted -- a stock reader finds nothing in it`() {
        // Calibration: without this, the test above could pass on a fixture a
        // reader with no inversion support also reads.
        val frame = paint(foreground = LIGHT, background = DARK)
        val source = PlanarYUVLuminanceSource(frame.luma, frame.width, frame.height, 0, 0, frame.width, frame.height, false)
        assertThrows<NotFoundException> { MultiFormatReader().decode(BinaryBitmap(HybridBinarizer(source))) }
    }

    @Test
    fun `a frame with no code yields nothing`() {
        assertNull(QrDecoder.decode(ByteArray(BLANK * BLANK) { LIGHT.toByte() }, BLANK, BLANK))
    }

    @Test
    fun `a camera plane's padded rows are packed tight, its short last row included`() {
        // Two rows of width 3, stride 5: the padding (9) never reaches the frame.
        val plane = ByteBuffer.wrap(byteArrayOf(1, 2, 3, 9, 9, 4, 5, 6))
        assertArrayEquals(byteArrayOf(1, 2, 3, 4, 5, 6), QrDecoder.packed(plane, rowStride = 5, width = 3, height = 2))
        assertEquals(0, plane.position(), "the camera's buffer is left where it was")
    }

    private companion object {
        const val SCALE = 4
        const val DARK = 0x1C
        const val LIGHT = 0xF0
        const val BLANK = 64
    }
}
