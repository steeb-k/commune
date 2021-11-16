// SPDX-License-Identifier: GPL-3.0-or-later
use crate::spawn;
use ashpd::{desktop::camera, zbus};
use glib::clone;
use glib::subclass;
use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use matrix_sdk::encryption::verification::QrVerificationData;
use std::os::unix::prelude::RawFd;

mod camera_paintable;
mod qr_code_detector;
pub mod screenshot;

use camera_paintable::CameraPaintable;

mod imp {
    use adw::subclass::prelude::*;
    use gtk::CompositeTemplate;
    use once_cell::sync::Lazy;
    use std::cell::Cell;

    use super::*;

    #[derive(Debug, CompositeTemplate, Default)]
    #[template(resource = "/org/gnome/FractalNext/qr-code-scanner.ui")]
    pub struct QrCodeScanner {
        pub paintable: CameraPaintable,
        #[template_child]
        pub picture: TemplateChild<gtk::Picture>,
        pub has_camera: Cell<bool>,
        pub is_started: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for QrCodeScanner {
        const NAME: &'static str = "QrCodeScanner";
        type Type = super::QrCodeScanner;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }
    impl ObjectImpl for QrCodeScanner {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![glib::ParamSpec::new_boolean(
                    "has-camera",
                    "Has Camera",
                    "Whether we have a working camera",
                    false,
                    glib::ParamFlags::READABLE,
                )]
            });

            PROPERTIES.as_ref()
        }

        fn property(&self, obj: &Self::Type, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "has-camera" => obj.has_camera().to_value(),
                _ => unimplemented!(),
            }
        }

        fn constructed(&self, obj: &Self::Type) {
            self.picture.set_paintable(Some(&self.paintable));

            let callback = glib::clone!(@weak obj => @default-return None, move |args: &[glib::Value]| {
                let code = args.get(1).unwrap().get::<QrVerificationDataBoxed>().unwrap();
                obj.emit_by_name("code-detected", &[&code]).unwrap();

                None
            });
            self.paintable
                .connect_local("code-detected", false, callback)
                .unwrap();
            obj.init_has_camera();
        }

        fn signals() -> &'static [subclass::Signal] {
            static SIGNALS: Lazy<Vec<subclass::Signal>> = Lazy::new(|| {
                vec![subclass::Signal::builder(
                    "code-detected",
                    &[QrVerificationDataBoxed::static_type().into()],
                    glib::Type::UNIT.into(),
                )
                .flags(glib::SignalFlags::RUN_FIRST)
                .build()]
            });
            SIGNALS.as_ref()
        }
    }
    impl WidgetImpl for QrCodeScanner {
        fn unmap(&self, widget: &Self::Type) {
            self.parent_unmap(widget);
            widget.stop();
        }
    }
    impl BinImpl for QrCodeScanner {}
}

glib::wrapper! {
    pub struct QrCodeScanner(ObjectSubclass<imp::QrCodeScanner>) @extends gtk::Widget, adw::Bin;
}

impl QrCodeScanner {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        glib::Object::new(&[]).expect("Failed to create a QrCodeScanner")
    }

    pub fn stop(&self) {
        let self_ = imp::QrCodeScanner::from_instance(self);

        self_.paintable.close_pipeline();
    }

    async fn start_internal(&self) -> bool {
        let self_ = imp::QrCodeScanner::from_instance(self);
        if let Ok(Some(stream_fd)) = stream().await {
            if let Ok(node_id) = camera::pipewire_node_id(stream_fd).await {
                self_.paintable.set_pipewire_fd(stream_fd, node_id);
                self_.has_camera.set(true);
                self.notify("has-camera");
                return true;
            }
        }
        self_.has_camera.set(false);
        self.notify("has-camera");
        false
    }

    pub async fn start(&self) -> bool {
        let priv_ = imp::QrCodeScanner::from_instance(self);
        let is_started = self.start_internal().await;
        priv_.is_started.set(is_started);
        is_started
    }

    fn init_has_camera(&self) {
        spawn!(clone!(@weak self as obj => async move {
            let priv_ = imp::QrCodeScanner::from_instance(&obj);
            let has_camera = if obj.start_internal().await {
                if !priv_.is_started.get() {
                    obj.stop();
                }
                true
            } else {
                false
            };
            priv_.has_camera.set(has_camera);
            obj.notify("has-camera");
        }));
    }

    pub fn has_camera(&self) -> bool {
        let priv_ = imp::QrCodeScanner::from_instance(self);
        priv_.has_camera.get()
    }

    /// Connects the prepared signals to the function f given in input
    pub fn connect_code_detected<F: Fn(&Self, QrVerificationData) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_local("code-detected", true, move |values| {
            let obj = values[0].get::<Self>().unwrap();
            let data = values[1].get::<QrVerificationDataBoxed>().unwrap();

            f(&obj, data.0);

            None
        })
        .unwrap()
    }
}

async fn stream() -> Result<Option<RawFd>, ashpd::Error> {
    let connection = zbus::Connection::session().await?;
    let proxy = camera::CameraProxy::new(&connection).await?;

    if proxy.is_camera_present().await? {
        proxy.access_camera().await?;
        Ok(Some(proxy.open_pipe_wire_remote().await?))
    } else {
        Ok(None)
    }
}

#[derive(Clone, Debug, PartialEq, glib::GBoxed)]
#[gboxed(type_name = "QrVerificationDataBoxed")]
struct QrVerificationDataBoxed(QrVerificationData);
