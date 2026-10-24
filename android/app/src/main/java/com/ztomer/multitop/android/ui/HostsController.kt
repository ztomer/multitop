package com.ztomer.multitop.android.ui

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import com.ztomer.multitop.android.api.Fetch
import com.ztomer.multitop.android.api.Overview

/** What the hosts panel shows. A failure keeps the last overview it had, said to be the last one. */
sealed interface HostsState {
    /** A read is in flight; [last] is what the previous read found, if any. */
    data class Reading(
        val last: Overview?,
    ) : HostsState

    data class Read(
        val overview: Overview,
    ) : HostsState

    data class Failed(
        val reason: String,
        val last: Overview?,
    ) : HostsState
}

/**
 * One read at a time of the paired server, off the main thread through [run].
 * Last wins: a refresh started while one is in flight supersedes it, and the
 * superseded answer is dropped when it arrives -- never shown over a newer one.
 */
class HostsController(
    private val fetch: () -> Fetch,
    private val run: (Runnable) -> Unit,
) {
    var state: HostsState by mutableStateOf(HostsState.Reading(null))
        private set

    private var generation = 0

    fun refresh() {
        val mine = synchronized(this) { ++generation }
        state = HostsState.Reading(last())
        run(
            Runnable {
                val result = fetch()
                synchronized(this) {
                    if (mine != generation) return@Runnable
                    state =
                        when (result) {
                            is Fetch.Read -> HostsState.Read(result.overview)
                            is Fetch.Failed -> HostsState.Failed(result.reason, last())
                        }
                }
            },
        )
    }

    private fun last(): Overview? =
        when (val s = state) {
            is HostsState.Reading -> s.last
            is HostsState.Read -> s.overview
            is HostsState.Failed -> s.last
        }
}
