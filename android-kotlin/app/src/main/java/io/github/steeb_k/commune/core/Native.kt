// The one JNI class of the core: seeding the Java VM and application
// Context into Rust, for the pieces (the Keystore-backed secret store) that
// have to work on the other side of JNI.
package io.github.steeb_k.commune.core

import android.content.Context

object Native {
    init {
        // Also triggers JNI_OnLoad, which captures the VM.
        System.loadLibrary("commune_core")
    }

    external fun seed(context: Context)
}
