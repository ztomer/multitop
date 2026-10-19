package com.ztomer.multitop.android

import android.app.Activity
import android.os.Bundle
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.TextView

/**
 * The only screen in this slice, and it says so: the app is not paired and
 * cannot be yet, so it shows no data rather than a plausible-looking empty
 * dashboard (house rule: honest placeholders).
 */
class MainActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val pad = (PADDING_DP * resources.displayMetrics.density).toInt()
        val status =
            TextView(this).apply {
                setText(R.string.status_unpaired)
                setTextAppearance(android.R.style.TextAppearance_DeviceDefault_Large)
            }
        val detail =
            TextView(this).apply {
                setText(R.string.status_unpaired_detail)
                setTextAppearance(android.R.style.TextAppearance_DeviceDefault_Medium)
            }
        val column =
            LinearLayout(this).apply {
                orientation = LinearLayout.VERTICAL
                setPadding(pad, pad, pad, pad)
                addView(status)
                addView(detail)
            }
        // Edge-to-edge is enforced from targetSdk 35: the outer frame takes the
        // system-bar insets as padding (every API level from minSdk), and the
        // column keeps its own margin inside them.
        val root =
            FrameLayout(this).apply {
                fitsSystemWindows = true
                addView(column)
            }
        setContentView(root)
    }

    private companion object {
        const val PADDING_DP = 24
    }
}
