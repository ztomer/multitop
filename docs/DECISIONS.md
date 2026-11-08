# Decisions

Standing technical decisions this repo made itself, each with its evidence and
what retires it. Owner rulings are in `docs/roadmap.md` ("Rulings in force").

## D1 - Android lint's MissingFirebaseInstanceTokenRefresh is disabled (2026-11-08, M.FCMDEPR)

The check demands that every `FirebaseMessagingService` subclass override
`onNewToken`. Firebase Messaging 25.1.0 added registration by Firebase
installation ID (FID) and deprecated `getToken`, `deleteToken` and `onNewToken`
([release notes](https://firebase.google.com/support/release-notes/android),
[firebase-android-sdk#8087](https://github.com/firebase/firebase-android-sdk/pull/8087));
26.0.1 is the newest release and still marks `onNewToken` `@Deprecated`.

Ground truth, read from the 26.0.1 bytecode: `FirebaseMessaging` dispatches
`FCM_REGISTERED` (to `onRegistered`) when the manifest flag
`firebase_messaging_installation_id_enabled` is true, and `NEW_TOKEN` (to
`onNewToken`) only when it is not; `register()` fails with "API disabled"
without the flag. The app sets the flag (`PushAndroidTest.fcmRegistersByInstallationId`
holds it), so `onNewToken` is never called and overriding it would be dead,
deprecated code. The check is therefore a false positive here. Its detector
(`com.android.tools.lint.checks.FirebaseMessagingDetector`) is byte-identical in
lint 32.4.1 (AGP 9.4.1), 32.5.1 (AGP 9.5.1, newest stable) and 32.6.0-alpha03,
so no AGP release retires it.

Upstream: no issue found (GitHub issue search for
`MissingFirebaseInstanceTokenRefresh`, in firebase/firebase-android-sdk and
across GitHub, 2026-11-08: none in Firebase's tracker; other apps disable it
with a note, e.g. OrhanYigitDurmaz/h2open-mobile#4). The check belongs to
Android lint, whose tracker is Google's Issue Tracker (Android Studio, Lint).
Not filed from here.

Retired by: a lint release whose detector accepts `onRegistered`, or Firebase
removing `onNewToken`; then the `disable` line in `android/app/build.gradle.kts`
goes. `NoDeprecatedApiTest` keeps the deprecated override out either way.
