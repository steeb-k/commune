// Imported rather than written as `java.util.Properties`: in the Gradle
// Kotlin DSL `java` resolves to the Java plugin's accessor, not to the
// package, and the unqualified name is the only spelling that compiles.
import java.util.Properties

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.plugin.compose") version "2.2.10"
}

// A build setting that must not enter the repository.
//
// `local.properties` is the only file next to this one that git ignores —
// `gradle.properties` is tracked, so a credential put there is a credential
// committed, which is exactly how the KLIPY key was published once already
// (see doc/gif-search.md). Gradle does not load `local.properties` into
// project properties by itself, so it is read here; `~/.gradle/`'s
// `gradle.properties` and `-P` on the command line are the other two ways in,
// and both are outside the repository too.
fun secretProperty(name: String): String {
    val local = rootProject.file("local.properties")

    if (local.isFile) {
        val properties = Properties()
        local.inputStream().use { properties.load(it) }
        properties.getProperty(name)?.let { return it }
    }

    return project.findProperty(name) as String? ?: ""
}

// The version the whole project releases under, read from the one place
// `RELEASING.md` says to bump. Typing it here as well is how the APK came to
// claim 0.1.0 while the desktop build shipped 1.0.0-rc1, which the update
// feed would have turned into a phone that never sees a new release.
val communeVersionName: String by lazy {
    val manifest = rootProject.file("../Cargo.toml")
    val packageSection = manifest.readText().substringAfter("[package]", "")
    val version = Regex("""(?m)^version\s*=\s*"([^"]+)"""").find(packageSection)?.groupValues?.get(1)

    requireNotNull(version) { "No [package] version in ${manifest.absolutePath}" }
}

// The build number, and the only thing Android orders two installs by. It has
// to rise on every build that could be published, and nothing in the version
// name does: five nightlies all call themselves 1.0.0-rc1. The commit count
// is what `CFBundleVersion` already uses on macOS for the same reason
// (build-aux/macos/bundle.sh), so the three platforms agree by construction.
//
// Outside a git checkout there is no count to read, and 1 is the honest
// answer: such a build cannot be part of an ordered series anyway.
val communeVersionCode: Int by lazy {
    val counted =
        runCatching {
            providers
                .exec {
                    commandLine("git", "rev-list", "--count", "HEAD")
                    workingDir = rootProject.file("..")
                }.standardOutput
                .asText
                .get()
                .trim()
                .toInt()
        }.getOrNull()

    counted ?: 1
}

// The release signing identity, when there is one. `keystore.properties` and
// the keystore it names are git-ignored and live outside every build output;
// CI writes both from repository secrets before it builds. Without them the
// release variant falls back to the device debug key, which is what a
// contributor's checkout gets — it still builds and still installs, it just
// cannot produce an APK that upgrades a released one.
val releaseKeystore: Properties? by lazy {
    val file = rootProject.file("keystore.properties")

    if (!file.isFile) {
        null
    } else {
        Properties().also { properties -> file.inputStream().use { properties.load(it) } }
    }
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
        versionCode = communeVersionCode
        versionName = communeVersionName

        // The KLIPY API key for the GIF search, through `secretProperty` so
        // that it can only come from somewhere git does not track. This
        // mirrors the desktop build's `klipy-api-key` Meson option, empty
        // default included — with no key the GIF search is inert rather than
        // broken, which is what `gifSearchAvailable()` reports.
        buildConfigField(
            "String",
            "KLIPY_API_KEY",
            "\"${secretProperty("communeKlipyApiKey")}\"",
        )
    }

    signingConfigs {
        // The sideload identity for the device: the same debug keystore
        // that signed every install there, so `install -r` upgrades in
        // place and the adopted session survives. Still what debug builds
        // and a keyless checkout's release build use.
        create("device") {
            storeFile = file(System.getProperty("user.home") + "/.android/debug.keystore")
            storePassword = "android"
            keyAlias = "androiddebugkey"
            keyPassword = "android"
        }

        // The published identity. Android refuses to install an update
        // signed by a different key than the installed copy, which is the
        // whole basis of the in-app updater in doc/updates-plan.md: it is
        // the system, not us, that checks that an update came from here.
        // The first APK signed with this key cannot upgrade any of the
        // debug-keyed installs that came before it, and that one uninstall
        // is paid once, per device.
        releaseKeystore?.let { properties ->
            create("release") {
                storeFile = rootProject.file(properties.getProperty("storeFile"))
                storePassword = properties.getProperty("storePassword")
                keyAlias = properties.getProperty("keyAlias")
                keyPassword = properties.getProperty("keyPassword")
            }
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
            signingConfig = signingConfigs.findByName("release") ?: signingConfigs.getByName("device")
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
