plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.plugin.compose") version "2.2.10"
}

android {
    namespace = "io.github.steeb_k.commune"
    compileSdk = 36

    defaultConfig {
        // The real application id — see doc/kotlin-plan.md, "App identity":
        // release builds must be able to adopt the GTK build's session in
        // place.
        applicationId = "io.github.steeb_k.commune"
        minSdk = 29
        targetSdk = 36
        versionCode = 1
        versionName = "0.1.0"

        // The KLIPY API key for the GIF search. It is a credential, so it is
        // never in this repository: set `communeKlipyApiKey` in your own
        // `local.properties` or `~/.gradle/gradle.properties`, or pass
        // `-PcommuneKlipyApiKey=…` on the command line. This mirrors the
        // desktop build's `klipy-api-key` Meson option, empty default
        // included — with no key the GIF search is inert rather than broken,
        // which is what `gifSearchAvailable()` reports.
        buildConfigField(
            "String",
            "KLIPY_API_KEY",
            "\"${project.findProperty("communeKlipyApiKey") as String? ?: ""}\"",
        )
    }

    signingConfigs {
        // The sideload identity for the device: the same debug keystore
        // that signed every install there, so `install -r` upgrades in
        // place and the adopted session survives. Play Store signing
        // arrives with the Store work.
        create("device") {
            storeFile = file(System.getProperty("user.home") + "/.android/debug.keystore")
            storePassword = "android"
            keyAlias = "androiddebugkey"
            keyPassword = "android"
        }
    }

    buildTypes {
        debug {
            // Keep development builds from displacing the GTK build (and
            // its seeded test session) on a device. Dropped when a build is
            // meant to exercise session adoption.
            applicationIdSuffix = ".skeleton"
            // The emulator loop; keep the arm64 core out of its APK.
            ndk { abiFilters += "x86_64" }
        }
        release {
            signingConfig = signingConfigs.getByName("device")
            // The device APK carries only its own ABI. No minification:
            // the size is the Rust core's, and R8 has nothing to shrink
            // that is worth the JNA/UniFFI keep-rule risk.
            ndk { abiFilters += "arm64-v8a" }
        }
    }

    buildFeatures {
        compose = true
        // For KLIPY_API_KEY above.
        buildConfig = true
    }
}

dependencies {
    // What the uniffi-generated bindings load the native library with.
    implementation("net.java.dev.jna:jna:5.17.0@aar")
    // The uniffi-generated async functions are suspend functions.
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.10.2")
    implementation("androidx.compose.material:material-icons-extended")
    implementation("com.journeyapps:zxing-android-embedded:4.3.0")
    // UnifiedPush: ntfy-style instant notifications, stable connector.
    implementation("org.unifiedpush.android:connector:3.3.5")
    // WebRTC for 1:1 calls: the maintained build of Google's library,
    // on stable release numbers (see [[stable-dependencies-only]]).
    implementation("io.github.webrtc-sdk:android:125.6422.07")
    implementation("androidx.media3:media3-exoplayer:1.6.1")
    implementation("androidx.media3:media3-ui:1.6.1")

    // The UI. Material 3 with dynamic color, per doc/kotlin-plan.md.
    val composeBom = platform("androidx.compose:compose-bom:2025.06.01")
    implementation(composeBom)
    implementation("androidx.activity:activity-compose:1.10.1")
    implementation("androidx.compose.material3:material3")
    // The stable home of the wavy progress indicator; Compose still gates
    // it behind alphas, the View library shipped it in 1.13.
    implementation("com.google.android.material:material:1.14.0")
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.foundation:foundation")
}
