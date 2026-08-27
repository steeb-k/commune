use std::{collections::BTreeMap, fmt::Debug};

use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{glib, glib::clone};
use ruma::api::client::uiaa::{PolicyDefinition, PolicyTranslation};
use tracing::error;

use crate::components::LoadingButton;

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/org/gnome/Fractal/ui/components/dialogs/auth/terms_page.ui")]
    pub struct AuthDialogTermsPage {
        #[template_child]
        policies: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub(super) confirm_button: TemplateChild<LoadingButton>,
        /// The check button of each policy the server sent.
        check_buttons: RefCell<Vec<gtk::CheckButton>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for AuthDialogTermsPage {
        const NAME: &'static str = "AuthDialogTermsPage";
        type Type = super::AuthDialogTermsPage;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            LoadingButton::ensure_type();

            Self::bind_template(klass);
            Self::bind_template_callbacks(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for AuthDialogTermsPage {}
    impl WidgetImpl for AuthDialogTermsPage {}
    impl BinImpl for AuthDialogTermsPage {}

    #[gtk::template_callbacks]
    impl AuthDialogTermsPage {
        /// Add a row for each of the given policies.
        pub(super) fn set_policies(&self, policies: &BTreeMap<String, PolicyDefinition>) {
            for policy in policies.values() {
                let Some(translation) = preferred_translation(policy) else {
                    // A policy with no translation at all cannot be presented,
                    // and agreeing to a document nobody can read is worse than
                    // skipping the row.
                    error!("Ignoring a policy document with no translation");
                    continue;
                };

                let check_button = gtk::CheckButton::builder()
                    .valign(gtk::Align::Center)
                    .build();
                check_button.connect_toggled(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.update_confirm();
                    }
                ));

                let link = gtk::Button::builder()
                    .icon_name("external-link-symbolic")
                    .valign(gtk::Align::Center)
                    .tooltip_text(gettext("Read Document"))
                    .css_classes(["flat"])
                    .build();
                link.update_property(&[gtk::accessible::Property::Label(&gettext(
                    "Read Document",
                ))]);

                let url = translation.url.clone();
                link.connect_clicked(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        let url = url.clone();
                        glib::spawn_future_local(async move {
                            if let Err(error) = gtk::UriLauncher::new(&url)
                                .launch_future(imp.obj().root().and_downcast_ref::<gtk::Window>())
                                .await
                            {
                                error!("Could not launch policy document URI: {error}");
                            }
                        });
                    }
                ));

                let row = adw::ActionRow::builder()
                    .title(glib::markup_escape_text(&translation.name))
                    .subtitle(glib::markup_escape_text(&policy.version))
                    .activatable_widget(&check_button)
                    .build();
                row.add_prefix(&check_button);
                row.add_suffix(&link);

                self.policies.append(&row);
                self.check_buttons.borrow_mut().push(check_button);
            }

            self.update_confirm();
        }

        /// Whether the user can proceed given the current state.
        fn can_proceed(&self) -> bool {
            let check_buttons = self.check_buttons.borrow();

            // A server that sends a terms stage with no policy in it has asked
            // for nothing, so there is nothing to agree to and no reason to
            // block.
            check_buttons.iter().all(gtk::CheckButton::is_active)
        }

        /// Update the confirm button for the current state.
        fn update_confirm(&self) {
            self.confirm_button.set_sensitive(self.can_proceed());
        }

        /// Proceed to authentication, having agreed.
        #[template_callback]
        fn proceed(&self) {
            if !self.can_proceed() {
                return;
            }

            self.confirm_button.set_is_loading(true);
            let _ = self.obj().activate_action("auth-dialog.continue", None);
        }

        /// Retry this stage.
        pub(super) fn retry(&self) {
            self.confirm_button.set_is_loading(false);
            self.update_confirm();
        }
    }
}

glib::wrapper! {
    /// Page to pass the terms and conditions stage for the [`AuthDialog`].
    pub struct AuthDialogTermsPage(ObjectSubclass<imp::AuthDialogTermsPage>)
        @extends gtk::Widget, adw::Bin,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl AuthDialogTermsPage {
    pub fn new(policies: &BTreeMap<String, PolicyDefinition>) -> Self {
        let obj: Self = glib::Object::new();
        obj.imp().set_policies(policies);
        obj
    }

    /// Get the default widget of this page.
    pub fn default_widget(&self) -> &gtk::Widget {
        self.imp().confirm_button.upcast_ref()
    }

    /// Retry this stage.
    pub fn retry(&self) {
        self.imp().retry();
    }
}

/// The translation of the given policy that best matches the languages this
/// session asked for.
///
/// The spec says language codes *should* follow RFC 5646 and notes that some
/// implementations write `en_US` where it says `en-US`, so both spellings are
/// treated as the same tag.
fn preferred_translation(policy: &PolicyDefinition) -> Option<&PolicyTranslation> {
    let normalized = |tag: &str| tag.replace('_', "-").to_lowercase();

    let translations = policy
        .translations
        .iter()
        .map(|(tag, translation)| (normalized(tag), translation))
        .collect::<Vec<_>>();

    let find = |wanted: &str| {
        translations
            .iter()
            .find(|(tag, _)| tag == wanted)
            .map(|(_, translation)| *translation)
    };

    for language in glib::language_names() {
        let wanted = normalized(&language);

        if let Some(translation) = find(&wanted) {
            return Some(translation);
        }

        // `en_GB.UTF-8` and `en-GB` should both find an `en` document.
        if let Some((prefix, _)) = wanted.split_once(['-', '.'])
            && let Some(translation) = find(prefix)
        {
            return Some(translation);
        }
    }

    // English is the language the spec's own example uses, and the one a
    // homeserver is most likely to have. Failing that, anything is better than
    // an empty row.
    find("en").or_else(|| policy.translations.values().next())
}
