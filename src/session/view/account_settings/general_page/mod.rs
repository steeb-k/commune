use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{
    gio,
    glib::{self, clone},
    CompositeTemplate,
};
use tracing::error;

mod change_password_subpage;
mod deactivate_account_subpage;
mod log_out_subpage;

pub use self::{
    change_password_subpage::ChangePasswordSubpage,
    deactivate_account_subpage::DeactivateAccountSubpage, log_out_subpage::LogOutSubpage,
};
use crate::{
    components::{ActionButton, ActionState, ButtonRow, EditableAvatar},
    prelude::*,
    session::model::Session,
    spawn, spawn_tokio, toast,
    utils::{media::load_file, template_callbacks::TemplateCallbacks, OngoingAsyncAction},
};

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/account_settings/general_page/mod.ui"
    )]
    #[properties(wrapper_type = super::GeneralPage)]
    pub struct GeneralPage {
        /// The current session.
        #[property(get, set = Self::set_session, nullable)]
        pub session: glib::WeakRef<Session>,
        #[template_child]
        pub avatar: TemplateChild<EditableAvatar>,
        #[template_child]
        pub display_name: TemplateChild<adw::EntryRow>,
        #[template_child]
        pub display_name_button: TemplateChild<ActionButton>,
        #[template_child]
        pub change_password_group: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        pub homeserver: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub user_id: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub session_id: TemplateChild<adw::ActionRow>,
        pub changing_avatar: RefCell<Option<OngoingAsyncAction<String>>>,
        pub changing_display_name: RefCell<Option<OngoingAsyncAction<String>>>,
        pub avatar_uri_handler: RefCell<Option<glib::SignalHandlerId>>,
        pub display_name_handler: RefCell<Option<glib::SignalHandlerId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for GeneralPage {
        const NAME: &'static str = "AccountSettingsGeneralPage";
        type Type = super::GeneralPage;
        type ParentType = adw::PreferencesPage;

        fn class_init(klass: &mut Self::Class) {
            ButtonRow::static_type();

            Self::bind_template(klass);
            Self::Type::bind_template_callbacks(klass);
            TemplateCallbacks::bind_template_callbacks(klass);

            klass.install_action("account-user.copy-homeserver", None, |obj, _, _| {
                let text = obj.imp().homeserver.subtitle().unwrap_or_default();
                obj.clipboard().set_text(&text);
                toast!(obj, gettext("Homeserver address copied to clipboard"));
            });

            klass.install_action("account-user.copy-user-id", None, |obj, _, _| {
                let text = obj.imp().user_id.subtitle().unwrap_or_default();
                obj.clipboard().set_text(&text);
                toast!(obj, gettext("Matrix user ID copied to clipboard"));
            });

            klass.install_action("account-user.copy-session-id", None, |obj, _, _| {
                let text = obj.imp().session_id.subtitle().unwrap_or_default();
                obj.clipboard().set_text(&text);
                toast!(obj, gettext("Session ID copied to clipboard"));
            });
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for GeneralPage {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();

            obj.init_avatar();
            obj.init_display_name();
            obj.init_change_password();
        }
    }

    impl WidgetImpl for GeneralPage {}
    impl PreferencesPageImpl for GeneralPage {}

    impl GeneralPage {
        /// Set the current session.
        fn set_session(&self, session: Option<Session>) {
            let prev_session = self.session.upgrade();
            if prev_session == session {
                return;
            }
            let obj = self.obj();

            if let Some(session) = prev_session {
                let user = session.user();

                if let Some(handler) = self.avatar_uri_handler.take() {
                    user.avatar_data().image().unwrap().disconnect(handler)
                }
                if let Some(handler) = self.display_name_handler.take() {
                    user.disconnect(handler);
                }
            }

            self.session.set(session.as_ref());
            obj.notify_session();

            let Some(session) = session else {
                return;
            };

            self.user_id.set_subtitle(session.user_id().as_str());
            self.homeserver.set_subtitle(session.homeserver().as_str());
            self.session_id.set_subtitle(session.device_id().as_str());

            let user = session.user();
            let avatar_uri_handler = user.avatar_data().image().unwrap().connect_uri_notify(
                clone!(@weak obj => move |avatar_image| {
                    obj.avatar_changed(avatar_image.uri());
                }),
            );
            self.avatar_uri_handler.replace(Some(avatar_uri_handler));

            let display_name_handler =
                user.connect_display_name_notify(clone!(@weak obj => move |user| {
                    obj.display_name_changed(user.display_name());
                }));
            self.display_name_handler
                .replace(Some(display_name_handler));
        }
    }
}

glib::wrapper! {
    /// Account settings page about the user and the session.
    pub struct GeneralPage(ObjectSubclass<imp::GeneralPage>)
        @extends gtk::Widget, adw::PreferencesPage, @implements gtk::Accessible;
}

#[gtk::template_callbacks]
impl GeneralPage {
    pub fn new(session: &Session) -> Self {
        glib::Object::builder().property("session", session).build()
    }

    fn init_avatar(&self) {
        let avatar = &self.imp().avatar;
        avatar.connect_edit_avatar(clone!(@weak self as obj => move |_, file| {
            spawn!(
                clone!(@weak obj => async move {
                    obj.change_avatar(file).await;
                })
            );
        }));
        avatar.connect_remove_avatar(clone!(@weak self as obj => move |_| {
            spawn!(
                clone!(@weak obj => async move {
                    obj.remove_avatar().await;
                })
            );
        }));
    }

    fn avatar_changed(&self, uri: Option<String>) {
        let imp = self.imp();

        if let Some(action) = imp.changing_avatar.borrow().as_ref() {
            if uri.as_ref() != action.as_value() {
                // This is not the change we expected, maybe another device did a change too.
                // Let's wait for another change.
                return;
            }
        } else {
            // No action is ongoing, we don't need to do anything.
            return;
        };

        // Reset the state.
        imp.changing_avatar.take();
        imp.avatar.success();
        if uri.is_none() {
            toast!(self, gettext("Avatar removed successfully"));
        } else {
            toast!(self, gettext("Avatar changed successfully"));
        }
    }

    async fn change_avatar(&self, file: gio::File) {
        let Some(session) = self.session() else {
            return;
        };

        let imp = self.imp();
        let avatar = &imp.avatar;
        avatar.edit_in_progress();

        let (data, info) = match load_file(&file).await {
            Ok(res) => res,
            Err(error) => {
                error!("Could not load user avatar file: {error}");
                toast!(self, gettext("Could not load file"));
                avatar.reset();
                return;
            }
        };

        let client = session.client();
        let client_clone = client.clone();
        let handle =
            spawn_tokio!(async move { client_clone.media().upload(&info.mime, data).await });

        let uri = match handle.await.unwrap() {
            Ok(res) => res.content_uri,
            Err(error) => {
                error!("Could not upload user avatar: {error}");
                toast!(self, gettext("Could not upload avatar"));
                avatar.reset();
                return;
            }
        };

        let (action, weak_action) = OngoingAsyncAction::set(uri.to_string());
        imp.changing_avatar.replace(Some(action));

        let uri_clone = uri.clone();
        let handle =
            spawn_tokio!(async move { client.account().set_avatar_url(Some(&uri_clone)).await });

        match handle.await.unwrap() {
            Ok(_) => {
                // If the user is in no rooms, we won't receive the update via sync, so change
                // the avatar manually if this request succeeds before the avatar is updated.
                // Because this action can finish in avatar_changed, we must only act if this is
                // still the current action.
                if weak_action.is_ongoing() {
                    session.user().set_avatar_url(Some(uri))
                }
            }
            Err(error) => {
                // Because this action can finish in avatar_changed, we must only act if this is
                // still the current action.
                if weak_action.is_ongoing() {
                    imp.changing_avatar.take();
                    error!("Could not change user avatar: {error}");
                    toast!(self, gettext("Could not change avatar"));
                    avatar.reset();
                }
            }
        }
    }

    async fn remove_avatar(&self) {
        let Some(session) = self.session() else {
            return;
        };

        // Ask for confirmation.
        let confirm_dialog = adw::AlertDialog::builder()
            .default_response("cancel")
            .heading(gettext("Remove Avatar?"))
            .body(gettext("Do you really want to remove your avatar?"))
            .build();
        confirm_dialog.add_responses(&[
            ("cancel", &gettext("Cancel")),
            ("remove", &gettext("Remove")),
        ]);
        confirm_dialog.set_response_appearance("remove", adw::ResponseAppearance::Destructive);

        if confirm_dialog.choose_future(self).await != "remove" {
            return;
        }

        let imp = self.imp();
        let avatar = &*imp.avatar;
        avatar.removal_in_progress();

        let (action, weak_action) = OngoingAsyncAction::remove();
        imp.changing_avatar.replace(Some(action));

        let client = session.client();
        let handle = spawn_tokio!(async move { client.account().set_avatar_url(None).await });

        match handle.await.unwrap() {
            Ok(_) => {
                // If the user is in no rooms, we won't receive the update via sync, so change
                // the avatar manually if this request succeeds before the avatar is updated.
                // Because this action can finish in avatar_changed, we must only act if this is
                // still the current action.
                if weak_action.is_ongoing() {
                    session.user().set_avatar_url(None)
                }
            }
            Err(error) => {
                // Because this action can finish in avatar_changed, we must only act if this is
                // still the current action.
                if weak_action.is_ongoing() {
                    imp.changing_avatar.take();
                    error!("Couldn’t remove user avatar: {error}");
                    toast!(self, gettext("Could not remove avatar"));
                    avatar.reset();
                }
            }
        }
    }

    fn init_display_name(&self) {
        let imp = self.imp();
        let entry = &imp.display_name;
        entry.connect_changed(clone!(@weak self as obj => move |entry| {
            let Some(session) = obj.session() else {
                return;
            };

            obj.imp().display_name_button.set_visible(entry.text() != session.user().display_name());
        }));
    }

    fn display_name_changed(&self, name: String) {
        let imp = self.imp();

        if let Some(action) = imp.changing_display_name.borrow().as_ref() {
            if action.as_value() == Some(&name) {
                // This is not the change we expected, maybe another device did a change too.
                // Let's wait for another change.
                return;
            }
        } else {
            // No action is ongoing, we don't need to do anything.
            return;
        }

        // Reset state.
        imp.changing_display_name.take();

        let entry = &imp.display_name;
        let button = &imp.display_name_button;

        entry.remove_css_class("error");
        entry.set_sensitive(true);
        button.set_visible(false);
        button.set_state(ActionState::Confirm);
        toast!(self, gettext("Name changed successfully"));
    }

    #[template_callback]
    fn change_display_name(&self) {
        spawn!(clone!(@weak self as obj => async move {
            obj.change_display_name_inner().await;
        }));
    }

    async fn change_display_name_inner(&self) {
        let Some(session) = self.session() else {
            return;
        };

        let imp = self.imp();
        let entry = &imp.display_name;
        let button = &imp.display_name_button;

        entry.set_sensitive(false);
        button.set_state(ActionState::Loading);

        let display_name = entry.text().trim().to_string();

        let (action, weak_action) = OngoingAsyncAction::set(display_name.clone());
        imp.changing_display_name.replace(Some(action));

        let client = session.client();
        let display_name_clone = display_name.clone();
        let handle = spawn_tokio!(async move {
            client
                .account()
                .set_display_name(Some(&display_name_clone))
                .await
        });

        match handle.await.unwrap() {
            Ok(_) => {
                // If the user is in no rooms, we won't receive the update via sync, so change
                // the avatar manually if this request succeeds before the avatar is updated.
                // Because this action can finish in display_name_changed, we must only act if
                // this is still the current action.
                if weak_action.is_ongoing() {
                    session.user().set_name(Some(display_name));
                }
            }
            Err(error) => {
                // Because this action can finish in display_name_changed, we must only act if
                // this is still the current action.
                if weak_action.is_ongoing() {
                    imp.changing_display_name.take();
                    error!("Could not change user display name: {error}");
                    toast!(self, gettext("Could not change display name"));
                    button.set_state(ActionState::Retry);
                    entry.add_css_class("error");
                    entry.set_sensitive(true);
                }
            }
        }
    }

    fn init_change_password(&self) {
        let Some(session) = self.session() else {
            return;
        };
        let client = session.client();

        spawn!(
            glib::Priority::LOW,
            clone!(@weak self as obj => async move {
                // Check whether the user can change their password.
                let handle = spawn_tokio!(async move {
                    client.get_capabilities().await
                });
                match handle.await.unwrap() {
                    Ok(capabilities) => {
                        obj.imp().change_password_group.set_visible(capabilities.change_password.enabled);
                    }
                    Err(error) => error!("Could not get server capabilities: {error}"),
                }
            })
        );
    }
}
