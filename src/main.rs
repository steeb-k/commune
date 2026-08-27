//! The desktop entry point.
//!
//! Everything is in the library next to this file; see `src/lib.rs` for why it
//! is arranged that way. Android never runs this: its glue loads the library
//! itself and calls `main` in there.

// A Windows GUI application that asks for a console gets one, and it flashes up
// behind the window for as long as the app runs. Development builds keep it,
// because it is where `tracing` writes and where a panic is legible; release
// builds do without.
#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

fn main() {
    commune::run();
}
