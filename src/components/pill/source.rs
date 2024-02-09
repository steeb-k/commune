use gtk::{glib, prelude::*, subclass::prelude::*};

use super::Pill;
use crate::components::AvatarData;

mod imp {
    use std::marker::PhantomData;

    use super::*;

    #[repr(C)]
    pub struct PillSourceClass {
        pub parent_class: glib::object::ObjectClass,
        pub identifier: fn(&super::PillSource) -> String,
    }

    unsafe impl ClassStruct for PillSourceClass {
        type Type = PillSource;
    }

    pub(super) fn pill_source_identifier(this: &super::PillSource) -> String {
        let klass = this.class();
        (klass.as_ref().identifier)(this)
    }

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::PillSource)]
    pub struct PillSource {
        /// A unique identifier for this source.
        #[property(get = Self::identifier)]
        pub identifier: PhantomData<String>,
        /// The display name of this source.
        #[property(get = Self::display_name, set = Self::set_display_name, explicit_notify)]
        pub display_name: PhantomData<String>,
        /// The avatar data of this source.
        #[property(get)]
        pub avatar_data: AvatarData,
    }

    #[glib::object_subclass]
    unsafe impl ObjectSubclass for PillSource {
        const NAME: &'static str = "PillSource";
        const ABSTRACT: bool = true;
        type Type = super::PillSource;
        type Class = PillSourceClass;
    }

    #[glib::derived_properties]
    impl ObjectImpl for PillSource {}

    impl PillSource {
        /// A unique identifier for this source.
        fn identifier(&self) -> String {
            imp::pill_source_identifier(&self.obj())
        }

        /// The display name of this source.
        fn display_name(&self) -> String {
            self.avatar_data.display_name()
        }

        /// Set the display name of this source.
        fn set_display_name(&self, display_name: String) {
            if self.display_name() == display_name {
                return;
            }

            self.avatar_data.set_display_name(display_name);
            self.obj().notify_display_name();
        }
    }
}

glib::wrapper! {
    /// Parent class of objects that can be represented as a `Pill`.
    pub struct PillSource(ObjectSubclass<imp::PillSource>);
}

/// Public trait containing implemented methods for everything that derives from
/// `PillSource`.
///
/// To override the behavior of these methods, override the corresponding method
/// of `PillSourceImpl`.
pub trait PillSourceExt: 'static {
    /// A unique identifier for this source.
    #[allow(dead_code)]
    fn identifier(&self) -> String;

    /// The display name of this source.
    fn display_name(&self) -> String;

    /// Set the display name of this source.
    fn set_display_name(&self, display_name: String);

    /// The avatar data of this source.
    fn avatar_data(&self) -> AvatarData;

    /// Connect to the signal emitted when the display name changes.
    fn connect_display_name_notify<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId;

    /// Get a `Pill` representing this source.
    fn to_pill(&self) -> Pill;
}

impl<O: IsA<PillSource>> PillSourceExt for O {
    /// A unique identifier for this source.
    fn identifier(&self) -> String {
        self.upcast_ref().identifier()
    }

    /// The display name of this source.
    fn display_name(&self) -> String {
        self.upcast_ref().display_name()
    }

    /// Set the display name of this source.
    fn set_display_name(&self, display_name: String) {
        self.upcast_ref().set_display_name(display_name);
    }

    /// The avatar data of this source.
    fn avatar_data(&self) -> AvatarData {
        self.upcast_ref().avatar_data()
    }

    /// Connect to the signal emitted when the display name changes.
    fn connect_display_name_notify<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.upcast_ref()
            .connect_display_name_notify(move |source| f(source.downcast_ref().unwrap()))
    }

    /// Get a `Pill` representing this source.
    fn to_pill(&self) -> Pill {
        Pill::new(self)
    }
}

/// Public trait that must be implemented for everything that derives from
/// `PillSource`.
///
/// Overriding a method from this Trait overrides also its behavior in
/// `PillSourceExt`.
pub trait PillSourceImpl: ObjectImpl {
    fn identifier(&self) -> String;
}

// Make `PillSource` subclassable.
unsafe impl<T> IsSubclassable<T> for PillSource
where
    T: PillSourceImpl,
    T::Type: IsA<PillSource>,
{
    fn class_init(class: &mut glib::Class<Self>) {
        Self::parent_class_init::<T>(class.upcast_ref_mut());

        let klass = class.as_mut();

        klass.identifier = identifier_trampoline::<T>;
    }
}

// Virtual method implementation trampolines.
fn identifier_trampoline<T>(this: &PillSource) -> String
where
    T: ObjectSubclass + PillSourceImpl,
    T::Type: IsA<PillSource>,
{
    let this = this.downcast_ref::<T::Type>().unwrap();
    this.imp().identifier()
}
