use gtk::{self, glib, glib::clone, prelude::*, subclass::prelude::*, CompositeTemplate};
use matrix_sdk::ruma::api::client::session::get_login_types::v3::{
    IdentityProvider, IdentityProviderBrand,
};

use crate::gettext_f;

#[derive(Debug, Default, Hash, Eq, PartialEq, Clone, Copy, glib::Enum, strum::Display)]
#[repr(i32)]
#[enum_type(name = "IdpBrand")]
pub enum IdpBrand {
    #[default]
    Apple = 0,
    Facebook = 1,
    GitHub = 2,
    GitLab = 3,
    Google = 4,
    Twitter = 5,
}

impl IdpBrand {
    /// Get the icon name of this brand, according to the current theme.
    pub fn icon(&self) -> &'static str {
        let dark = adw::StyleManager::default().is_dark();
        match self {
            IdpBrand::Apple => {
                if dark {
                    "idp-apple-dark"
                } else {
                    "idp-apple"
                }
            }
            IdpBrand::Facebook => "idp-facebook",
            IdpBrand::GitHub => {
                if dark {
                    "idp-github-dark"
                } else {
                    "idp-github"
                }
            }
            IdpBrand::GitLab => "idp-gitlab",
            IdpBrand::Google => "idp-google",
            IdpBrand::Twitter => "idp-twitter",
        }
    }
}

impl TryFrom<&IdentityProviderBrand> for IdpBrand {
    type Error = ();

    fn try_from(item: &IdentityProviderBrand) -> Result<Self, Self::Error> {
        match item {
            IdentityProviderBrand::Apple => Ok(IdpBrand::Apple),
            IdentityProviderBrand::Facebook => Ok(IdpBrand::Facebook),
            IdentityProviderBrand::GitHub => Ok(IdpBrand::GitHub),
            IdentityProviderBrand::GitLab => Ok(IdpBrand::GitLab),
            IdentityProviderBrand::Google => Ok(IdpBrand::Google),
            IdentityProviderBrand::Twitter => Ok(IdpBrand::Twitter),
            _ => Err(()),
        }
    }
}

mod imp {
    use std::cell::{Cell, OnceCell};

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/login/idp_button.ui")]
    #[properties(wrapper_type = super::IdpButton)]
    pub struct IdpButton {
        /// The brand of this button.
        #[property(get, construct_only, builder(IdpBrand::default()))]
        pub brand: Cell<IdpBrand>,
        /// The ID of the identity provider of this button.
        #[property(get, set = Self::set_id, construct_only)]
        pub id: OnceCell<String>,
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
    impl ObjectImpl for IdpButton {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();

            adw::StyleManager::default()
                .connect_dark_notify(clone!(@weak obj => move |_| obj.update_icon()));
            obj.update_icon();

            obj.set_tooltip_text(Some(&gettext_f(
                // Translators: Do NOT translate the content between '{' and '}', this is a
                // variable name.
                // This is the tooltip text on buttons to log in via Single Sign-On.
                // The brand is something like Facebook, Apple, GitHub…
                "Log in with {brand}",
                &[("brand", &self.brand.get().to_string())],
            )))
        }
    }

    impl WidgetImpl for IdpButton {}
    impl ButtonImpl for IdpButton {}

    impl IdpButton {
        /// Set the id of the identity-provider represented by this button.
        fn set_id(&self, id: String) {
            self.obj()
                .set_action_target_value(Some(&Some(&id).to_variant()));
            self.id.set(id).unwrap();
        }
    }
}

glib::wrapper! {
    pub struct IdpButton(ObjectSubclass<imp::IdpButton>)
        @extends gtk::Widget, gtk::Button,
        @implements gtk::Accessible, gtk::Actionable;
}

impl IdpButton {
    pub fn new_from_identity_provider(idp: &IdentityProvider) -> Option<Self> {
        let gidp: IdpBrand = idp.brand.as_ref()?.try_into().ok()?;

        Some(
            glib::Object::builder()
                .property("brand", gidp)
                .property("id", &idp.id)
                .build(),
        )
    }

    pub fn update_icon(&self) {
        self.set_icon_name(self.brand().icon());
    }
}
