package com.ztomer.multitop.android.push

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import com.ztomer.multitop.android.api.Sent

/**
 * The push panel's two lines: the route the server will use, and what the last
 * test alert did. Each request runs off the main thread through [run]; the
 * test button is not offered while one is in flight (a second tap would be a
 * second alert, not a faster one).
 */
class PushController(
    private val choice: () -> PushChoice,
    private val status: () -> Sent,
    private val test: () -> Sent,
    private val run: (Runnable) -> Unit,
) {
    var route: String by mutableStateOf(PushCopy.READING)
        private set
    var result: String? by mutableStateOf(null)
        private set
    var testing: Boolean by mutableStateOf(false)
        private set

    fun refresh() {
        run(Runnable { route = PushCopy.status(status(), choice()) })
    }

    fun sendTest() {
        if (testing) return
        testing = true
        run(
            Runnable {
                result = PushCopy.tested(test())
                testing = false
                route = PushCopy.status(status(), choice())
            },
        )
    }
}
