/* Glue between the GStreamer binaries and the rest of the Android build.
 *
 * Compiled into `libcommune.so` beside `stub.c`; see `src/meson.build`. The
 * prefix these symbols reconcile against is built by
 * `build-aux/android/gstreamer-prefix.sh`.
 *
 * # proxy-libintl, twice, under two sets of names
 *
 * Neither GStreamer nor GTK uses GNU gettext on Android. Both use
 * proxy-libintl, a stub that forwards to a real libintl if the process happens
 * to have one and otherwise returns the untranslated string. The two builds
 * disagree about what to call it:
 *
 *   * The GStreamer binaries were built against proxy-libintl 0.4, which
 *     exports `libintl_gettext`, `libintl_bindtextdomain` and so on, and whose
 *     `libintl.h` `#define`s the plain names onto those.
 *   * pixiewood builds proxy-libintl 0.5, which renamed every one of them to
 *     `g_libintl_*`.
 *
 * So `libgstreamer-1.0.a` arrives with undefined references to `libintl_*` —
 * `gst_init` calls `bindtextdomain` in `init_pre` — and the libintl actually in
 * the process exports none of them.
 *
 * Linking the tarball's own `libintl.a` as well would resolve them, and would
 * put a second proxy-libintl in the process with its own idea of the bound
 * domains. Forwarding costs nine lines instead and leaves one.
 *
 * These have to match proxy-libintl's prototypes exactly rather than GNU
 * gettext's; `char *` returns, and `unsigned long` rather than `unsigned long
 * int` only because they are the same type.
 */

extern char *g_libintl_gettext (const char *msgid);
extern char *g_libintl_ngettext (const char *msgid1,
                                 const char *msgid2,
                                 unsigned long n);
extern char *g_libintl_dgettext (const char *domain, const char *msgid);
extern char *g_libintl_dngettext (const char *domain,
                                  const char *msgid1,
                                  const char *msgid2,
                                  unsigned long n);
extern char *g_libintl_dcgettext (const char *domain,
                                  const char *msgid,
                                  int category);
extern char *g_libintl_dcngettext (const char *domain,
                                   const char *msgid1,
                                   const char *msgid2,
                                   unsigned long n,
                                   int category);
extern char *g_libintl_textdomain (const char *domain);
extern char *g_libintl_bindtextdomain (const char *domain, const char *dirname);
extern char *g_libintl_bind_textdomain_codeset (const char *domain,
                                                const char *codeset);

char *
libintl_gettext (const char *msgid)
{
  return g_libintl_gettext (msgid);
}

char *
libintl_ngettext (const char *msgid1, const char *msgid2, unsigned long n)
{
  return g_libintl_ngettext (msgid1, msgid2, n);
}

char *
libintl_dgettext (const char *domain, const char *msgid)
{
  return g_libintl_dgettext (domain, msgid);
}

char *
libintl_dngettext (const char *domain,
                   const char *msgid1,
                   const char *msgid2,
                   unsigned long n)
{
  return g_libintl_dngettext (domain, msgid1, msgid2, n);
}

char *
libintl_dcgettext (const char *domain, const char *msgid, int category)
{
  return g_libintl_dcgettext (domain, msgid, category);
}

char *
libintl_dcngettext (const char *domain,
                    const char *msgid1,
                    const char *msgid2,
                    unsigned long n,
                    int category)
{
  return g_libintl_dcngettext (domain, msgid1, msgid2, n, category);
}

char *
libintl_textdomain (const char *domain)
{
  return g_libintl_textdomain (domain);
}

char *
libintl_bindtextdomain (const char *domain, const char *dirname)
{
  return g_libintl_bindtextdomain (domain, dirname);
}

char *
libintl_bind_textdomain_codeset (const char *domain, const char *codeset)
{
  return g_libintl_bind_textdomain_codeset (domain, codeset);
}
