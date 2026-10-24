package com.ztomer.multitop.android.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.unit.dp
import com.ztomer.multitop.android.api.HealthSummary
import com.ztomer.multitop.android.api.HostRow
import com.ztomer.multitop.android.api.Overview

/**
 * The read-only view of the paired server: its own count (`/api/health`), then
 * one line per host (`/api/hosts`). Every line leads with a glyph AND says its
 * state in words; the error colour is kept for the one fault, a failed read.
 */
@Composable
fun HostsPanel(
    state: HostsState,
    refresh: () -> Unit,
) {
    Column(verticalArrangement = Arrangement.spacedBy(ROW_GAP.dp)) {
        when (state) {
            is HostsState.Reading -> {
                Text(HostsCopy.READING, style = MaterialTheme.typography.titleMedium)
                state.last?.let { Last(it) }
            }

            is HostsState.Read -> {
                Body(state.overview)
            }

            is HostsState.Failed -> {
                Text(
                    HostsCopy.failed(state.reason),
                    style = MaterialTheme.typography.titleMedium,
                    color = MaterialTheme.colorScheme.error,
                )
                state.last?.let { Last(it) }
            }
        }
        OutlinedButton(onClick = refresh, enabled = state !is HostsState.Reading) { Text(HostsCopy.REFRESH) }
    }
}

@Composable
private fun Last(overview: Overview) {
    Text(HostsCopy.LAST_READ, style = MaterialTheme.typography.bodyMedium)
    Body(overview)
}

@Composable
private fun Body(overview: Overview) {
    Text(HostsCopy.summary(overview.health), style = MaterialTheme.typography.titleMedium)
    for (row in overview.hosts) {
        Text(HostsCopy.row(row), style = MaterialTheme.typography.bodyLarge)
    }
}

/** The panel's words, in one place, so the tests assert what the person reads. */
object HostsCopy {
    const val READING = "→ Reading the server"
    const val REFRESH = "Read again"
    const val LAST_READ = "What the last read found:"
    const val NO_HOSTS = "· The server watches no hosts"

    fun failed(reason: String): String = "✗ Could not read the server: $reason"

    /**
     * The server's count, never re-derived. ⚠ only when a host is breaching (the
     * one fault); a host not read yet is a status (·), and ✓ only when every
     * host was read and none breaches -- an unread host is never counted well.
     */
    fun summary(h: HealthSummary): String {
        if (h.total == 0) return NO_HOSTS
        val glyph =
            when {
                h.breaching > 0 -> "⚠"
                h.unread > 0 -> "·"
                else -> "✓"
            }
        val parts = mutableListOf("${h.healthy} healthy")
        if (h.breaching > 0) parts += "${h.breaching} breaching"
        if (h.unread > 0) parts += "${h.unread} not read yet"
        return "$glyph ${parts.joinToString(", ")} of ${h.total} ${if (h.total == 1) "host" else "hosts"}"
    }

    fun row(r: HostRow): String =
        when {
            !r.reachable -> "✗ ${r.label}: unreachable"
            !r.hasReading || r.health == null -> "· ${r.label}: not read yet"
            r.breaching -> "⚠ ${r.label}: breaching, health ${r.health}/100"
            else -> "✓ ${r.label}: health ${r.health}/100"
        }
}

private const val ROW_GAP = 8
