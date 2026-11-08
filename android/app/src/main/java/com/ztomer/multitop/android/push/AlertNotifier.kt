package com.ztomer.multitop.android.push

import android.Manifest
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import com.ztomer.multitop.android.MainActivity
import com.ztomer.multitop.android.R

/**
 * Draws a pushed [Alert], the same on either route. One channel, high
 * importance: an alert is a host down, which is what a person wants to hear.
 * Each host keeps one notification (a recovery replaces its "down").
 */
object AlertNotifier {
    const val CHANNEL = "alerts"
    private const val CHANNEL_NAME = "Host alerts"

    fun ensureChannel(context: Context) {
        val manager = context.getSystemService(NotificationManager::class.java) ?: return
        manager.createNotificationChannel(
            NotificationChannel(CHANNEL, CHANNEL_NAME, NotificationManager.IMPORTANCE_HIGH),
        )
    }

    /** True when the alert was handed to the system; false when notifications are not permitted. */
    fun show(
        context: Context,
        alert: Alert,
    ): Boolean {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
            context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
        ) {
            return false
        }
        ensureChannel(context)
        val open =
            PendingIntent.getActivity(
                context,
                0,
                Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                PendingIntent.FLAG_IMMUTABLE,
            )
        val notification =
            android.app.Notification
                .Builder(context, CHANNEL)
                .setSmallIcon(R.drawable.ic_notify)
                .setContentTitle(alert.title)
                .setContentText(alert.body)
                .setStyle(
                    android.app.Notification
                        .BigTextStyle()
                        .bigText(alert.body),
                ).setContentIntent(open)
                .setAutoCancel(true)
                .build()
        val manager = context.getSystemService(NotificationManager::class.java) ?: return false
        manager.notify(alert.host.ifEmpty { alert.kind }.hashCode(), notification)
        return true
    }
}
