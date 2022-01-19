use gettextrs::gettext;
use gtk::{gio, glib, glib::clone, prelude::*};
use log::error;
use matrix_sdk::ruma::events::{room::message::MessageType, AnyMessageEventContent};
use once_cell::sync::Lazy;

use crate::{
    session::{event_source_dialog::EventSourceDialog, room::Event},
    spawn,
    utils::cache_dir,
    Error, UserFacingError, Window,
};

// This is only save because the trait `EventActions` can
// only be implemented on `gtk::Widgets` that run only on the main thread
struct MenuModelSendSync(gio::MenuModel);
#[allow(clippy::non_send_fields_in_send_ty)]
unsafe impl Send for MenuModelSendSync {}
unsafe impl Sync for MenuModelSendSync {}

pub trait EventActions
where
    Self: IsA<gtk::Widget>,
    Self: glib::clone::Downgrade,
    <Self as glib::clone::Downgrade>::Weak: glib::clone::Upgrade<Strong = Self>,
{
    /// The `MenuModel` for common message event actions.
    fn event_message_menu_model() -> &'static gio::MenuModel {
        static MODEL: Lazy<MenuModelSendSync> = Lazy::new(|| {
            MenuModelSendSync(
                gtk::Builder::from_resource("/org/gnome/FractalNext/event-menu.ui")
                    .object::<gio::MenuModel>("message_menu_model")
                    .unwrap(),
            )
        });
        &MODEL.0
    }

    /// The `MenuModel` for common media message event actions.
    fn event_media_menu_model() -> &'static gio::MenuModel {
        static MODEL: Lazy<MenuModelSendSync> = Lazy::new(|| {
            MenuModelSendSync(
                gtk::Builder::from_resource("/org/gnome/FractalNext/event-menu.ui")
                    .object::<gio::MenuModel>("media_menu_model")
                    .unwrap(),
            )
        });
        &MODEL.0
    }

    /// The default `MenuModel` for common state event actions.
    fn event_state_menu_model() -> &'static gio::MenuModel {
        static MODEL: Lazy<MenuModelSendSync> = Lazy::new(|| {
            MenuModelSendSync(
                gtk::Builder::from_resource("/org/gnome/FractalNext/event-menu.ui")
                    .object::<gio::MenuModel>("state_menu_model")
                    .unwrap(),
            )
        });
        &MODEL.0
    }

    /// Set the actions available on `self` for `event`.
    ///
    /// Unsets the actions if `event` is `None`.
    ///
    /// Should be paired with the `EventActions` menu models.
    fn set_event_actions(&self, event: Option<&Event>) -> Option<gio::SimpleActionGroup> {
        if event.is_none() {
            self.insert_action_group("event", gio::ActionGroup::NONE);
            return None;
        }

        let event = event.unwrap();
        let action_group = gio::SimpleActionGroup::new();

        // View Event Source
        let view_source = gio::SimpleAction::new("view-source", None);
        view_source.connect_activate(clone!(@weak self as widget, @weak event => move |_, _| {
            let window = widget.root().unwrap().downcast().unwrap();
            let dialog = EventSourceDialog::new(&window, &event);
            dialog.show();
        }));
        action_group.add_action(&view_source);

        if let Some(AnyMessageEventContent::RoomMessage(message)) = event.message_content() {
            // Send/redact a reaction
            let toggle_reaction =
                gio::SimpleAction::new("toggle-reaction", Some(&String::static_variant_type()));
            toggle_reaction.connect_activate(clone!(@weak event => move |_, variant| {
                let key: String = variant.unwrap().get().unwrap();
                let room = event.room();

                let reaction_group = event.reactions().reaction_group_by_key(&key);

                if let Some(reaction) = reaction_group.and_then(|group| group.user_reaction()) {
                    // The user already sent that reaction, redact it.
                    room.redact(reaction.matrix_event_id(), None);
                } else {
                    // The user didn't send that redaction, send it.
                    room.send_reaction(key, event.matrix_event_id());
                }
            }));
            action_group.add_action(&toggle_reaction);

            if let MessageType::File(_) = message.msgtype {
                // Save message's file
                let file_save = gio::SimpleAction::new("file-save", None);
                file_save.connect_activate(
                    clone!(@weak self as widget, @weak event => move |_, _| {
                        widget.save_event_file(event);
                    }),
                );
                action_group.add_action(&file_save);

                // Open message's file
                let file_open = gio::SimpleAction::new("file-open", None);
                file_open.connect_activate(
                    clone!(@weak self as widget, @weak event => move |_, _| {
                        widget.open_event_file(event);
                    }),
                );
                action_group.add_action(&file_open);
            }
        }

        self.insert_action_group("event", Some(&action_group));
        Some(action_group)
    }

    /// Save the file in `event`.
    ///
    /// See `Event::get_media_content` for compatible events. Panics on an incompatible event.
    fn save_event_file(&self, event: Event) {
        let window: Window = self.root().unwrap().downcast().unwrap();
        spawn!(
            glib::PRIORITY_LOW,
            clone!(@weak window => async move {
                let (_, filename, data) = match event.get_media_content().await {
                    Ok(res) => res,
                    Err(err) => {
                        error!("Could not get file: {}", err);

                        let error_message = err.to_user_facing();
                        let error = Error::new(move |_| {
                            let error_label = gtk::Label::builder()
                                .label(&error_message)
                                .wrap(true)
                                .build();
                            Some(error_label.upcast())
                        });
                        window.append_error(&error);

                        return;
                    }
                };

                let dialog = gtk::FileChooserDialog::new(
                    Some(&gettext("Save File")),
                    Some(&window),
                    gtk::FileChooserAction::Save,
                    &[
                        (&gettext("Save"), gtk::ResponseType::Accept),
                        (&gettext("Cancel"), gtk::ResponseType::Cancel),
                    ],
                );
                dialog.set_current_name(&filename);

                let response = dialog.run_future().await;
                if response == gtk::ResponseType::Accept {
                    if let Some(file) = dialog.file() {
                        file.replace_contents(
                            &data,
                            None,
                            false,
                            gio::FileCreateFlags::REPLACE_DESTINATION,
                            gio::Cancellable::NONE,
                        )
                        .unwrap();
                    }
                }

                dialog.close();
            })
        );
    }

    /// Open the file in `event`.
    ///
    /// See `Event::get_media_content` for compatible events. Panics on an incompatible event.
    fn open_event_file(&self, event: Event) {
        let window: Window = self.root().unwrap().downcast().unwrap();
        spawn!(
            glib::PRIORITY_LOW,
            clone!(@weak window => async move {
                let (uid, filename, data) = match event.get_media_content().await {
                    Ok(res) => res,
                    Err(err) => {
                        error!("Could not get file: {}", err);

                        let error_message = err.to_user_facing();
                        let error = Error::new(move |_| {
                            let error_label = gtk::Label::builder()
                                .label(&error_message)
                                .wrap(true)
                                .build();
                            Some(error_label.upcast())
                        });
                        window.append_error(&error);

                        return;
                    }
                };

                let mut path = cache_dir();
                path.push(uid);
                if !path.exists() {
                    let dir = gio::File::for_path(path.clone());
                    dir.make_directory_with_parents(gio::Cancellable::NONE)
                        .unwrap();
                }

                path.push(filename);
                let file = gio::File::for_path(path);

                file.replace_contents(
                    &data,
                    None,
                    false,
                    gio::FileCreateFlags::REPLACE_DESTINATION,
                    gio::Cancellable::NONE,
                )
                .unwrap();

                if let Err(error) = gio::AppInfo::launch_default_for_uri_future(
                    &file.uri(),
                    gio::AppLaunchContext::NONE,
                )
                .await
                {
                    error!("Error opening file '{}': {}", file.uri(), error);
                }
            })
        );
    }
}
