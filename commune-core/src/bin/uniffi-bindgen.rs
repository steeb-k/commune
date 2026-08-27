//! The uniffi bindings generator, as a binary of this crate so it is always
//! at the exact uniffi revision the library uses.
//!
//! Generate the Kotlin bindings from the built library:
//!
//! ```sh
//! cargo run --features cli --bin uniffi-bindgen -- \
//!     generate --library target/debug/commune_core.dll \
//!     --language kotlin --out-dir target/bindings
//! ```

fn main() {
    uniffi::uniffi_bindgen_main();
}
