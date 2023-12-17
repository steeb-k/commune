use adw::subclass::prelude::*;
use gtk::{gdk, glib, prelude::*, CompositeTemplate};
use sourceview::prelude::*;

use crate::session::model::Event;

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/session/view/event_source_dialog.ui")]
    #[properties(wrapper_type = super::EventSourceDialog)]
    pub struct EventSourceDialog {
        /// The event that is displayed in the dialog.
        #[property(get, construct_only)]
        pub event: RefCell<Option<Event>>,
        #[template_child]
        pub source_view: TemplateChild<sourceview::View>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for EventSourceDialog {
        const NAME: &'static str = "EventSourceDialog";
        type Type = super::EventSourceDialog;
        type ParentType = adw::Window;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            klass.install_action("event-source-dialog.copy", None, move |widget, _, _| {
                widget.copy_to_clipboard();
            });

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
    impl ObjectImpl for EventSourceDialog {
        fn constructed(&self) {
            let buffer = self
                .source_view
                .buffer()
                .downcast::<sourceview::Buffer>()
                .unwrap();

            let json_lang = sourceview::LanguageManager::default().language("json");
            buffer.set_language(json_lang.as_ref());
            crate::utils::sourceview::setup_style_scheme(&buffer);

            self.parent_constructed();
        }
    }

    impl WidgetImpl for EventSourceDialog {}
    impl WindowImpl for EventSourceDialog {}
    impl AdwWindowImpl for EventSourceDialog {}
}

glib::wrapper! {
    pub struct EventSourceDialog(ObjectSubclass<imp::EventSourceDialog>)
        @extends gtk::Widget, gtk::Window, adw::Window, @implements gtk::Accessible;
}

impl EventSourceDialog {
    pub fn new(window: &gtk::Window, event: &Event) -> Self {
        glib::Object::builder()
            .property("transient-for", window)
            .property("event", event)
            .build()
    }

    pub fn copy_to_clipboard(&self) {
        let clipboard = self.clipboard();
        let buffer = self.imp().source_view.buffer();
        let (start_iter, end_iter) = buffer.bounds();
        clipboard.set_text(buffer.text(&start_iter, &end_iter, true).as_ref());
    }
}
