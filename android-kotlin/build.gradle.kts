// Versions match what the build machine's Gradle cache already holds.
// AGP 9 brings its own built-in Kotlin, so there is no separate Kotlin
// plugin to declare.
plugins {
    id("com.android.application") version "9.1.0" apply false
}
