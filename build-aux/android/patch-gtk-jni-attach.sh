#!/bin/sh
# Let GDK's Android backend reach the JVM from a thread GLib made.
#
#     sh build-aux/android/patch-gtk-jni-attach.sh [gdkandroidinit.c path]
#
# Run this after `pixiewood generate` and before `pixiewood build`, next to the
# other `patch-gtk-*.sh` scripts. It writes into `subprojects/gtk`, which a
# re-extracted wrap loses.
#
# # What is wrong
#
# `gdk_android_get_env` returns the calling thread's `JNIEnv`, or NULL if that
# thread was never attached to the JVM. Every one of its callers dereferences
# the result without checking -- 29 of them in `gdkandroidcontentfile.c` alone,
# starting with `(*env)->PushLocalFrame (env, 2)`. A NULL there is a segfault,
# not an error return.
#
# That would be harmless if only the main thread ever called it, and GIO
# guarantees the opposite. `GdkAndroidContentFile` implements the synchronous
# `GFile` vfuncs and none of the `_async` ones, so GIO supplies the async
# variants itself by running the synchronous vfunc in a `GTask` thread pool.
# `g_file_query_info_async` on a `content://` file therefore lands in
# `gdk_android_content_file_query_info` on a GLib worker thread, by design,
# and dies:
#
#     F libc: Fatal signal 11 (SIGSEGV), code 1 (SEGV_MAPERR), fault addr 0x0
#     F DEBUG: #00 libgtk-4.so (gdk_android_content_file_query_info+497)
#     F DEBUG: #01 libgio-2.0.so (g_file_query_info+376)
#     F DEBUG: #02 libgio-2.0.so (query_info_async_thread+94)
#     F DEBUG: #03 libgio-2.0.so (g_task_thread_pool_thread+66)
#
# Measured on an x86_64 emulator, API 35. In Commune this is every attachment:
# picking any file at all -- image, video, anything -- crashes the application
# the moment the picker returns, because the composer asks the returned file
# what it is. It is not specific to this application; it is one
# `g_file_query_info_async` on a picked file.
#
# # The fix
#
# Attach the thread instead of returning NULL. The backend already knows how --
# `gdk_android_get_thread_env` does exactly this and hands back a guard whose
# `needs_detach` flag says whether to detach afterwards -- but it is a different
# function with a different signature, and the 29 call sites use the other one.
# Teaching `gdk_android_get_env` to attach fixes all of them at once, and every
# other unchecked caller in the backend besides.
#
# The detach cannot be dropped: ART aborts a thread that exits while still
# attached ("native thread exited without detaching"), and GLib's thread pools
# do retire idle threads. So the attachment is tied to the thread's lifetime
# with a `pthread_key` destructor, which is the ordinary JNI idiom for exactly
# this. `pthread_setspecific` is given the env pointer only so that the
# destructor is non-NULL and therefore runs.
#
# An attached thread gets the system classloader rather than the application's,
# so `FindClass` on an application class would fail there. Nothing on these
# paths does: the backend resolves its classes once during
# `gdk_android_initialize` and keeps global refs, so a worker thread only ever
# calls cached method IDs.
#
# # When to delete this
#
# When GTK stops handing NULL to callers that cannot take it -- either by
# attaching here as this does, or by moving the `GFile` vfuncs onto
# `gdk_android_get_thread_env`. The script fails rather than silently doing
# nothing if the code it expects is gone, so a GTK update that reworks this is
# noticed rather than papered over.
set -eu

SOURCE=${1:-subprojects/gtk/gdk/android/gdkandroidinit.c}

if [ ! -f "$SOURCE" ]; then
    printf 'no gdkandroidinit.c at %s -- run `pixiewood prepare` first\n' "$SOURCE" >&2
    exit 1
fi

MARKER='gdk_android_detach_on_thread_exit'
ANCHOR='  gint rc = (*gdk_android_vm)->GetEnv (gdk_android_vm, (void **) &gdk_android_thread_env, JNI_VERSION_1_6);
  if (G_UNLIKELY (rc != JNI_OK))
    g_critical ("Unable to get env for the current thread. Is is attached?");'

if grep -qF "$MARKER" "$SOURCE"; then
    printf 'already patched: %s\n' "$SOURCE"
    exit 0
fi

if ! grep -qF 'static __thread JNIEnv *gdk_android_thread_env = NULL;' "$SOURCE"; then
    printf 'no `static __thread JNIEnv *gdk_android_thread_env` in %s.\n' "$SOURCE" >&2
    printf 'GTK has probably changed this code -- check whether the patch is still needed.\n' >&2
    exit 1
fi

if ! grep -qF 'Unable to get env for the current thread' "$SOURCE"; then
    printf 'no `gdk_android_get_env` body to replace in %s.\n' "$SOURCE" >&2
    printf 'GTK has probably changed this code -- check whether the patch is still needed.\n' >&2
    exit 1
fi

# The thread-local declaration is the anchor for the helpers, and the failing
# `GetEnv` call is the anchor for the attach.
perl -0pi -e 's/(static __thread JNIEnv \*gdk_android_thread_env = NULL;\n)/$1
\/* Tie the attachment below to the thread that made it: ART aborts a thread
 * that exits while still attached. The value stored is only ever the env
 * pointer, because a NULL value would stop the destructor running at all.
 * See build-aux\/android\/patch-gtk-jni-attach.sh.
 *\/
static pthread_key_t gdk_android_detach_key;
static pthread_once_t gdk_android_detach_once = PTHREAD_ONCE_INIT;

static void
gdk_android_detach_on_thread_exit (void *unused)
{
  (void) unused;
  gdk_android_thread_env = NULL;
  if (gdk_android_vm)
    (*gdk_android_vm)->DetachCurrentThread (gdk_android_vm);
}

static void
gdk_android_detach_key_init (void)
{
  pthread_key_create (&gdk_android_detach_key, gdk_android_detach_on_thread_exit);
}
/' "$SOURCE"

perl -0pi -e 's/  gint rc = \(\*gdk_android_vm\)->GetEnv \(gdk_android_vm, \(void \*\*\) &gdk_android_thread_env, JNI_VERSION_1_6\);\n  if \(G_UNLIKELY \(rc != JNI_OK\)\)\n    g_critical \("Unable to get env for the current thread\. Is is attached\?"\);\n/  gint rc = (*gdk_android_vm)->GetEnv (gdk_android_vm, (void **) &gdk_android_thread_env, JNI_VERSION_1_6);
  if (rc == JNI_EDETACHED)
    {
      \/* GIO runs the synchronous GFile vfuncs in a GTask thread pool, so this
       * is reached on threads GLib made and never attached. Returning NULL
       * here is a segfault at the caller, which dereferences it unchecked.
       *\/
      JavaVMAttachArgs args = {
        .version = JNI_VERSION_1_6,
        .name = "GDK Worker Thread",
        .group = NULL
      };
      rc = (*gdk_android_vm)->AttachCurrentThread (gdk_android_vm, &gdk_android_thread_env, &args);
      if (rc == JNI_OK)
        {
          pthread_once (&gdk_android_detach_once, gdk_android_detach_key_init);
          pthread_setspecific (gdk_android_detach_key, gdk_android_thread_env);
        }
    }
  if (G_UNLIKELY (rc != JNI_OK))
    {
      g_critical ("Unable to get env for the current thread, and unable to attach it.");
      gdk_android_thread_env = NULL;
    }
/' "$SOURCE"

if ! grep -qF "$MARKER" "$SOURCE"; then
    printf 'patch did not apply to %s\n' "$SOURCE" >&2
    exit 1
fi

if ! grep -qF 'AttachCurrentThread (gdk_android_vm, &gdk_android_thread_env, &args)' "$SOURCE"; then
    printf 'helpers were added to %s but the attach was not\n' "$SOURCE" >&2
    exit 1
fi

printf 'patched %s: a GLib worker thread can reach the JVM instead of crashing\n' "$SOURCE"
