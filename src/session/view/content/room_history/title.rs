use adw::{prelude::*, subclass::prelude::*};
use gtk::{glib, glib::clone, CompositeTemplate};

use crate::{
    components::Avatar,
    prelude::*,
    session::model::Room,
    utils::{string::linkify, BoundObjectWeakRef},
};

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/session/view/content/room_history/title.ui")]
    #[properties(wrapper_type = super::RoomHistoryTitle)]
    pub struct RoomHistoryTitle {
        // The room to present the title of.
        #[property(get, set = Self::set_room, explicit_notify, nullable)]
        pub room: BoundObjectWeakRef<Room>,
        // The title of the room.
        #[property(get)]
        pub title: RefCell<Option<String>>,
        // The title of the room that can be presented on a single line.
        #[property(get)]
        pub title_excerpt: RefCell<Option<String>>,
        // The subtitle of the room.
        #[property(get)]
        pub subtitle: RefCell<Option<String>>,
        // The subtitle of the room that can be presented on a single line.
        #[property(get)]
        pub subtitle_excerpt: RefCell<Option<String>>,
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub title_excerpt_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub popover: TemplateChild<gtk::Popover>,
        #[template_child]
        pub title_label: TemplateChild<gtk::Label>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RoomHistoryTitle {
        const NAME: &'static str = "RoomHistoryTitle";
        type Type = super::RoomHistoryTitle;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Avatar::ensure_type();

            Self::bind_template(klass);
            Self::Type::bind_template_callbacks(klass);

            klass.set_css_name("room-title");
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for RoomHistoryTitle {
        fn constructed(&self) {
            self.parent_constructed();

            self.popover.set_offset(0, 5);
        }
    }

    impl WidgetImpl for RoomHistoryTitle {}
    impl BinImpl for RoomHistoryTitle {}

    impl RoomHistoryTitle {
        /// Set the room to present the title of.
        fn set_room(&self, room: Option<Room>) {
            if self.room.obj() == room {
                return;
            }

            self.room.disconnect_signals();

            if let Some(room) = room {
                let display_name_handler = room.connect_display_name_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.update_title();
                    }
                ));
                let topic_handler = room.connect_topic_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.update_subtitle();
                    }
                ));

                self.room
                    .set(&room, vec![display_name_handler, topic_handler]);
            }

            self.obj().notify_room();
            self.update_title();
            self.update_subtitle();
        }

        /// Update the title of the room.
        fn update_title(&self) {
            let original_title = self.room.obj().map(|r| r.display_name());

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

        /// Update the subtitle of the room.
        fn update_subtitle(&self) {
            let original_subtitle = self.room.obj().and_then(|r| r.topic());

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
    pub struct RoomHistoryTitle(ObjectSubclass<imp::RoomHistoryTitle>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

#[gtk::template_callbacks]
impl RoomHistoryTitle {
    /// Construct a new empty `RoomHistoryTitle`.
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Handle when the title's label was mapped.
    #[template_callback]
    fn title_mapped(&self) {
        let imp = self.imp();
        // Put the cursor at the beginning of the title instead of having the title
        // selected.
        glib::idle_add_local_once(clone!(
            #[weak]
            imp,
            move || {
                imp.title_label.select_region(0, 0);
            }
        ));
    }
}

impl Default for RoomHistoryTitle {
    fn default() -> Self {
        Self::new()
    }
}
