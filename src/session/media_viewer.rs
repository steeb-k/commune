use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{
    gdk, gdk_pixbuf::Pixbuf, gio, glib, glib::clone, subclass::prelude::*, CompositeTemplate,
};
use log::warn;
use matrix_sdk::ruma::events::{room::message::MessageType, AnyMessageEventContent};

use crate::{
    components::{ContextMenuBin, ContextMenuBinImpl},
    session::room::Event,
    spawn,
    utils::cache_dir,
    Window,
};

use super::room::EventActions;

mod imp {
    use crate::components::ContextMenuBinExt;

    use super::*;
    use glib::object::WeakRef;
    use glib::subclass::InitializingObject;
    use once_cell::sync::Lazy;
    use std::cell::{Cell, RefCell};

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(resource = "/org/gnome/FractalNext/media-viewer.ui")]
    pub struct MediaViewer {
        pub fullscreened: Cell<bool>,
        pub event: RefCell<Option<WeakRef<Event>>>,
        pub body: RefCell<Option<String>>,
        #[template_child]
        pub headerbar_revealer: TemplateChild<gtk::Revealer>,
        #[template_child]
        pub menu_full: TemplateChild<gtk::MenuButton>,
        #[template_child]
        pub media: TemplateChild<adw::Bin>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MediaViewer {
        const NAME: &'static str = "MediaViewer";
        type Type = super::MediaViewer;
        type ParentType = ContextMenuBin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);

            klass.install_action("media-viewer.close", None, move |obj, _, _| {
                let priv_ = imp::MediaViewer::from_instance(obj);
                if let Some(stream) = priv_
                    .media
                    .child()
                    .and_then(|w| w.downcast::<gtk::Video>().ok())
                    .and_then(|video| video.media_stream())
                {
                    if stream.is_playing() {
                        stream.pause();
                        stream.seek(0);
                    }
                }
                obj.activate_action("session.show-content", None);
            });
            klass.add_binding_action(
                gdk::keys::constants::Escape,
                gdk::ModifierType::empty(),
                "media-viewer.close",
                None,
            );
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for MediaViewer {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![
                    glib::ParamSpec::new_boolean(
                        "fullscreened",
                        "Fullscreened",
                        "Whether the viewer is fullscreen",
                        false,
                        glib::ParamFlags::READWRITE | glib::ParamFlags::EXPLICIT_NOTIFY,
                    ),
                    glib::ParamSpec::new_object(
                        "event",
                        "Event",
                        "The media event to display",
                        Event::static_type(),
                        glib::ParamFlags::READWRITE | glib::ParamFlags::EXPLICIT_NOTIFY,
                    ),
                    glib::ParamSpec::new_string(
                        "body",
                        "Body",
                        "The body of the media event",
                        None,
                        glib::ParamFlags::READABLE,
                    ),
                ]
            });

            PROPERTIES.as_ref()
        }

        fn set_property(
            &self,
            obj: &Self::Type,
            _id: usize,
            value: &glib::Value,
            pspec: &glib::ParamSpec,
        ) {
            match pspec.name() {
                "fullscreened" => obj.set_fullscreened(value.get().unwrap()),
                "event" => obj.set_event(value.get().unwrap()),
                _ => unimplemented!(),
            }
        }

        fn property(&self, obj: &Self::Type, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "fullscreened" => obj.fullscreened().to_value(),
                "event" => obj.event().to_value(),
                "body" => obj.body().to_value(),
                _ => unimplemented!(),
            }
        }

        fn constructed(&self, obj: &Self::Type) {
            self.parent_constructed(obj);

            let menu_model = Some(Self::Type::event_menu_model());
            self.menu_full.set_menu_model(menu_model);
            obj.set_context_menu(menu_model);

            // Bind `fullscreened` to the window property of the same name.
            obj.connect_notify_local(Some("root"), |obj, _| {
                if let Some(window) = obj.root().and_then(|root| root.downcast::<Window>().ok()) {
                    window
                        .bind_property("fullscreened", obj, "fullscreened")
                        .flags(glib::BindingFlags::SYNC_CREATE)
                        .build();
                }
            });

            // Toggle fullscreen on double click.
            let click_gesture = gtk::GestureClick::builder().button(1).build();
            click_gesture.connect_pressed(clone!(@weak obj => move |_, n_pressed, _, _| {
                if n_pressed == 2 {
                    obj.activate_action("win.toggle-fullscreen", None);
                }
            }));
            obj.add_controller(&click_gesture);

            // Show headerbar when revealer is hovered.
            let revealer: &gtk::Revealer = &*self.headerbar_revealer;
            let menu: &gtk::MenuButton = &*self.menu_full;
            let motion_controller = gtk::EventControllerMotion::new();
            motion_controller.connect_enter(clone!(@weak revealer => move |_, _, _| {
                revealer.set_reveal_child(true);
            }));
            // Hide the headerbar when revealer is not hovered and header menu is closed.
            motion_controller.connect_leave(clone!(@weak revealer, @weak menu => move |_| {
                if menu.popover().filter(|popover| popover.is_visible()).is_none() {
                    revealer.set_reveal_child(false);
                }
            }));
            menu.popover().unwrap().connect_closed(
                clone!(@weak revealer, @weak motion_controller, => move |_| {
                    if !motion_controller.contains_pointer() {
                        revealer.set_reveal_child(false);
                    }
                }),
            );
            revealer.add_controller(&motion_controller);
        }
    }

    impl WidgetImpl for MediaViewer {}
    impl BinImpl for MediaViewer {}
    impl ContextMenuBinImpl for MediaViewer {}
}

glib::wrapper! {
    pub struct MediaViewer(ObjectSubclass<imp::MediaViewer>)
        @extends gtk::Widget, adw::Bin, ContextMenuBin, @implements gtk::Accessible;
}

impl MediaViewer {
    pub fn new() -> Self {
        glib::Object::new(&[]).expect("Failed to create MediaViewer")
    }

    pub fn event(&self) -> Option<Event> {
        let priv_ = imp::MediaViewer::from_instance(self);
        priv_
            .event
            .borrow()
            .as_ref()
            .and_then(|event| event.upgrade())
    }

    pub fn set_event(&self, event: Option<Event>) {
        let priv_ = imp::MediaViewer::from_instance(self);

        if event == self.event() {
            return;
        }

        priv_.event.replace(event.map(|event| event.downgrade()));
        self.build();
        self.notify("event");
    }

    pub fn body(&self) -> Option<String> {
        let priv_ = imp::MediaViewer::from_instance(self);
        priv_.body.borrow().clone()
    }

    pub fn set_body(&self, body: Option<String>) {
        let priv_ = imp::MediaViewer::from_instance(self);

        if body == self.body() {
            return;
        }

        priv_.body.replace(body);
        self.notify("body");
    }

    pub fn fullscreened(&self) -> bool {
        let priv_ = imp::MediaViewer::from_instance(self);
        priv_.fullscreened.get()
    }

    pub fn set_fullscreened(&self, fullscreened: bool) {
        let priv_ = imp::MediaViewer::from_instance(self);

        if fullscreened == self.fullscreened() {
            return;
        }

        priv_.fullscreened.set(fullscreened);

        // Upscale the media on fullscreen
        if fullscreened {
            priv_.media.set_halign(gtk::Align::Fill);
        } else {
            priv_.media.set_halign(gtk::Align::Center);
        }

        self.notify("fullscreened");
    }

    fn build(&self) {
        if let Some(event) = self.event() {
            self.set_event_actions(Some(&event));
            if let Some(AnyMessageEventContent::RoomMessage(content)) = event.message_content() {
                match content.msgtype {
                    MessageType::Image(image) => {
                        self.set_body(Some(image.body.clone()));

                        spawn!(
                            glib::PRIORITY_LOW,
                            clone!(@weak self as obj => async move {
                                let priv_ = imp::MediaViewer::from_instance(&obj);

                                match event.get_media_content().await {
                                    Ok((_, _, data)) => {
                                        let stream = gio::MemoryInputStream::from_bytes(&glib::Bytes::from(&data));
                                        let texture = Pixbuf::from_stream(&stream, gio::NONE_CANCELLABLE)
                                            .ok()
                                            .map(|pixbuf| gdk::Texture::for_pixbuf(&pixbuf));
                                        let child = gtk::Picture::for_paintable(texture.as_ref());

                                        priv_.media.set_child(Some(&child));
                                    }
                                    Err(error) => {
                                        warn!("Could not retrieve image file: {}", error);
                                        let child = gtk::Label::new(Some(&gettext("Could not retrieve image")));
                                        priv_.media.set_child(Some(&child));
                                    }
                                }
                            })
                        );
                    }
                    MessageType::Video(video) => {
                        self.set_body(Some(video.body.clone()));

                        spawn!(
                            glib::PRIORITY_LOW,
                            clone!(@weak self as obj => async move {
                                let priv_ = imp::MediaViewer::from_instance(&obj);

                                match event.get_media_content().await {
                                    Ok((uid, filename, data)) => {
                                        // The GStreamer backend of GtkVideo doesn't work with input streams so
                                        // we need to store the file.
                                        // See: https://gitlab.gnome.org/GNOME/gtk/-/issues/4062
                                        let mut path = cache_dir();
                                        path.push(format!("{}_{}", uid, filename));
                                        let file = gio::File::for_path(path);
                                        file.replace_contents(
                                            &data,
                                            None,
                                            false,
                                            gio::FileCreateFlags::REPLACE_DESTINATION,
                                            gio::NONE_CANCELLABLE,
                                        )
                                        .unwrap();
                                        let child = gtk::Video::builder().file(&file).autoplay(true).build();

                                        priv_.media.set_child(Some(&child));
                                    }
                                    Err(error) => {
                                        warn!("Could not retrieve video file: {}", error);
                                        let child = gtk::Label::new(Some(&gettext("Could not retrieve video")));
                                        priv_.media.set_child(Some(&child));
                                    }
                                }
                            })
                        );
                    }
                    _ => {}
                }
            }
        }
    }
}

impl EventActions for MediaViewer {}
