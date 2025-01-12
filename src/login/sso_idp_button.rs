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
    #[template(resource = "/org/gnome/Fractal/ui/login/sso_idp_button.ui")]
    #[properties(wrapper_type = super::SsoIdpButton)]
    pub struct SsoIdpButton {
        /// The identity provider of this button.
        identity_provider: OnceCell<IdentityProvider>,
        /// The ID of the identity provider.
        #[property(get = Self::id)]
        id: PhantomData<String>,
        /// The name of the identity provider.
        #[property(get = Self::name)]
        name: PhantomData<String>,
        /// The brand of the identity provider, as a string.
        #[property(get = Self::brand_string)]
        brand_string: PhantomData<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SsoIdpButton {
        const NAME: &'static str = "SsoIdpButton";
        type Type = super::SsoIdpButton;
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
    impl ObjectImpl for SsoIdpButton {}

    impl WidgetImpl for SsoIdpButton {}
    impl ButtonImpl for SsoIdpButton {}

    impl SsoIdpButton {
        /// Set the identity provider of this button.
        pub(super) fn set_identity_provider(&self, identity_provider: IdentityProvider) {
            let identity_provider = self.identity_provider.get_or_init(|| identity_provider);

            adw::StyleManager::default().connect_dark_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_| imp.update_icon()
            ));
            self.update_icon();

            self.obj()
                .set_action_target_value(Some(&Some(&identity_provider.id).to_variant()));
            self.obj().set_tooltip_text(Some(&gettext_f(
                // Translators: Do NOT translate the content between '{' and '}', this is a
                // variable name.
                // This is the tooltip text on buttons to log in via Single Sign-On.
                // The brand is something like Facebook, Apple, GitHub…
                "Log in with {brand}",
                &[("brand", &identity_provider.name)],
            )));
        }

        /// The identity provider of this button.
        fn identity_provider(&self) -> &IdentityProvider {
            self.identity_provider
                .get()
                .expect("identity provider is initialized")
        }

        /// The ID of the identity provider.
        fn id(&self) -> String {
            self.identity_provider().id.clone()
        }

        /// The name of the identity provider.
        fn name(&self) -> String {
            self.identity_provider().name.clone()
        }

        /// The brand of the identity provider.
        fn brand(&self) -> &IdentityProviderBrand {
            self.identity_provider()
                .brand
                .as_ref()
                .expect("identity provider has a brand")
        }

        /// The brand of the identity provider, as a string.
        fn brand_string(&self) -> String {
            self.brand().to_string()
        }

        /// The icon name of the brand, according to the current theme.
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
    pub struct SsoIdpButton(ObjectSubclass<imp::SsoIdpButton>)
        @extends gtk::Widget, gtk::Button,
        @implements gtk::Accessible, gtk::Actionable;
}

impl SsoIdpButton {
    /// The supported SSO identity provider brands of `SsoIdpButton`.
    const SUPPORTED_IDP_BRANDS: &[IdentityProviderBrand] = &[
        IdentityProviderBrand::Apple,
        IdentityProviderBrand::Facebook,
        IdentityProviderBrand::GitHub,
        IdentityProviderBrand::GitLab,
        IdentityProviderBrand::Google,
        IdentityProviderBrand::Twitter,
    ];

    /// Create a new `SsoIdpButton` with the given identity provider.
    ///
    /// Returns `None` if the identity provider's brand is not supported.
    pub fn new(identity_provider: IdentityProvider) -> Option<Self> {
        // If this is not a supported brand, return `None`.
        let brand = identity_provider.brand.as_ref()?;
        if !Self::SUPPORTED_IDP_BRANDS.contains(brand) {
            return None;
        }

        let obj = glib::Object::new::<Self>();
        obj.imp().set_identity_provider(identity_provider);

        Some(obj)
    }
}
