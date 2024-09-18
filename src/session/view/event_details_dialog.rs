use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{glib, CompositeTemplate};
use sourceview::prelude::*;

use crate::{
    components::{CopyableRow, ToastableDialog},
    prelude::*,
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
        pub navigation_view: TemplateChild<adw::NavigationView>,
        #[template_child]
        pub source_page: TemplateChild<adw::NavigationPage>,
        #[template_child]
        pub source_view: TemplateChild<sourceview::View>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for EventDetailsDialog {
        const NAME: &'static str = "EventDetailsDialog";
        type Type = super::EventDetailsDialog;
        type ParentType = ToastableDialog;

        fn class_init(klass: &mut Self::Class) {
            CopyableRow::ensure_type();

            Self::bind_template(klass);
            Self::Type::bind_template_callbacks(klass);

            klass.install_action("event-details-dialog.copy-source", None, |obj, _, _| {
                let clipboard = obj.clipboard();
                let buffer = obj.imp().source_view.buffer();
                let (start_iter, end_iter) = buffer.bounds();
                clipboard.set_text(&buffer.text(&start_iter, &end_iter, true));
                toast!(obj, gettext("Source copied to clipboard"))
            });
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
                .source_view
                .buffer()
                .downcast::<sourceview::Buffer>()
                .unwrap();
            buffer.set_language(json_lang.as_ref());
            utils::sourceview::setup_style_scheme(&buffer);
        }
    }

    impl WidgetImpl for EventDetailsDialog {}
    impl AdwDialogImpl for EventDetailsDialog {}
    impl ToastableDialogImpl for EventDetailsDialog {}
}

glib::wrapper! {
    /// A dialog showing the details of an event.
    pub struct EventDetailsDialog(ObjectSubclass<imp::EventDetailsDialog>)
        @extends gtk::Widget, adw::Dialog, ToastableDialog, @implements gtk::Accessible;
}

#[gtk::template_callbacks]
impl EventDetailsDialog {
    pub fn new(event: &Event) -> Self {
        glib::Object::builder().property("event", event).build()
    }

    /// View the given source.
    fn show_source(&self, title: &str, source: &str) {
        let imp = self.imp();

        imp.source_view.buffer().set_text(source);
        imp.source_page.set_title(title);
        imp.navigation_view.push_by_tag("source");
    }

    /// View the original source.
    #[template_callback]
    fn show_original_source(&self) {
        let Some(event) = self.event() else {
            return;
        };

        if let Some(source) = event.source() {
            let title = if event.is_edited() {
                gettext("Original Event Source")
            } else {
                gettext("Event Source")
            };
            self.show_source(&title, &source);
        }
    }

    /// View the source of the latest edit.
    #[template_callback]
    fn show_edit_source(&self) {
        let Some(event) = self.event() else {
            return;
        };

        let source = event.latest_edit_source();
        let title = gettext("Latest Edit Source");
        self.show_source(&title, &source);
    }
}
