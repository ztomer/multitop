plugins {
    id("com.android.application")
    id("org.jlleitschuh.gradle.ktlint")
    id("org.jetbrains.kotlinx.kover")
}

android {
    namespace = "com.ztomer.multitop.android"
    // The newest STABLE platform on 2026-10-18 is android-37.2 (37.2-beta3 and
    // the canaries are not releases). A minor API level is named this way in
    // AGP 9; `compileSdk = 37` would compile against 37.0.
    compileSdk {
        version =
            release(37) {
                minorApiLevel = 2
            }
    }

    defaultConfig {
        applicationId = "com.ztomer.multitop.android"
        // 28, and why (docs/roadmap.md, A2): the floor at which Android refuses
        // cleartext by default -- the platform protection this client relies on
        // rather than opts out of -- and the floor the owner's other side-loaded
        // app (koffee_big) already ships to, so the phone it is for meets it.
        minSdk = 28
        targetSdk = 37
        versionCode = 1
        versionName = "0.1.0"
        // 64-bit only (house platform policy). The app is pure Kotlin today and
        // packages no native code; the filter is what keeps a future native
        // dependency from smuggling 32-bit ABIs into the APK.
        ndk {
            abiFilters += listOf("arm64-v8a", "x86_64")
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    // Warnings are errors in every tool the gate runs: the compiler, Android
    // lint and ktlint. A gate that reports and passes is not a gate.
    lint {
        warningsAsErrors = true
        abortOnError = true
        checkAllWarnings = true
        checkTestSources = true
        // No baseline and no disabled ids: a finding is fixed, never suppressed.
    }

    testOptions {
        unitTests.all { it.useJUnitPlatform() }
    }
}

kotlin {
    compilerOptions {
        allWarningsAsErrors.set(true)
    }
}

ktlint {
    version.set("1.8.0")
    ignoreFailures.set(false)
}

// The coverage floor: line coverage of the debug variant's main code under the
// JVM unit tests, as a whole-number percentage. The number lives in
// tools/android_coverage_floor.txt (one place, read by the build and by the
// roadmap's record of it); `./gradlew koverVerify` fails below it.
kover {
    reports {
        verify {
            rule {
                minBound(
                    providers
                        .fileContents(rootProject.layout.projectDirectory.file("../tools/android_coverage_floor.txt"))
                        .asText
                        .map { it.trim().toInt() }
                        .get(),
                )
            }
        }
    }
}

dependencies {
    testImplementation("org.junit.jupiter:junit-jupiter:6.1.3")
    testRuntimeOnly("org.junit.platform:junit-platform-launcher:6.1.3")
}
