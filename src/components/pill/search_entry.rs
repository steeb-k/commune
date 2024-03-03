use adw::{prelude::*, subclass::prelude::*};
use gtk::{
    glib,
    glib::{clone, closure_local},
    CompositeTemplate,
};

use crate::{
    components::{Pill, PillSource},
    prelude::*,
};

mod imp {
    use std::{cell::RefCell, collections::HashMap, marker::PhantomData};

    use glib::subclass::{InitializingObject, Signal};
    use once_cell::sync::Lazy;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/components/pill/search_entry.ui")]
    #[properties(wrapper_type = super::PillSearchEntry)]
    pub struct PillSearchEntry {
        #[template_child]
        pub text_view: TemplateChild<gtk::TextView>,
        #[template_child]
        pub text_buffer: TemplateChild<gtk::TextBuffer>,
        /// The text of the entry.
        #[property(get = Self::text)]
        text: PhantomData<glib::GString>,
        /// Whether the entry is editable.
        #[property(get = Self::editable, set = Self::set_editable, explicit_notify)]
        editable: PhantomData<bool>,
        /// The pills in the text view.
        ///
        /// A map of pill identifier to anchor of the pill in the text view.
        pub pills: RefCell<HashMap<String, gtk::TextChildAnchor>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PillSearchEntry {
        const NAME: &'static str = "PillSearchEntry";
        type Type = super::PillSearchEntry;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for PillSearchEntry {
        fn signals() -> &'static [Signal] {
            static SIGNALS: Lazy<Vec<Signal>> = Lazy::new(|| {
                vec![Signal::builder("pill-removed")
                    .param_types([PillSource::static_type()])
                    .build()]
            });
            SIGNALS.as_ref()
        }

        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();

            self.text_buffer
                .connect_delete_range(clone!(@weak obj => move |_, start, end| {
                    if start == end {
                        // Nothing to do.
                        return;
                    }

                    let mut current = *start;
                    loop {
                        if let Some(source) = current
                            .child_anchor()
                            .and_then(|a| a.widgets().first().cloned())
                            .and_downcast_ref::<Pill>()
                            .and_then(|p| p.source())
                        {
                            let removed = obj.imp().pills.borrow_mut().remove(&source.identifier()).is_some();

                            if removed {
                                obj.emit_by_name::<()>("pill-removed", &[&source]);
                            }
                        }

                        current.forward_char();

                        if &current == end {
                            break;
                        }
                    }
                }));

            self.text_buffer
                .connect_insert_text(|text_buffer, location, text| {
                    let mut changed = false;

                    // We don't allow adding chars before and between pills
                    loop {
                        if location.child_anchor().is_some() {
                            changed = true;
                            if !location.forward_char() {
                                break;
                            }
                        } else {
                            break;
                        }
                    }

                    if changed {
                        text_buffer.place_cursor(location);
                        text_buffer.stop_signal_emission_by_name("insert-text");
                        text_buffer.insert(location, text);
                    }
                });

            self.text_buffer
                .connect_text_notify(clone!(@weak obj => move |_| {
                    obj.notify_text();
                }));
        }
    }

    impl WidgetImpl for PillSearchEntry {}
    impl BinImpl for PillSearchEntry {}

    impl PillSearchEntry {
        /// The text of the entry.
        fn text(&self) -> glib::GString {
            let (start, end) = self.text_buffer.bounds();
            self.text_buffer.text(&start, &end, false)
        }

        /// Whether the entry is editable.
        fn editable(&self) -> bool {
            self.text_view.is_editable()
        }

        /// Set whether the entry is editable.
        fn set_editable(&self, editable: bool) {
            if self.editable() == editable {
                return;
            }

            self.text_view.set_editable(editable);
            self.obj().notify_editable();
        }
    }
}

glib::wrapper! {
    /// Search entry where selected results can be added as [`Pill`]s.
    pub struct PillSearchEntry(ObjectSubclass<imp::PillSearchEntry>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl PillSearchEntry {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Add a pill for the given source to the entry.
    pub fn add_pill(&self, source: &impl IsA<PillSource>) {
        let imp = self.imp();
        let identifier = source.identifier();

        // If the pill already exists, don't insert it again.
        if imp.pills.borrow().contains_key(&identifier) {
            return;
        }

        let pill = Pill::new(source);
        pill.set_margin_start(3);
        pill.set_margin_end(3);

        let (mut start_iter, mut end_iter) = imp.text_buffer.bounds();

        // We don't allow adding chars before and between pills
        loop {
            if start_iter.child_anchor().is_some() {
                start_iter.forward_char();
            } else {
                break;
            }
        }

        imp.text_buffer.delete(&mut start_iter, &mut end_iter);
        let anchor = imp.text_buffer.create_child_anchor(&mut start_iter);
        imp.text_view.add_child_at_anchor(&pill, &anchor);
        imp.pills.borrow_mut().insert(identifier, anchor);

        imp.text_view.grab_focus();
    }

    /// Remove the pill with the given identifier.
    pub fn remove_pill(&self, identifier: &str) {
        let imp = self.imp();

        let Some(anchor) = imp.pills.borrow_mut().remove(identifier) else {
            return;
        };

        if anchor.is_deleted() {
            // Nothing to do.
            return;
        }

        let text_buffer = &self.imp().text_buffer;
        let mut start_iter = text_buffer.iter_at_child_anchor(&anchor);
        let mut end_iter = start_iter;
        end_iter.forward_char();
        text_buffer.delete(&mut start_iter, &mut end_iter);
    }

    /// Clear this entry.
    pub fn clear(&self) {
        let text_buffer = &self.imp().text_buffer;
        let (mut start, mut end) = text_buffer.bounds();
        text_buffer.delete(&mut start, &mut end);
    }

    /// Connect to the signal emitted when a pill is removed from the entry.
    ///
    /// The second parameter is the source of the pill.
    pub fn connect_pill_removed<F: Fn(&Self, PillSource) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_closure(
            "pill-removed",
            true,
            closure_local!(|obj: Self, source: PillSource| {
                f(&obj, source);
            }),
        )
    }
}
