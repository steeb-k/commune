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
# Three changes, for three unrelated reasons.
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
#
# An `intent-filter` for `io.github.steeb-k.commune:`, which pixiewood has no
# way to express at all. This is the redirect URI OAuth 2.0 and Matrix SSO
# login use on Android instead of the loopback address other platforms use —
# see `src/login/local_server.rs` and `doc/android.md`. Without it, the
# `Intent` the browser sends back after authentication has nothing registered
# to receive it and Android drops it.
#
# And `POST_NOTIFICATIONS`, which pixiewood declares nothing of. Since API 33
# this is a runtime permission, so the entry does not grant anything — it only
# earns the right to ask, which `utils::android_notifications` does once the
# window exists. Without the entry the request is refused outright and every
# notification is dropped silently.
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

    my $scheme = "io.github.steeb-k.commune";
    my ($existing) = $xpc->findnodes(
        qq(//activity/intent-filter/data[\@android:scheme="$scheme"])
    );
    if (!$existing) {
        my $filter = $doc->createElement("intent-filter");

        my $action = $doc->createElement("action");
        $action->setAttributeNS($android, "android:name", "android.intent.action.VIEW");
        $filter->appendChild($action);

        for my $category ("android.intent.category.DEFAULT", "android.intent.category.BROWSABLE") {
            my $node = $doc->createElement("category");
            $node->setAttributeNS($android, "android:name", $category);
            $filter->appendChild($node);
        }

        my $data = $doc->createElement("data");
        $data->setAttributeNS($android, "android:scheme", $scheme);
        $filter->appendChild($data);

        $activity->appendChild($filter);
    }

    # `uses-permission` is a child of <manifest>, not of <application>, and
    # Android ignores one that is put in the wrong place rather than refusing
    # the build.
    my $permission = "android.permission.POST_NOTIFICATIONS";
    my ($granted) = $xpc->findnodes(
        qq(/manifest/uses-permission[\@android:name="$permission"])
    );
    if (!$granted) {
        my $node = $doc->createElement("uses-permission");
        $node->setAttributeNS($android, "android:name", $permission);
        $doc->documentElement->insertBefore($node, $application);
    }

    $doc->toFile($path, 1);

    printf "patched %s: launchMode=singleTask, allowBackup=false, %s intent-filter, %s\n",
        $path, $scheme, $permission;
' "$MANIFEST"

# Say so if any of the four did not take, rather than letting a silent no-op
# through.
for expected in \
    'android:launchMode="singleTask"' \
    'android:allowBackup="false"' \
    'android:scheme="io.github.steeb-k.commune"' \
    'android:name="android.permission.POST_NOTIFICATIONS"'
do
    if ! grep -q "$expected" "$MANIFEST"; then
        printf 'expected %s in %s after patching\n' "$expected" "$MANIFEST" >&2
        exit 1
    fi
done
