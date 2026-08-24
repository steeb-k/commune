/* The `main` that GTK's Android glue looks for.
 *
 * `gdkandroidruntime.c` does not start a process. It `g_module_open`s the
 * application's shared object, `g_module_symbol`s for `main`, and calls it on a
 * thread of its own. Rust does not export a `main`, so this provides one and
 * hands straight over to `commune_main` in `src/lib.rs`.
 *
 * The indirection is not only about the symbol. Cargo builds the Rust as a
 * `staticlib`, which rustc never links, so Cargo needs no `libgtk-4.so` to
 * exist — only the pkg-config description of it. Meson does the one real link,
 * of this stub plus that archive plus the libraries it already knows how to
 * find. That is what keeps the whole thing inside pixiewood's single ninja
 * pass; see `doc/android.md`.
 *
 * `argc` and `argv` are ignored because there is nothing in them: the glue
 * calls `main` with `argv = { arg0 }` and no console exists to have passed
 * anything else.
 */

int commune_main (void);

int
main (int argc, char **argv, char **envp)
{
  (void) argc;
  (void) argv;
  (void) envp;

  return commune_main ();
}
