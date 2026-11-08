package com.ztomer.multitop.android.push

/**
 * How this phone asks for alerts (docs/roadmap.md A3, owner ruling OW.26:
 * "both, ntfy first"). Decided from what the phone HAS, every time the app
 * opens, so installing ntfy later moves the next registration to it.
 */
sealed interface PushChoice {
    /** A UnifiedPush distributor (ntfy) is installed: register with it. */
    data class UnifiedPush(
        val distributor: String,
    ) : PushChoice

    /** No distributor, but this build carries a Firebase configuration. */
    data object Fcm : PushChoice

    /** Neither: alerts cannot reach this phone, and the panel says how to fix it. */
    data object None : PushChoice
}

object PushPlan {
    /**
     * The distributor already chosen while it is still installed; otherwise the
     * first installed one that is not this app itself; FCM only after both;
     * [PushChoice.None] last.
     */
    fun choose(
        distributors: List<String>,
        saved: String?,
        ownPackage: String,
        fcmConfigured: Boolean,
    ): PushChoice {
        val others = distributors.filter { it != ownPackage }
        val pick = saved?.takeIf { it in others } ?: others.firstOrNull()
        return when {
            pick != null -> PushChoice.UnifiedPush(pick)
            fcmConfigured -> PushChoice.Fcm
            else -> PushChoice.None
        }
    }
}
