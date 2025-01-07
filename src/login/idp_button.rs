use gtk::{self, glib, glib::clone, prelude::*, subclass::prelude::*, CompositeTemplate};
use matrix_sdk::ruma::api::client::session::get_login_types::v3::{
    IdentityProvider, IdentityProviderBrand,
};

use crate::gettext_f;

mod imp {
    use std::{cell::OnceCell, marker::PhantomData};

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/login/idp_button.ui")]
    #[properties(wrapper_type = super::IdpButton)]
    pub struct IdpButton {
        /// The identity provider brand of this button.
        brand: OnceCell<IdentityProviderBrand>,
        /// The identity provider brand of this button, as a string.
        #[property(get = Self::brand_string)]
        brand_string: PhantomData<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for IdpButton {
        const NAME: &'static str = "IdpButton";
        type Type = super::IdpButton;
        type ParentType = gtk::Button;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);

            klass.set_accessible_role(gtk::AccessibleRole::Button);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for IdpButton {}

    impl WidgetImpl for IdpButton {}
    impl ButtonImpl for IdpButton {}

    impl IdpButton {
        /// Set the identity provider brand of this button.
        pub(super) fn set_brand(&self, brand: IdentityProviderBrand) {
            let brand = self.brand.get_or_init(|| brand);

            adw::StyleManager::default().connect_dark_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_| imp.update_icon()
            ));
            self.update_icon();

            let obj = self.obj();
            obj.set_action_target_value(Some(&Some(&brand.as_str()).to_variant()));
            obj.set_tooltip_text(Some(&gettext_f(
                // Translators: Do NOT translate the content between '{' and '}', this is a
                // variable name.
                // This is the tooltip text on buttons to log in via Single Sign-On.
                // The brand is something like Facebook, Apple, GitHub…
                "Log in with {brand}",
                &[("brand", brand.as_str())],
            )));
        }

        /// The identity provider brand of this button.
        fn brand(&self) -> &IdentityProviderBrand {
            self.brand.get().expect("brand is initialized")
        }

        /// The identity provider brand of this button, as a string.
        fn brand_string(&self) -> String {
            self.brand().to_string()
        }

        /// The icon name of the current brand, according to the current theme.
        fn brand_icon(&self) -> &str {
            let is_dark = adw::StyleManager::default().is_dark();

            match self.brand() {
                IdentityProviderBrand::Apple => {
                    if is_dark {
                        "idp-apple-dark"
                    } else {
                        "idp-apple"
                    }
                }
                IdentityProviderBrand::Facebook => "idp-facebook",
                IdentityProviderBrand::GitHub => {
                    if is_dark {
                        "idp-github-dark"
                    } else {
                        "idp-github"
                    }
                }
                IdentityProviderBrand::GitLab => "idp-gitlab",
                IdentityProviderBrand::Google => "idp-google",
                IdentityProviderBrand::Twitter => {
                    if is_dark {
                        "idp-x-dark"
                    } else {
                        "idp-x-light"
                    }
                }
                // We do not construct this for other brands.
                _ => unreachable!(),
            }
        }

        /// Update the icon of this button for the current state.
        fn update_icon(&self) {
            self.obj().set_icon_name(self.brand_icon());
        }
    }
}

glib::wrapper! {
    /// A button to represent an SSO identity provider.
    pub struct IdpButton(ObjectSubclass<imp::IdpButton>)
        @extends gtk::Widget, gtk::Button,
        @implements gtk::Accessible, gtk::Actionable;
}

impl IdpButton {
    /// The supported SSO identity provider brands of `IdpButton`.
    const SUPPORTED_IDP_BRANDS: &[IdentityProviderBrand] = &[
        IdentityProviderBrand::Apple,
        IdentityProviderBrand::Facebook,
        IdentityProviderBrand::GitHub,
        IdentityProviderBrand::GitLab,
        IdentityProviderBrand::Google,
        IdentityProviderBrand::Twitter,
    ];

    /// Create a new `IdpButton` with the given identity provider.
    ///
    /// Returns `None` if the identity provider's brand is not supported.
    pub fn new(idp: &IdentityProvider) -> Option<Self> {
        // If this is not a supported brand, return `None`.
        let brand = idp.brand.as_ref()?;
        if !Self::SUPPORTED_IDP_BRANDS.contains(brand) {
            return None;
        }

        let obj = glib::Object::new::<Self>();
        obj.imp().set_brand(brand.clone());

        Some(obj)
    }
}
