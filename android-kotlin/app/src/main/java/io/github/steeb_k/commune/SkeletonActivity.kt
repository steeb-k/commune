// The walking skeleton: the smallest Android app that loads
// libcommune_core.so through the uniffi-generated Kotlin bindings and shows
// that the call round-trips. The real UI (Jetpack Compose, chunk 8 of
// doc/kotlin-plan.md) replaces this activity; nothing here is meant to
// survive.
package io.github.steeb_k.commune

import android.app.Activity
import android.os.Bundle
import android.view.Gravity
import android.widget.TextView
import io.github.steeb_k.commune.core.coreVersion

class SkeletonActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        val text = TextView(this)
        text.textSize = 24f
        text.gravity = Gravity.CENTER
        text.text = "commune-core ${coreVersion()}\nRust core loaded over UniFFI"
        setContentView(text)
    }
}
