plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.plugin.compose")
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

    buildFeatures {
        compose = true
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
        // Robolectric (the Compose UI tests) needs the merged resources and the
        // manifest on the JVM unit-test classpath.
        unitTests.isIncludeAndroidResources = true
        unitTests.all {
            it.useJUnitPlatform()
            // Robolectric on JDK 24+: it sets a FileDescriptor's raw fd through
            // jdk.internal.access, and loads its native runtime with System.load.
            it.jvmArgs("--add-exports=java.base/jdk.internal.access=ALL-UNNAMED", "--enable-native-access=ALL-UNNAMED")
        }
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

// The newest stable release of each on 2026-10-19 (Google's maven-metadata
// <release> names alphas; these are the newest without alpha/beta/rc).
dependencies {
    implementation(platform("androidx.compose:compose-bom:2026.09.00"))
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.ui:ui")
    implementation("androidx.activity:activity-compose:1.13.0")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.11.0")
    // CameraX for the scanner's frames; ZXing (core 3.5.4, released 2025-11)
    // decodes them. Pure Java, so the decoder is tested on the JVM against the
    // QR code the server really draws, and it reads both polarities
    // (DecodeHintType.ALSO_INVERTED): the terminal code is light-on-dark.
    implementation("androidx.camera:camera-camera2:1.6.2")
    implementation("androidx.camera:camera-lifecycle:1.6.2")
    implementation("androidx.camera:camera-view:1.6.2")
    implementation("com.google.zxing:core:3.5.4")

    testImplementation("org.junit.jupiter:junit-jupiter:6.1.3")
    testRuntimeOnly("org.junit.platform:junit-platform-launcher:6.1.3")
    // The Compose UI tests run on the JVM under Robolectric, whose runner is
    // JUnit 4's; the vintage engine runs them on the same JUnit Platform.
    testImplementation(platform("androidx.compose:compose-bom:2026.09.00"))
    testImplementation("androidx.compose.ui:ui-test-junit4")
    testImplementation("org.robolectric:robolectric:4.17")
    testImplementation("junit:junit:4.13.2")
    // ui-test-junit4 pulls espresso-core 3.5.0, whose input injection calls
    // InputManager.getInstance(), gone from the platform Robolectric runs.
    testImplementation("androidx.test.espresso:espresso-core:3.7.0")
    testRuntimeOnly("org.junit.vintage:junit-vintage-engine:6.1.3")
    debugImplementation("androidx.compose.ui:ui-test-manifest")
}
