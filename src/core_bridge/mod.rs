//! What sits between the core's plain Rust and the application's `GObject`s.
//!
//! `doc/track3-convergence.md` calls this Phase 1 and says it is built where
//! it is first needed rather than up front, because a bridge written before
//! its consumers has nothing to be designed against. This is the first piece:
//! the leaves of Phase 2 were stateless and needed none of it, and
//! `session_list/` is the first module that does.
//!
//! Still to arrive here, with the modules that need them: an
//! `ObservableVectorModel` presenting a `VectorDiff` stream as a
//! `gio::ListModel`, the wrapper cache that keeps `GObject` identity across
//! diffs, and the `bridge_properties!` macro driving `notify_*` from one
//! `select_all` over the core's `eyeball` subscribers.

pub(crate) mod settings_store;
