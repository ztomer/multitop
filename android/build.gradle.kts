// Versions are the newest RELEASES on 2026-10-18 (house rule: every layer
// current, none on an alpha/beta/RC). Each moves deliberately, in its own
// commit, when a newer release exists.
plugins {
    // AGP 9.4.1: the newest stable (9.5.0 is alpha). AGP 9 compiles Kotlin
    // itself ("built-in Kotlin"); applying org.jetbrains.kotlin.android in the
    // module is refused.
    id("com.android.application") version "9.4.1" apply false
    // AGP 9.4.1 resolves KGP 2.2.10 by default. Declaring the Kotlin plugin
    // here, unapplied, puts the newest stable KGP on the build classpath and
    // the built-in Kotlin compiles with it instead.
    id("org.jetbrains.kotlin.android") version "2.4.21" apply false
    // The Compose compiler ships with Kotlin since 2.0, versioned with it.
    id("org.jetbrains.kotlin.plugin.compose") version "2.4.21" apply false
    // ktlint 1.8.0 through the Gradle plugin: every violation is an error
    // (ktlint has no warning level), so the style gate cannot report and pass.
    id("org.jlleitschuh.gradle.ktlint") version "14.2.0" apply false
    // Kover 0.9.11 (JetBrains, released 2026-09-29): line coverage of the JVM
    // unit tests, and the floor (koverVerify) in app/build.gradle.kts.
    id("org.jetbrains.kotlinx.kover") version "0.9.11" apply false
}
