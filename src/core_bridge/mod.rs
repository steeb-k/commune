//! What sits between the core's plain Rust and the application's `GObject`s.
//!
//! `doc/track3-convergence.md` calls this Phase 1 and says it is built where
//! it is first needed rather than up front, because a bridge written before
//! its consumers has nothing to be designed against. The first piece was
//! `settings_store`: the leaves of Phase 2 were stateless and needed none of
//! it, and `session_list/` was the first module that did.
//!
//! The other two pieces arrived with Phase 4's first module, the session
//! and its room list: [`observe::ObjectWatcher`], one tokio task per
//! `GObject` over every stream it follows, applying each value on the main
//! thread; and [`list_model::apply_diff`], a `VectorDiff` applied to the
//! `IndexMap` that is both a list model's order and its wrapper cache. The
//! plan called the first a `bridge_properties!` macro; a builder says the
//! same thing without hiding it.

pub(crate) mod list_model;
pub(crate) mod observe;
pub(crate) mod settings_store;

pub(crate) use self::observe::ObjectWatcher;
