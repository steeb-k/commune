use crate::session::sidebar::HighlightFlags;
use adw;
use adw::subclass::prelude::BinImpl;
use gtk::subclass::prelude::*;
use gtk::{self, prelude::*};
use gtk::{gio, glib, CompositeTemplate};

mod imp {
    use super::*;
    use glib::subclass::InitializingObject;

    #[derive(Debug, CompositeTemplate)]
    #[template(resource = "/org/gnome/FractalNext/sidebar-room-row.ui")]
    pub struct SidebarRoomRow {
        #[template_child]
        pub avatar: TemplateChild<adw::Avatar>,
        #[template_child]
        pub display_name: TemplateChild<gtk::Label>,
        #[template_child]
        pub notification_count: TemplateChild<gtk::Label>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SidebarRoomRow {
        const NAME: &'static str = "SidebarRoomRow";
        type Type = super::SidebarRoomRow;
        type ParentType = adw::Bin;

        fn new() -> Self {
            Self {
                avatar: TemplateChild::default(),
                display_name: TemplateChild::default(),
                notification_count: TemplateChild::default(),
            }
        }

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for SidebarRoomRow {
        fn properties() -> &'static [glib::ParamSpec] {
            use once_cell::sync::Lazy;
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![
                    glib::ParamSpec::new_object(
                        "avatar",
                        "Avatar",
                        "The url of the avatar of this room",
                        gio::LoadableIcon::static_type(),
                        glib::ParamFlags::WRITABLE,
                    ),
                    glib::ParamSpec::new_string(
                        "display-name",
                        "Display Name",
                        "The display name of this room",
                        None,
                        glib::ParamFlags::WRITABLE,
                    ),
                    glib::ParamSpec::new_flags(
                        "highlight",
                        "Highlight",
                        "What type of highligh this room needs",
                        HighlightFlags::static_type(),
                        HighlightFlags::default().bits(),
                        glib::ParamFlags::WRITABLE,
                    ),
                    glib::ParamSpec::new_uint64(
                        "notification-count",
                        "Notification count",
                        "The notification count of this room",
                        std::u64::MIN,
                        std::u64::MAX,
                        0,
                        glib::ParamFlags::WRITABLE,
                    ),
                ]
            });

            PROPERTIES.as_ref()
        }

        fn set_property(
            &self,
            _obj: &Self::Type,
            _id: usize,
            value: &glib::Value,
            pspec: &glib::ParamSpec,
        ) {
            match pspec.name() {
                "avatar" => {
                    let _avatar = value
                        .get::<Option<gio::LoadableIcon>>()
                        .expect("type conformity checked by `Object::set_property`");
                    // TODO: set custom avatar https://gitlab.gnome.org/exalm/libadwaita/-/issues/29
                }
                "display-name" => {
                    let display_name = value
                        .get()
                        .expect("type conformity checked by `Object::set_property`");
                    self.display_name.set_label(display_name);
                }
                "highlight" => {
                    let flags = value
                        .get::<HighlightFlags>()
                        .expect("type conformity checked by `Object::set_property`");
                    match flags {
                        HighlightFlags::NONE => {
                            self.notification_count.remove_css_class("highlight");
                            self.display_name.remove_css_class("bold");
                        }
                        HighlightFlags::HIGHLIGHT => {
                            self.notification_count.add_css_class("highlight");
                            self.display_name.remove_css_class("bold");
                        }
                        HighlightFlags::BOLD => {
                            self.display_name.add_css_class("bold");
                            self.notification_count.remove_css_class("highlight");
                        }
                        HighlightFlags::HIGHLIGHT_BOLD => {
                            self.notification_count.add_css_class("highlight");
                            self.display_name.add_css_class("bold");
                        }
                        _ => {}
                    }
                }
                "notification-count" => {
                    let count = value
                        .get::<u64>()
                        .expect("type conformity checked by `Object::set_property`");
                    self.notification_count.set_label(&count.to_string());
                    self.notification_count.set_visible(count > 0);
                }
                _ => unimplemented!(),
            }
        }

        fn constructed(&self, obj: &Self::Type) {
            self.parent_constructed(obj);
        }
    }

    impl WidgetImpl for SidebarRoomRow {}
    impl BinImpl for SidebarRoomRow {}
}

glib::wrapper! {
    pub struct SidebarRoomRow(ObjectSubclass<imp::SidebarRoomRow>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl SidebarRoomRow {
    pub fn new() -> Self {
        glib::Object::new(&[]).expect("Failed to create SidebarRoomRow")
    }
}
