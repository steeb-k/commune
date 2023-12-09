use adw::subclass::prelude::*;
use gtk::{glib, prelude::*, CompositeTemplate};
use html2pango::markup;

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/components/room_title.ui")]
    #[properties(wrapper_type = super::RoomTitle)]
    pub struct RoomTitle {
        // The title of the room.
        #[property(get, set = Self::set_title, explicit_notify)]
        pub title: RefCell<Option<String>>,
        // The subtitle of the room.
        #[property(get, set = Self::set_subtitle, explicit_notify)]
        pub subtitle: RefCell<Option<String>>,
        #[template_child]
        pub title_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub subtitle_label: TemplateChild<gtk::Label>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RoomTitle {
        const NAME: &'static str = "RoomTitle";
        type Type = super::RoomTitle;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);

            klass.set_css_name("roomtitle");
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for RoomTitle {}

    impl WidgetImpl for RoomTitle {}
    impl BinImpl for RoomTitle {}

    impl RoomTitle {
        /// Set the title of the room.
        fn set_title(&self, title: Option<String>) {
            // Parse and escape markup in title.
            let title = title.map(|s| markup(&s));

            if *self.title.borrow() == title {
                return;
            }

            self.title.replace(title);
            self.title_label.set_visible(self.title.borrow().is_some());

            self.obj().notify_title();
        }

        /// Set the subtitle of the room.
        pub fn set_subtitle(&self, subtitle: Option<String>) {
            // Parse and escape markup in subtitle.
            let subtitle = subtitle.map(|s| markup(&s));

            if *self.subtitle.borrow() == subtitle {
                return;
            }

            self.subtitle.replace(subtitle);
            self.subtitle_label
                .set_visible(self.subtitle.borrow().is_some());

            self.obj().notify_subtitle();
        }
    }
}

glib::wrapper! {
    /// A widget to show a room's title and topic in a header bar.
    pub struct RoomTitle(ObjectSubclass<imp::RoomTitle>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl RoomTitle {
    pub fn new() -> Self {
        glib::Object::new()
    }
}

impl Default for RoomTitle {
    fn default() -> Self {
        Self::new()
    }
}
