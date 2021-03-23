use crate::config::{APP_ID, PROFILE};
use crate::FrctlApplication;
use crate::FrctlLogin;
use crate::FrctlSession;
use adw::subclass::prelude::AdwApplicationWindowImpl;
use glib::signal::Inhibit;
use gtk::subclass::prelude::*;
use gtk::{self, prelude::*};
use gtk::{gio, glib, CompositeTemplate};
use log::warn;

mod imp {
    use super::*;
    use glib::subclass::InitializingObject;

    #[derive(Debug, CompositeTemplate)]
    #[template(resource = "/org/gnome/FractalNext/window.ui")]
    pub struct FrctlWindow {
        #[template_child]
        pub main_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub login: TemplateChild<FrctlLogin>,
        // Eventually we want to create the session dynamically, since we want multi account
        // support
        #[template_child]
        pub session: TemplateChild<FrctlSession>,
        pub settings: gio::Settings,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FrctlWindow {
        const NAME: &'static str = "FrctlWindow";
        type Type = super::FrctlWindow;
        type ParentType = adw::ApplicationWindow;

        fn new() -> Self {
            Self {
                main_stack: TemplateChild::default(),
                login: TemplateChild::default(),
                session: TemplateChild::default(),
                settings: gio::Settings::new(APP_ID),
            }
        }

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for FrctlWindow {
        fn constructed(&self, obj: &Self::Type) {
            self.parent_constructed(obj);

            let builder = gtk::Builder::from_resource("/org/gnome/FractalNext/shortcuts.ui");
            let shortcuts = builder.get_object("shortcuts").unwrap();
            obj.set_help_overlay(Some(&shortcuts));

            // Devel Profile
            if PROFILE == "Devel" {
                obj.add_css_class("devel");
            }

            // load latest window state
            obj.load_window_size();
        }
    }

    impl WindowImpl for FrctlWindow {
        // save window state on delete event
        fn close_request(&self, obj: &Self::Type) -> Inhibit {
            if let Err(err) = obj.save_window_size() {
                warn!("Failed to save window state, {}", &err);
            }
            Inhibit(false)
        }
    }

    impl WidgetImpl for FrctlWindow {}
    impl ApplicationWindowImpl for FrctlWindow {}
    impl AdwApplicationWindowImpl for FrctlWindow {}
}

glib::wrapper! {
    pub struct FrctlWindow(ObjectSubclass<imp::FrctlWindow>)
        @extends gtk::Widget, gtk::Window, gtk::ApplicationWindow, adw::ApplicationWindow, @implements gio::ActionMap, gio::ActionGroup;
}

impl FrctlWindow {
    pub fn new(app: &FrctlApplication) -> Self {
        glib::Object::new(&[("application", &Some(app)), ("icon-name", &Some(APP_ID))])
            .expect("Failed to create FrctlWindow")
    }

    pub fn save_window_size(&self) -> Result<(), glib::BoolError> {
        let settings = &imp::FrctlWindow::from_instance(self).settings;

        let size = self.get_default_size();

        settings.set_int("window-width", size.0)?;
        settings.set_int("window-height", size.1)?;

        settings.set_boolean("is-maximized", self.is_maximized())?;

        Ok(())
    }

    fn load_window_size(&self) {
        let settings = &imp::FrctlWindow::from_instance(self).settings;

        let width = settings.get_int("window-width");
        let height = settings.get_int("window-height");
        let is_maximized = settings.get_boolean("is-maximized");

        self.set_default_size(width, height);
        self.set_property_maximized(is_maximized);
    }
}
