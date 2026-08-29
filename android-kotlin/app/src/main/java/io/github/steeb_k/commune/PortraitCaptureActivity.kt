// The QR scanner, in the orientation the rest of the app lives in.
// zxing-android-embedded declares its own CaptureActivity as
// sensorLandscape, which turns the phone sideways mid-verification;
// this subclass exists only to be declared portrait in the manifest.
package io.github.steeb_k.commune

import com.journeyapps.barcodescanner.CaptureActivity

class PortraitCaptureActivity : CaptureActivity()
