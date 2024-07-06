use adw::{prelude::*, subclass::prelude::*};
use gtk::{glib, CompositeTemplate};

use crate::{prelude::*, utils::string::linkify};

mod imp {
    use std::cell::{OnceCell, RefCell};

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/components/room_title.ui")]
    #[properties(wrapper_type = super::RoomTitle)]
    pub struct RoomTitle {
        // The title of the room.
        #[property(get, set = Self::set_title, explicit_notify)]
        pub title: RefCell<Option<String>>,
        // The title of the room that can be presented on a single line.
        #[property(get)]
        pub title_excerpt: RefCell<Option<String>>,
        // The subtitle of the room.
        #[property(get, set = Self::set_subtitle, explicit_notify)]
        pub subtitle: RefCell<Option<String>>,
        // The subtitle of the room that can be presented on a single line.
        #[property(get)]
        pub subtitle_excerpt: RefCell<Option<String>>,
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub title_excerpt_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub start_bin: TemplateChild<adw::Bin>,
        #[template_child]
        pub arrow_icon: TemplateChild<gtk::Image>,
        size_group: OnceCell<gtk::SizeGroup>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RoomTitle {
        const NAME: &'static str = "RoomTitle";
        type Type = super::RoomTitle;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);

            klass.set_css_name("room-title");
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for RoomTitle {
        fn constructed(&self) {
            self.parent_constructed();

            let size_group = self
                .size_group
                .get_or_init(|| gtk::SizeGroup::new(gtk::SizeGroupMode::Horizontal));
            size_group.add_widget(&*self.start_bin);
            size_group.add_widget(&*self.arrow_icon);
        }
    }

    impl WidgetImpl for RoomTitle {}
    impl BinImpl for RoomTitle {}

    impl RoomTitle {
        /// Set the title of the room.
        fn set_title(&self, original_title: Option<String>) {
            let title = original_title.as_deref().map(|s| {
                // Detect links.
                let mut s = linkify(s);
                // Remove trailing spaces.
                s.truncate_end_whitespaces();
                s
            });

            if *self.title.borrow() == title {
                return;
            }

            let has_title = title.is_some();
            let title_excerpt = original_title.map(|s| {
                // Remove newlines.
                let mut s = s.replace('\n', "");
                // Remove trailing spaces.
                s.truncate_end_whitespaces();
                s
            });

            self.title.replace(title);
            self.title_excerpt.replace(title_excerpt);

            let obj = self.obj();
            obj.notify_title();
            obj.notify_title_excerpt();

            self.title_excerpt_label.set_visible(has_title);
        }

        /// Set the subtitle of the room.
        pub fn set_subtitle(&self, original_subtitle: Option<String>) {
            let subtitle = original_subtitle.as_deref().map(|s| {
                // Detect links.
                let mut s = linkify(s);
                // Remove trailing spaces.
                s.truncate_end_whitespaces();
                s
            });

            if *self.subtitle.borrow() == subtitle {
                return;
            }

            let has_subtitle = subtitle.is_some();
            let subtitle_excerpt = original_subtitle.map(|s| {
                // Remove newlines.
                let mut s = s.replace('\n', "");
                // Remove trailing spaces.
                s.truncate_end_whitespaces();
                s
            });

            self.subtitle.replace(subtitle);
            self.subtitle_excerpt.replace(subtitle_excerpt);

            let obj = self.obj();
            obj.notify_subtitle();
            obj.notify_subtitle_excerpt();

            // Show the button only if there is a subtitle.
            if has_subtitle {
                self.stack.set_visible_child_name("button");
            } else {
                self.stack.set_visible_child_name("title-only");
            }
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
