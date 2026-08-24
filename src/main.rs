//! The desktop entry point.
//!
//! Everything is in the library next to this file; see `src/lib.rs` for why it
//! is arranged that way. Android never runs this: its glue loads the library
//! itself and calls `main` in there.

fn main() {
    commune::run();
}
