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
    }

    buildTypes {
        debug {
            // Keep development builds from displacing the GTK build (and
            // its seeded test session) on a device. Dropped when a build is
            // meant to exercise session adoption.
            applicationIdSuffix = ".skeleton"
        }
    }

    buildFeatures {
        compose = true
    }
}

dependencies {
    // What the uniffi-generated bindings load the native library with.
    implementation("net.java.dev.jna:jna:5.17.0@aar")
    // The uniffi-generated async functions are suspend functions.
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.10.2")

    // The UI. Material 3 with dynamic color, per doc/kotlin-plan.md.
    val composeBom = platform("androidx.compose:compose-bom:2025.06.01")
    implementation(composeBom)
    implementation("androidx.activity:activity-compose:1.10.1")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.foundation:foundation")
}
