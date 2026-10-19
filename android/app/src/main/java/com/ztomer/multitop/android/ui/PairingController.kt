package com.ztomer.multitop.android.ui

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import com.ztomer.multitop.android.pairing.Pairing
import com.ztomer.multitop.android.pairing.PairingPayload
import com.ztomer.multitop.android.store.PairingStore
import java.io.IOException
import java.security.GeneralSecurityException

/** What the one screen shows. Each state says what is true, never a plausible-looking guess. */
sealed interface PairingState {
    /** No pairing is stored. */
    data object NotPaired : PairingState

    /** The person is scanning or pasting a pairing string. */
    data object InProgress : PairingState

    data class Paired(
        val pairing: Pairing,
    ) : PairingState

    data class Failed(
        val reason: String,
    ) : PairingState
}

/**
 * Pairing as a small state machine over [PairingStore]. Every string -- scanned
 * or pasted -- goes through [PairingPayload.parse], so the camera and the paste
 * field are held to one set of rules, and a refusal shows its reason.
 */
class PairingController(
    private val store: PairingStore,
) {
    var state: PairingState by mutableStateOf(initial())
        private set

    private fun initial(): PairingState =
        when (val loaded = store.load()) {
            PairingStore.Loaded.Empty -> {
                PairingState.NotPaired
            }

            is PairingStore.Loaded.Found -> {
                PairingState.Paired(loaded.pairing)
            }

            is PairingStore.Loaded.Unreadable -> {
                // Useless without its key or its shape; keeping it would only
                // make the next start fail the same way.
                store.clear()
                PairingState.Failed("the saved pairing was unreadable and has been forgotten: ${loaded.reason}")
            }
        }

    /** Open the scanner and paste field. */
    fun begin() {
        state = PairingState.InProgress
    }

    /** Leave the scanner, or dismiss a failure, without pairing: back to whatever is stored. */
    fun cancel() {
        val loaded = store.load()
        state = if (loaded is PairingStore.Loaded.Found) PairingState.Paired(loaded.pairing) else PairingState.NotPaired
    }

    fun submit(text: String) {
        state =
            when (val parsed = PairingPayload.parse(text)) {
                is PairingPayload.Refused -> PairingState.Failed(parsed.reason)
                is PairingPayload.Parsed -> stored(parsed.pairing)
            }
    }

    private fun stored(pairing: Pairing): PairingState =
        try {
            store.save(pairing)
            PairingState.Paired(pairing)
        } catch (e: IOException) {
            PairingState.Failed("the pairing could not be saved: ${e.message}")
        } catch (e: GeneralSecurityException) {
            PairingState.Failed("the pairing could not be encrypted: ${e.javaClass.simpleName}")
        }

    /** Forget the server: the token, the pin and the address all go. */
    fun unpair() {
        state =
            if (store.clear()) {
                PairingState.NotPaired
            } else {
                PairingState.Failed("the saved pairing could not be deleted")
            }
    }
}
