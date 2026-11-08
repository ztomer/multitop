package com.ztomer.multitop.android.push

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.unit.dp

/** The alert route and the test button, under the hosts. */
@Composable
fun PushPanel(controller: PushController) {
    Column(verticalArrangement = Arrangement.spacedBy(ROW_GAP.dp)) {
        Text(controller.route, style = MaterialTheme.typography.bodyLarge)
        controller.result?.let {
            Text(
                it,
                style = MaterialTheme.typography.bodyMedium,
                color = if (it.startsWith("✗")) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurface,
            )
        }
        OutlinedButton(onClick = controller::sendTest, enabled = !controller.testing) { Text(PushCopy.TEST) }
    }
}

private const val ROW_GAP = 8
