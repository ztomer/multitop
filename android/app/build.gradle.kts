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
        versionCode = 3
        versionName = "0.2.1"
        // 64-bit only (house platform policy). The app is pure Kotlin today and
        // packages no native code; the filter is what keeps a future native
        // dependency from smuggling 32-bit ABIs into the APK.
        ndk {
            abiFilters += listOf("arm64-v8a", "x86_64")
        }
    }

    // A3's FCM fallback: Firebase's client reads its project from string
    // resources, which Google's google-services plugin writes from
    // google-services.json. This build writes the same five itself, and only
    // when the owner has put that file at android/app/google-services.json
    // (gitignored: it names the owner's Firebase project). Without it the
    // resources are absent, Firebase does not start, and the app registers
    // over UnifiedPush or says it cannot -- never a build failure.
    googleServices()?.let { g ->
        defaultConfig {
            resValue("string", "google_app_id", g.appId)
            resValue("string", "gcm_defaultSenderId", g.senderId)
            resValue("string", "google_api_key", g.apiKey)
            resValue("string", "project_id", g.projectId)
            resValue("string", "google_storage_bucket", g.bucket)
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
        }
    }

    buildFeatures {
        compose = true
        resValues = true
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
        // No baseline. One disabled id, a false positive with its reason
        // (docs/DECISIONS.md, D1): MissingFirebaseInstanceTokenRefresh demands
        // an override of onNewToken, which Firebase Messaging 25.1 deprecated
        // and never calls under FID registration (the manifest's
        // firebase_messaging_installation_id_enabled; PushAndroidTest holds it
        // on). The check is unchanged through lint 32.6.0-alpha03, so no AGP
        // release retires it; this line goes when one does.
        disable += "MissingFirebaseInstanceTokenRefresh"
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
            // A failure prints its message and cause on the console: CI keeps
            // nothing else, and a bare "AssertionFailedError at line 139" hid
            // why the end-to-end test's server exited there (2026-10-24).
            it.testLogging {
                events("failed")
                exceptionFormat = org.gradle.api.tasks.testing.logging.TestExceptionFormat.FULL
            }
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
    // A3 push (owner ruling OW.26, "both, ntfy first"). The UnifiedPush
    // connector 3.3.5 (newest, 2026) registers with the phone's distributor and
    // decrypts RFC 8291 Web Push itself; Firebase Messaging 26.0.2 (newest) is
    // the fallback route when the phone has no distributor.
    implementation("org.unifiedpush.android:connector:3.3.5")
    implementation("com.google.firebase:firebase-messaging:26.0.2")

    testImplementation("org.junit.jupiter:junit-jupiter:6.1.3")
    // org.json is the platform's on a phone; android.jar's copy is stubs on the
    // JVM, so the unit tests read JSON with the real library of the same API.
    testImplementation("org.json:json:20260814")
    testRuntimeOnly("org.junit.platform:junit-platform-launcher:6.1.3")
    // The Compose UI tests run on the JVM under Robolectric, whose runner is
    // JUnit 4's; the vintage engine runs them on the same JUnit Platform.
    testImplementation(platform("androidx.compose:compose-bom:2026.09.00"))
    testImplementation("androidx.compose.ui:ui-test-junit4")
    testImplementation("org.robolectric:robolectric:4.18")
    testImplementation("junit:junit:4.13.2")
    // ui-test-junit4 pulls espresso-core 3.5.0, whose input injection calls
    // InputManager.getInstance(), gone from the platform Robolectric runs.
    testImplementation("androidx.test.espresso:espresso-core:3.7.0")
    testRuntimeOnly("org.junit.vintage:junit-vintage-engine:6.1.3")
    debugImplementation("androidx.compose.ui:ui-test-manifest")
}

/** The five values Firebase reads, from android/app/google-services.json, or null without it. */
data class GoogleServices(
    val appId: String,
    val senderId: String,
    val apiKey: String,
    val projectId: String,
    val bucket: String,
)

fun googleServices(): GoogleServices? {
    val file = layout.projectDirectory.file("google-services.json").asFile
    if (!file.isFile) return null

    fun Any?.obj(key: String): Any? = (this as? Map<*, *>)?.get(key)
    val root = groovy.json.JsonSlurper().parse(file)
    val project = root.obj("project_info")
    val client =
        (root.obj("client") as? List<*>).orEmpty().firstOrNull {
            it.obj("client_info").obj("android_client_info").obj("package_name") == "com.ztomer.multitop.android"
        } ?: error("google-services.json has no client for com.ztomer.multitop.android")
    return GoogleServices(
        appId = client.obj("client_info").obj("mobilesdk_app_id").toString(),
        senderId = project.obj("project_number").toString(),
        apiKey = (client.obj("api_key") as? List<*>)?.firstOrNull().obj("current_key").toString(),
        projectId = project.obj("project_id").toString(),
        bucket = project.obj("storage_bucket")?.toString().orEmpty(),
    )
}
