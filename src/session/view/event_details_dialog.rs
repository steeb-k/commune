use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{gdk, glib, CompositeTemplate};
use sourceview::prelude::*;

use crate::{
    components::{ToastableWindow, ToastableWindowImpl},
    session::model::Event,
    toast, utils,
};

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/session/view/event_details_dialog.ui")]
    #[properties(wrapper_type = super::EventDetailsDialog)]
    pub struct EventDetailsDialog {
        /// The event that is displayed in the dialog.
        #[property(get, construct_only)]
        pub event: RefCell<Option<Event>>,
        #[template_child]
        pub original_event_id_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub room_id_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub sender_id_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub original_timestamp_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub original_source_view: TemplateChild<sourceview::View>,
        #[template_child]
        pub edit_event_id_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub edit_timestamp_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub edit_source_view: TemplateChild<sourceview::View>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for EventDetailsDialog {
        const NAME: &'static str = "EventDetailsDialog";
        type Type = super::EventDetailsDialog;
        type ParentType = ToastableWindow;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);

            klass.install_action(
                "event-details-dialog.copy-original-event-id",
                None,
                move |obj, _, _| {
                    let clipboard = obj.clipboard();
                    let subtitle = obj.imp().original_event_id_row.subtitle();
                    clipboard.set_text(subtitle.as_deref().unwrap_or_default());
                    toast!(obj, gettext("Event ID copied to clipboard"))
                },
            );

            klass.install_action(
                "event-details-dialog.copy-room-id",
                None,
                move |obj, _, _| {
                    let clipboard = obj.clipboard();
                    let subtitle = obj.imp().room_id_row.subtitle();
                    clipboard.set_text(subtitle.as_deref().unwrap_or_default());
                    toast!(obj, gettext("Room ID copied to clipboard"))
                },
            );

            klass.install_action(
                "event-details-dialog.copy-sender-id",
                None,
                move |obj, _, _| {
                    let clipboard = obj.clipboard();
                    let subtitle = obj.imp().sender_id_row.subtitle();
                    clipboard.set_text(subtitle.as_deref().unwrap_or_default());
                    toast!(obj, gettext("Sender ID copied to clipboard"))
                },
            );

            klass.install_action(
                "event-details-dialog.copy-original-timestamp",
                None,
                move |obj, _, _| {
                    let clipboard = obj.clipboard();
                    let subtitle = obj.imp().original_timestamp_row.subtitle();
                    clipboard.set_text(subtitle.as_deref().unwrap_or_default());
                    toast!(obj, gettext("Timestamp copied to clipboard"))
                },
            );

            klass.install_action(
                "event-details-dialog.copy-original-source",
                None,
                move |obj, _, _| {
                    let clipboard = obj.clipboard();
                    let buffer = obj.imp().original_source_view.buffer();
                    let (start_iter, end_iter) = buffer.bounds();
                    clipboard.set_text(&buffer.text(&start_iter, &end_iter, true));
                    toast!(obj, gettext("Source copied to clipboard"))
                },
            );

            klass.install_action(
                "event-details-dialog.copy-edit-event-id",
                None,
                move |obj, _, _| {
                    let clipboard = obj.clipboard();
                    let subtitle = obj.imp().edit_event_id_row.subtitle();
                    clipboard.set_text(subtitle.as_deref().unwrap_or_default());
                    toast!(obj, gettext("Event ID copied to clipboard"))
                },
            );

            klass.install_action(
                "event-details-dialog.copy-edit-timestamp",
                None,
                move |obj, _, _| {
                    let clipboard = obj.clipboard();
                    let subtitle = obj.imp().edit_timestamp_row.subtitle();
                    clipboard.set_text(subtitle.as_deref().unwrap_or_default());
                    toast!(obj, gettext("Timestamp copied to clipboard"))
                },
            );

            klass.install_action(
                "event-details-dialog.copy-edit-source",
                None,
                move |obj, _, _| {
                    let clipboard = obj.clipboard();
                    let buffer = obj.imp().edit_source_view.buffer();
                    let (start_iter, end_iter) = buffer.bounds();
                    clipboard.set_text(&buffer.text(&start_iter, &end_iter, true));
                    toast!(obj, gettext("Source copied to clipboard"))
                },
            );

            klass.add_binding_action(
                gdk::Key::Escape,
                gdk::ModifierType::empty(),
                "window.close",
                None,
            );
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for EventDetailsDialog {
        fn constructed(&self) {
            self.parent_constructed();

            let json_lang = sourceview::LanguageManager::default().language("json");

            let buffer = self
                .original_source_view
                .buffer()
                .downcast::<sourceview::Buffer>()
                .unwrap();
            buffer.set_language(json_lang.as_ref());
            utils::sourceview::setup_style_scheme(&buffer);

            let buffer = self
                .edit_source_view
                .buffer()
                .downcast::<sourceview::Buffer>()
                .unwrap();
            buffer.set_language(json_lang.as_ref());
            utils::sourceview::setup_style_scheme(&buffer);
        }
    }

    impl WidgetImpl for EventDetailsDialog {}
    impl WindowImpl for EventDetailsDialog {}
    impl AdwWindowImpl for EventDetailsDialog {}
    impl ToastableWindowImpl for EventDetailsDialog {}
}

glib::wrapper! {
    pub struct EventDetailsDialog(ObjectSubclass<imp::EventDetailsDialog>)
        @extends gtk::Widget, gtk::Window, adw::Window, ToastableWindow, @implements gtk::Accessible;
}

impl EventDetailsDialog {
    pub fn new(window: &gtk::Window, event: &Event) -> Self {
        glib::Object::builder()
            .property("transient-for", window)
            .property("event", event)
            .build()
    }
}
