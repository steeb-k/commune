//! The property half of the bridge: the core's observables into `notify_*()`.
//!
//! One tokio task per `GObject`, never one per property. Every stream the
//! object follows — an `eyeball::Subscriber`, a `VectorDiff` stream, a
//! `broadcast` receiver — is mapped to a stream of closures over the object
//! and the lot are merged; the task drains the merged stream and hands each
//! closure to the main context, where it runs against the live object, sets
//! its `Cell` and calls `notify_*()`. Values cross threads; the `GObject`
//! never does.
//!
//! What follows the object is only what something other than the `GObject`
//! can change. A setter that forwards to the core and notifies itself needs
//! no stream here.

use futures_util::{
    Stream, StreamExt,
    stream::{self, BoxStream},
};
use gtk::{glib, prelude::*};
use tokio::task::AbortHandle;

/// A change to apply to the object, on the main thread.
type Apply<O> = Box<dyn FnOnce(&O) + Send>;

/// The streams a `GObject` follows, before the task that drains them exists.
///
/// Build it with [`ObjectWatcher::new`], add streams with
/// [`ObjectWatcher::follow`], then [`ObjectWatcher::spawn`] the task and
/// keep the handle: the task ends when every stream has ended, and aborting
/// the handle from `dispose` ends it sooner.
pub(crate) struct ObjectWatcher<O: ObjectType> {
    weak: glib::SendWeakRef<O>,
    streams: Vec<BoxStream<'static, Apply<O>>>,
}

impl<O: ObjectType + 'static> ObjectWatcher<O> {
    /// A watcher for the given object, following nothing yet.
    pub(crate) fn new(obj: &O) -> Self {
        Self {
            weak: obj.downgrade().into(),
            streams: Vec::new(),
        }
    }

    /// Follow a stream: every item runs `apply` on the object, on the main
    /// thread, in the order the items arrived.
    pub(crate) fn follow<T, S, F>(mut self, stream: S, apply: F) -> Self
    where
        T: Send + 'static,
        S: Stream<Item = T> + Send + 'static,
        F: Fn(&O, T) + Clone + Send + Sync + 'static,
    {
        let stream = stream
            .map(move |value| {
                let apply = apply.clone();
                Box::new(move |obj: &O| apply(obj, value)) as Apply<O>
            })
            .boxed();
        self.streams.push(stream);
        self
    }

    /// Start the task that drains the streams.
    ///
    /// It ends when every stream has ended; an item that arrives after the
    /// object is gone is dropped.
    pub(crate) fn spawn(self) -> AbortHandle {
        let Self { weak, streams } = self;

        crate::spawn_tokio!(async move {
            let mut all = stream::select_all(streams);

            while let Some(apply) = all.next().await {
                let weak = weak.clone();
                glib::MainContext::default().spawn(async move {
                    if let Some(obj) = weak.upgrade() {
                        apply(&obj);
                    }
                });
            }
        })
        .abort_handle()
    }
}
