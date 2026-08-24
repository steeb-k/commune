#!/bin/sh
# Patch the `AndroidManifest.xml` that `pixiewood generate` writes.
#
#     sh build-aux/android/patch-manifest.sh [manifest path]
#
# Run this after `pixiewood generate` and before `pixiewood build`. pixiewood
# rewrites the manifest from `generate/manifest.xsl` every time it is run, so
# these cannot be hand-edited once and forgotten, and they are not settings the
# pixiewood manifest can express.
#
# Two changes, for two unrelated reasons.
#
# `launchMode`, which pixiewood hardcodes to `standard`
# (`generate/manifest.xsl:32`). GTK has a single toplevel, and Android stacks a
# new Activity on every launch, so returning to the app from the launcher shows
# an empty white window while the old, working Activity sits behind it —
# `numActivities=2` and `LAUNCH_MULTIPLE` in logcat. `singleTask` resumes the
# real UI instead. This is the Android form of the single-instance question the
# Windows port had to answer for `matrix:` links.
#
# `allowBackup`, which pixiewood leaves at Android's default of `true`. That
# default lets `adb backup` and the system's cloud backup copy the application's
# private files off the device. Commune's Android secret store is currently a
# plaintext file holding the passphrase that encrypts the local databases (see
# `src/secret/android.rs`), so leaving this on would hand that passphrase to
# anything that can run `adb backup`. It stays off even once the Keystore
# replaces that file: a Keystore key cannot leave the device, so a backup that
# carried the databases without it would only restore something unreadable.
set -eu

MANIFEST=${1:-.pixiewood/android/app/src/main/AndroidManifest.xml}

if [ ! -f "$MANIFEST" ]; then
    printf 'no manifest at %s -- run `pixiewood generate` first\n' "$MANIFEST" >&2
    exit 1
fi

# XML::LibXML rather than sed: pixiewood writes these as namespaced attributes
# on elements it may reorder, and a regex over attribute soup is the kind of
# thing that works until the day it silently does not.
perl -MXML::LibXML -e '
    my $path = $ARGV[0];
    my $android = "http://schemas.android.com/apk/res/android";

    my $doc = XML::LibXML->load_xml(location => $path);
    my $xpc = XML::LibXML::XPathContext->new($doc);
    $xpc->registerNs("android", $android);

    my ($application) = $xpc->findnodes("//application")
        or die "no <application> in $path\n";
    my ($activity) = $xpc->findnodes("//activity")
        or die "no <activity> in $path\n";

    $activity->setAttributeNS($android, "android:launchMode", "singleTask");
    $application->setAttributeNS($android, "android:allowBackup", "false");

    $doc->toFile($path, 1);

    printf "patched %s: launchMode=singleTask, allowBackup=false\n", $path;
' "$MANIFEST"

# Say so if either did not take, rather than letting a silent no-op through.
for expected in 'android:launchMode="singleTask"' 'android:allowBackup="false"'; do
    if ! grep -q "$expected" "$MANIFEST"; then
        printf 'expected %s in %s after patching\n' "$expected" "$MANIFEST" >&2
        exit 1
    fi
done
