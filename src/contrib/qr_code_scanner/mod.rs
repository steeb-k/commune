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
    use tokio::sync::OnceCell;

    use super::*;

    #[derive(Debug, CompositeTemplate, Default)]
    #[template(resource = "/org/gnome/FractalNext/qr-code-scanner.ui")]
    pub struct QrCodeScanner {
        pub paintable: CameraPaintable,
        #[template_child]
        pub picture: TemplateChild<gtk::Picture>,
        pub connection: OnceCell<zbus::Connection>,
        pub has_camera: Cell<bool>,
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

    async fn connection(&self) -> Result<&zbus::Connection, ashpd::Error> {
        let priv_ = imp::QrCodeScanner::from_instance(self);

        Ok(priv_
            .connection
            .get_or_try_init(|| zbus::Connection::session())
            .await?)
    }

    pub fn stop(&self) {
        let self_ = imp::QrCodeScanner::from_instance(self);

        self_.paintable.close_pipeline();
    }

    pub async fn start(&self) -> bool {
        let priv_ = imp::QrCodeScanner::from_instance(self);
        if let Ok(stream_fd) = self.stream().await {
            if let Ok(node_id) = camera::pipewire_node_id(stream_fd).await {
                priv_.paintable.set_pipewire_fd(stream_fd, node_id);
                self.set_has_camera(true);
                return true;
            }
        }

        self.set_has_camera(false);
        return false;
    }

    async fn has_camera_internal(&self) -> Result<bool, ashpd::Error> {
        let proxy = camera::CameraProxy::new(self.connection().await?).await?;

        proxy.is_camera_present().await
    }

    async fn stream(&self) -> Result<RawFd, ashpd::Error> {
        let proxy = camera::CameraProxy::new(self.connection().await?).await?;

        proxy.access_camera().await?;
        proxy.open_pipe_wire_remote().await
    }

    fn init_has_camera(&self) {
        spawn!(clone!(@weak self as obj => async move {
            obj.set_has_camera(obj.has_camera_internal().await.unwrap_or_default());
        }));
    }

    pub fn has_camera(&self) -> bool {
        let priv_ = imp::QrCodeScanner::from_instance(self);
        priv_.has_camera.get()
    }

    fn set_has_camera(&self, has_camera: bool) {
        let priv_ = imp::QrCodeScanner::from_instance(self);

        if has_camera == self.has_camera() {
            return;
        }

        priv_.has_camera.set(has_camera);
        self.notify("has-camera");
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

#[derive(Clone, Debug, PartialEq, glib::GBoxed)]
#[gboxed(type_name = "QrVerificationDataBoxed")]
struct QrVerificationDataBoxed(QrVerificationData);
