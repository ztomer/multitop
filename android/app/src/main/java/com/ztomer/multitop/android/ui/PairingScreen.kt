package com.ztomer.multitop.android.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import com.ztomer.multitop.android.pairing.Pairing

/** What each control does; the screen has no other behaviour. */
class PairingActions(
    val begin: () -> Unit,
    val submit: (String) -> Unit,
    val cancel: () -> Unit,
    val unpair: () -> Unit,
)

/**
 * The one screen, one layout per [PairingState]. Every state's headline leads
 * with its own glyph AND says the state in words (· not paired, → pairing,
 * ✓ paired, ✗ failed), so no state rests on colour; the error colour is used
 * only for the failure, the one state that is a fault.
 */
@Composable
fun PairingScreen(
    state: PairingState,
    actions: PairingActions,
    content: @Composable () -> Unit,
) {
    Scaffold { insets ->
        Column(
            modifier =
                Modifier
                    .fillMaxSize()
                    .padding(insets)
                    .padding(PAD.dp)
                    .verticalScroll(rememberScrollState()),
            verticalArrangement = Arrangement.spacedBy(GAP.dp),
        ) {
            when (state) {
                PairingState.NotPaired -> NotPaired(actions)
                PairingState.InProgress -> InProgress(actions, content)
                is PairingState.Paired -> Paired(state.pairing, actions)
                is PairingState.Failed -> Failed(state.reason, actions)
            }
        }
    }
}

@Composable
private fun Headline(text: String) = Text(text, style = MaterialTheme.typography.headlineSmall)

@Composable
private fun Body(text: String) = Text(text, style = MaterialTheme.typography.bodyLarge)

@Composable
private fun NotPaired(actions: PairingActions) {
    Headline(Copy.NOT_PAIRED)
    Body(Copy.NOT_PAIRED_DETAIL)
    Button(onClick = actions.begin) { Text(Copy.PAIR) }
}

@Composable
private fun InProgress(
    actions: PairingActions,
    content: @Composable () -> Unit,
) {
    var pasted by rememberSaveable { mutableStateOf("") }
    Headline(Copy.PAIRING)
    Body(Copy.PAIRING_DETAIL)
    Column(Modifier.fillMaxWidth().aspectRatio(1f)) { content() }
    OutlinedTextField(
        value = pasted,
        onValueChange = { pasted = it },
        label = { Text(Copy.PASTE_LABEL) },
        modifier = Modifier.fillMaxWidth(),
        minLines = 2,
        maxLines = 4,
    )
    Row(horizontalArrangement = Arrangement.spacedBy(GAP.dp)) {
        Button(onClick = { actions.submit(pasted) }) { Text(Copy.PAIR_PASTED) }
        OutlinedButton(onClick = actions.cancel) { Text(Copy.CANCEL) }
    }
}

@Composable
private fun Paired(
    pairing: Pairing,
    actions: PairingActions,
) {
    Headline(Copy.paired(pairing))
    Body(Copy.FINGERPRINT_DETAIL)
    Text(
        Copy.fingerprint(pairing.pin.hex),
        style = MaterialTheme.typography.bodyLarge,
        fontFamily = FontFamily.Monospace,
    )
    Body(Copy.PAIRED_DETAIL)
    OutlinedButton(onClick = actions.unpair) { Text(Copy.UNPAIR) }
}

@Composable
private fun Failed(
    reason: String,
    actions: PairingActions,
) {
    Text(Copy.failed(reason), style = MaterialTheme.typography.headlineSmall, color = MaterialTheme.colorScheme.error)
    Row(horizontalArrangement = Arrangement.spacedBy(GAP.dp)) {
        Button(onClick = actions.begin) { Text(Copy.TRY_AGAIN) }
        OutlinedButton(onClick = actions.cancel) { Text(Copy.BACK) }
    }
}

/** The screen's words, in one place, so the tests assert the same strings the person reads. */
object Copy {
    const val NOT_PAIRED = "· Not paired"
    const val NOT_PAIRED_DETAIL =
        "This phone has no multitop server to read, so nothing is shown. Run multitop --serve on a LAN " +
            "address: it prints a pairing string and its QR code."
    const val PAIR = "→ Pair with a server"
    const val PAIRING = "→ Pairing"
    const val PAIRING_DETAIL = "Point the camera at the QR code multitop --serve printed, or paste the pairing string."
    const val PASTE_LABEL = "Pairing string (multitop://pair?...)"
    const val PAIR_PASTED = "Pair"
    const val CANCEL = "Cancel"
    const val FINGERPRINT_DETAIL = "Certificate fingerprint -- compare it with the sha256= line multitop --serve printed:"
    const val PAIRED_DETAIL = "Host data is not read yet in this build: the live screens come next."
    const val UNPAIR = "Unpair"
    const val TRY_AGAIN = "Try again"
    const val BACK = "Back"
    const val CAMERA_ASKING = "Waiting for permission to use the camera."
    const val CAMERA_DENIED = "Camera not allowed, so nothing is scanned. Paste the pairing string below instead."
    const val CAMERA_ABSENT = "This phone has no camera to scan with. Paste the pairing string below instead."

    fun paired(pairing: Pairing): String {
        val host = if (':' in pairing.host) "[${pairing.host}]" else pairing.host
        return "✓ Paired to $host:${pairing.port}"
    }

    fun failed(reason: String): String = "✗ Pairing failed: $reason"

    /** `sha256=` hex in groups of eight: the same characters in the same order, easier to compare by eye. */
    fun fingerprint(hex: String): String = "sha256=" + hex.chunked(GROUP).joinToString(" ")

    private const val GROUP = 8
}

private const val PAD = 24
private const val GAP = 16
