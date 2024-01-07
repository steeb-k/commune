/// Subclassable camera paintable.
use gtk::{gdk, glib, glib::closure_local, prelude::*, subclass::prelude::*};
use matrix_sdk::encryption::verification::QrVerificationData;

#[cfg(target_os = "linux")]
pub mod linux;

use crate::contrib::qr_code_scanner::QrVerificationDataBoxed;

pub enum Action {
    QrCodeDetected(QrVerificationData),
}

mod imp {
    use glib::subclass::Signal;
    use once_cell::sync::Lazy;

    use super::*;

    #[repr(C)]
    pub struct CameraPaintableClass {
        pub parent_class: glib::object::Class<glib::Object>,
    }

    unsafe impl ClassStruct for CameraPaintableClass {
        type Type = CameraPaintable;
    }

    #[derive(Debug, Default)]
    pub struct CameraPaintable;

    #[glib::object_subclass]
    impl ObjectSubclass for CameraPaintable {
        const NAME: &'static str = "CameraPaintable";
        type Type = super::CameraPaintable;
        type Class = CameraPaintableClass;
        type Interfaces = (gdk::Paintable,);
    }

    impl ObjectImpl for CameraPaintable {
        fn signals() -> &'static [Signal] {
            static SIGNALS: Lazy<Vec<Signal>> = Lazy::new(|| {
                vec![Signal::builder("code-detected")
                    .param_types([QrVerificationDataBoxed::static_type()])
                    .run_first()
                    .build()]
            });
            SIGNALS.as_ref()
        }
    }

    impl PaintableImpl for CameraPaintable {
        fn snapshot(&self, _snapshot: &gdk::Snapshot, _width: f64, _height: f64) {
            // Nothing to do
        }
    }
}

glib::wrapper! {
    /// A subclassable paintable to display the output of a camera.
    pub struct CameraPaintable(ObjectSubclass<imp::CameraPaintable>)
        @implements gdk::Paintable;
}

pub trait CameraPaintableExt: 'static {
    /// Connect to the signal emitted when a code is detected.
    fn connect_code_detected<F: Fn(&Self, QrVerificationData) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId;
}

impl<O: IsA<CameraPaintable>> CameraPaintableExt for O {
    fn connect_code_detected<F: Fn(&Self, QrVerificationData) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_closure(
            "activated",
            true,
            closure_local!(move |obj: Self, data: QrVerificationDataBoxed| {
                f(&obj, data.0);
            }),
        )
    }
}

/// Public trait that must be implemented for everything that derives from
/// `CameraPaintable`.
///
/// Overriding a method from this Trait overrides also its behavior in
/// `CameraPaintableExt`.
#[allow(async_fn_in_trait)]
pub trait CameraPaintableImpl: ObjectImpl + PaintableImpl {}

unsafe impl<T> IsSubclassable<T> for CameraPaintable
where
    T: CameraPaintableImpl,
    T::Type: IsA<CameraPaintable>,
{
}
