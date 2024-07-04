use gtk::{
    glib,
    glib::{clone, closure_local},
    prelude::*,
    subclass::prelude::*,
};
use matrix_sdk_ui::timeline::{EditInfo, RepliedToInfo, TimelineEventItemId};
use sourceview::prelude::*;

use crate::session::model::{EventKey, Room};

mod imp {
    use std::{cell::RefCell, marker::PhantomData};

    use glib::subclass::Signal;
    use once_cell::sync::Lazy;

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::ComposerState)]
    pub struct ComposerState {
        /// The room associated with this state.
        #[property(get, construct_only, nullable)]
        pub room: glib::WeakRef<Room>,
        /// The buffer of this state.
        #[property(get)]
        pub buffer: sourceview::Buffer,
        /// The relation of this state.
        pub related_to: RefCell<Option<RelationInfo>>,
        /// Whether this state has a relation.
        #[property(get = Self::has_relation)]
        pub has_relation: PhantomData<bool>,
        /// The widgets of this state.
        ///
        /// These are the widgets inserted in the composer.
        pub widgets: RefCell<Vec<(gtk::Widget, gtk::TextChildAnchor)>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ComposerState {
        const NAME: &'static str = "ContentComposerState";
        type Type = super::ComposerState;
    }

    #[glib::derived_properties]
    impl ObjectImpl for ComposerState {
        fn signals() -> &'static [Signal] {
            static SIGNALS: Lazy<Vec<Signal>> =
                Lazy::new(|| vec![Signal::builder("related-to-changed").build()]);
            SIGNALS.as_ref()
        }

        fn constructed(&self) {
            self.parent_constructed();

            crate::utils::sourceview::setup_style_scheme(&self.buffer);

            // Markdown highlighting.
            let md_lang = sourceview::LanguageManager::default().language("markdown");
            self.buffer.set_language(md_lang.as_ref());

            self.buffer.connect_delete_range(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_, _, _| {
                    imp.update_widgets();
                }
            ));
        }
    }

    impl ComposerState {
        /// Whether this state has a relation.
        fn has_relation(&self) -> bool {
            self.related_to.borrow().is_some()
        }

        /// Update the list of widgets present in the composer.
        pub(super) fn update_widgets(&self) {
            self.widgets
                .borrow_mut()
                .retain(|(_w, anchor)| !anchor.is_deleted());
        }
    }
}

glib::wrapper! {
    /// The composer state for a room.
    ///
    /// This allows to save and restore the composer state between room changes.
    /// It keeps track of the related event and restores the state of the composer's `GtkSourceView`.
    pub struct ComposerState(ObjectSubclass<imp::ComposerState>);
}

impl ComposerState {
    /// Create a new empty `ComposerState` for the given room.
    pub fn new(room: Option<&Room>) -> Self {
        glib::Object::builder().property("room", room).build()
    }

    /// Attach the buffer of this state to the given view.
    pub fn attach_to_view(&self, view: &sourceview::View) {
        let imp = self.imp();
        view.set_buffer(Some(&imp.buffer));

        imp.update_widgets();

        for (widget, anchor) in &*imp.widgets.borrow() {
            view.add_child_at_anchor(widget, anchor);
        }
    }

    /// Clear this state.
    pub fn clear(&self) {
        self.set_related_to(None);

        let imp = self.imp();
        imp.buffer.set_text("");
        imp.widgets.borrow_mut().clear();
    }

    /// The relation to send with the current message.
    pub fn related_to(&self) -> Option<RelationInfo> {
        self.imp().related_to.borrow().clone()
    }

    /// Set the relation to send with the current message.
    pub fn set_related_to(&self, related_to: Option<RelationInfo>) {
        let imp = self.imp();

        let had_relation = self.has_relation();

        if imp
            .related_to
            .borrow()
            .as_ref()
            .is_some_and(|r| matches!(r, RelationInfo::Edit(_)))
        {
            // The user aborted the edit or the edit is done, clean up the entry.
            imp.buffer.set_text("");
        }

        imp.related_to.replace(related_to);

        if self.has_relation() != had_relation {
            self.notify_has_relation();
        }

        self.emit_by_name::<()>("related-to-changed", &[]);
    }

    /// Add the given widget and anchor to this state.
    pub fn add_widget(&self, widget: impl IsA<gtk::Widget>, anchor: gtk::TextChildAnchor) {
        self.imp()
            .widgets
            .borrow_mut()
            .push((widget.upcast(), anchor));
    }

    /// Connect to the signal emitted when the relation changed.
    pub fn connect_related_to_changed<F: Fn(&Self) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_closure(
            "related-to-changed",
            true,
            closure_local!(move |obj: Self| {
                f(&obj);
            }),
        )
    }
}

/// The possible relations to send with a message.
#[derive(Debug, Clone)]
pub enum RelationInfo {
    /// Send a reply with the given replied to info.
    Reply(RepliedToInfo),

    /// Send an edit with the given edit info.
    Edit(EditInfo),
}

impl RelationInfo {
    /// The unique key of the related event.
    pub fn key(&self) -> EventKey {
        match self {
            RelationInfo::Reply(info) => EventKey::EventId(info.event_id().to_owned()),
            RelationInfo::Edit(info) => match info.id() {
                TimelineEventItemId::TransactionId(txn_id) => {
                    EventKey::TransactionId(txn_id.clone())
                }
                TimelineEventItemId::EventId(event_id) => EventKey::EventId(event_id.clone()),
            },
        }
    }
}
