use adw::{prelude::*, subclass::prelude::*};
use gettextrs::{gettext, ngettext};
use gtk::{gdk, glib, glib::clone};
use tracing::error;

mod item;
mod list;
mod row;

use matrix_sdk::ruma::{
    api::client::membership::{Invite3pid, Invite3pidInit},
    thirdparty::Medium,
};

use self::{
    item::InviteItem,
    list::{InviteList, InviteListState},
    row::InviteRow,
};
use crate::{
    components::{LoadingButton, PillSearchEntry, PillSource},
    gettext_f,
    prelude::*,
    session::{EmailInviteReadiness, IdentityServerError, PendingTerm, Room, Session, User},
    spawn, spawn_tokio, toast,
};

/// The email address in the given search text, if that is what it holds.
///
/// The same rule the account settings use for an address: an `@` between
/// two non-empty halves, and nothing that cannot be part of one.
fn email_in_search(text: &str) -> Option<String> {
    let text = text.trim();
    let (local, domain) = text.split_once('@')?;

    (!local.is_empty()
        && !domain.is_empty()
        && !domain.contains('@')
        && !text.contains(char::is_whitespace))
    .then(|| text.to_owned())
}

mod imp {
    use std::cell::OnceCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/session_view/room_details/invite_subpage/mod.ui")]
    #[properties(wrapper_type = super::InviteSubpage)]
    pub struct InviteSubpage {
        #[template_child]
        search_entry: TemplateChild<PillSearchEntry>,
        #[template_child]
        list_view: TemplateChild<gtk::ListView>,
        #[template_child]
        invite_button: TemplateChild<LoadingButton>,
        #[template_child]
        email_invite_clamp: TemplateChild<adw::Clamp>,
        #[template_child]
        email_invite_button: TemplateChild<gtk::Button>,
        #[template_child]
        email_invite_label: TemplateChild<gtk::Label>,
        #[template_child]
        cancel_button: TemplateChild<gtk::Button>,
        #[template_child]
        stack: TemplateChild<gtk::Stack>,
        #[template_child]
        matching_page: TemplateChild<gtk::ScrolledWindow>,
        #[template_child]
        no_matching_page: TemplateChild<adw::StatusPage>,
        #[template_child]
        no_search_page: TemplateChild<adw::StatusPage>,
        #[template_child]
        error_page: TemplateChild<adw::StatusPage>,
        /// The room users will be invited to.
        #[property(get, set = Self::set_room, construct_only)]
        room: glib::WeakRef<Room>,
        /// The list managing the invited users.
        #[property(get)]
        invite_list: OnceCell<InviteList>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for InviteSubpage {
        const NAME: &'static str = "RoomDetailsInviteSubpage";
        type Type = super::InviteSubpage;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            InviteRow::ensure_type();

            Self::bind_template(klass);
            Self::bind_template_callbacks(klass);

            klass.add_binding(gdk::Key::Escape, gdk::ModifierType::empty(), |obj| {
                obj.imp().close();
                glib::Propagation::Stop
            });
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for InviteSubpage {}

    impl WidgetImpl for InviteSubpage {}

    impl NavigationPageImpl for InviteSubpage {
        fn shown(&self) {
            self.search_entry.grab_focus();
        }
    }

    #[gtk::template_callbacks]
    impl InviteSubpage {
        /// Set the room users will be invited to.
        fn set_room(&self, room: &Room) {
            let invite_list = self.invite_list.get_or_init(|| InviteList::new(room));
            invite_list.connect_invitee_added(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_, invitee| {
                    imp.search_entry.add_pill(&invitee.user());
                }
            ));

            invite_list.connect_invitee_removed(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_, invitee| {
                    imp.search_entry.remove_pill(&invitee.user().identifier());
                }
            ));

            invite_list.connect_state_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_| {
                    imp.update_view();
                }
            ));

            self.search_entry.connect_activated(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_| {
                    // Return does what the button does, and nothing when the
                    // button would do nothing.
                    if !imp.invite_button.is_sensitive() {
                        return;
                    }

                    spawn!(clone!(
                        #[weak]
                        imp,
                        async move {
                            imp.invite().await;
                        }
                    ));
                }
            ));

            self.search_entry
                .bind_property("text", invite_list, "search-term")
                .sync_create()
                .build();

            self.search_entry.connect_notify_local(
                Some("text"),
                clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_, _| {
                        imp.update_email_invite();
                    }
                ),
            );

            invite_list
                .bind_property("has-invitees", &*self.invite_button, "sensitive")
                .sync_create()
                .build();

            self.list_view
                .set_model(Some(&gtk::NoSelection::new(Some(invite_list.clone()))));

            self.room.set(Some(room));
            self.obj().notify_room();
        }

        /// The list managing the invited users.
        fn invite_list(&self) -> &InviteList {
            self.invite_list
                .get()
                .expect("invite list should be initialized")
        }

        /// Update the view for the current state of the list.
        fn update_view(&self) {
            let state = self.invite_list().state();

            let page = match state {
                InviteListState::Initial => "no-search",
                InviteListState::Loading => "loading",
                InviteListState::NoMatching => "no-results",
                InviteListState::Matching => "results",
                InviteListState::Error => "error",
            };

            self.stack.set_visible_child_name(page);
        }

        /// Offer to invite by email when that is what the search says.
        fn update_email_invite(&self) {
            let email = email_in_search(&self.search_entry.text());

            if let Some(email) = &email {
                self.email_invite_label.set_label(&gettext_f(
                    // Translators: Do NOT translate the content between '{' and '}', this is a
                    // variable name.
                    "Invite {email} by email",
                    &[("email", email)],
                ));
            }
            self.email_invite_clamp.set_visible(email.is_some());
        }

        /// Invite the email address in the search entry to the room.
        #[template_callback]
        async fn invite_by_email(&self) {
            let Some(room) = self.room.upgrade() else {
                return;
            };
            let Some(session) = room.session() else {
                return;
            };
            let Some(email) = email_in_search(&self.search_entry.text()) else {
                return;
            };

            self.email_invite_button.set_sensitive(false);
            self.run_email_invite(&room, &session, email).await;
            self.email_invite_button.set_sensitive(true);
        }

        /// Run the email invite flow for the given address.
        async fn run_email_invite(&self, room: &Room, session: &Session, email: String) {
            let identity_server = session.identity_server();

            // At most twice: once to learn about the terms, once after they
            // were agreed to.
            for terms_seen in [false, true] {
                let readiness = match identity_server.email_invite_readiness(session).await {
                    Ok(readiness) => readiness,
                    Err(IdentityServerError::NoServer) => {
                        toast!(
                            self.obj(),
                            gettext(
                                "Inviting by email needs an identity server, and there is none to use"
                            )
                        );
                        return;
                    }
                    Err(IdentityServerError::Other) => {
                        toast!(self.obj(), gettext("Could not invite by email"));
                        return;
                    }
                };

                match readiness {
                    EmailInviteReadiness::Terms(terms) => {
                        if terms_seen || !self.ask_terms(session, terms).await {
                            return;
                        }
                        // Agreed: go around and ask again.
                    }
                    EmailInviteReadiness::Ready { id_server, token } => {
                        self.send_email_invite(room, id_server, token, email).await;
                        return;
                    }
                }
            }
        }

        /// Present the terms of the identity server and accept them if the
        /// person agrees.
        ///
        /// Returns whether they were agreed to and accepted.
        async fn ask_terms(&self, session: &Session, terms: Vec<PendingTerm>) -> bool {
            let dialog = adw::AlertDialog::builder()
                .heading(gettext("Terms of the Identity Server"))
                .body(gettext(
                    "Inviting by email goes through an identity server, which asks you to agree to its terms first.",
                ))
                .build();

            let links = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .spacing(6)
                .build();
            for term in &terms {
                links.append(
                    &gtk::LinkButton::builder()
                        .label(&term.name)
                        .uri(&term.url)
                        .build(),
                );
            }
            dialog.set_extra_child(Some(&links));

            dialog.add_responses(&[("cancel", &gettext("Cancel")), ("agree", &gettext("Agree"))]);
            dialog.set_response_appearance("agree", adw::ResponseAppearance::Suggested);
            dialog.set_default_response(Some("cancel"));
            dialog.set_close_response("cancel");

            if dialog.choose_future(Some(&*self.obj())).await != "agree" {
                return false;
            }

            let urls = terms.into_iter().map(|term| term.url).collect();
            let accepted = session
                .identity_server()
                .accept_terms(session, urls)
                .await
                .is_ok();
            if !accepted {
                toast!(
                    self.obj(),
                    gettext("Could not accept the terms of the identity server")
                );
            }
            accepted
        }

        /// Send the email invitation itself.
        async fn send_email_invite(
            &self,
            room: &Room,
            id_server: String,
            token: String,
            email: String,
        ) {
            let invite = Invite3pid::from(Invite3pidInit {
                id_server,
                id_access_token: token,
                medium: Medium::Email,
                address: email.clone(),
            });

            let matrix_room = room.matrix_room().clone();
            let handle = spawn_tokio!(async move { matrix_room.invite_user_by_3pid(invite).await });

            match handle.await.expect("task was not aborted") {
                Ok(()) => {
                    self.search_entry.clear();
                    toast!(
                        self.obj(),
                        gettext(
                            // Translators: Do NOT translate the content between '{' and '}', this
                            // is a variable name.
                            "An invitation was sent to {email}",
                        ),
                        email,
                    );
                }
                Err(error) => {
                    error!("Could not invite {email} by email: {error}");
                    toast!(
                        self.obj(),
                        gettext(
                            // Translators: Do NOT translate the content between '{' and '}', this
                            // is a variable name.
                            "Could not invite {email}",
                        ),
                        email,
                    );
                }
            }
        }

        /// Close this subpage.
        #[template_callback]
        fn close(&self) {
            let obj = self.obj();
            let Some(window) = obj.root().and_downcast::<adw::PreferencesWindow>() else {
                return;
            };

            if obj.can_pop() {
                window.pop_subpage();
            } else {
                window.close();
            }
        }

        /// Toggle the invited state of the item at the given index.
        #[template_callback]
        fn toggle_item_is_invitee(&self, index: u32) {
            let Some(item) = self.invite_list().item(index).and_downcast::<InviteItem>() else {
                return;
            };

            item.set_is_invitee(!item.is_invitee());
        }

        /// Uninvite the user from the given pill source.
        #[template_callback]
        fn remove_pill_invitee(&self, source: PillSource) {
            if let Ok(user) = source.downcast::<User>() {
                self.invite_list().remove_invitee(user.user_id());
            }
        }

        /// Invite the selected users to the room.
        #[template_callback]
        async fn invite(&self) {
            let Some(room) = self.room.upgrade() else {
                return;
            };

            self.invite_button.set_is_loading(true);

            let invite_list = self.invite_list();
            let invitees = invite_list.invitees_ids();

            match room.invite(&invitees).await {
                Ok(()) => {
                    self.close();
                }
                Err(failed_users) => {
                    invite_list.retain_invitees(&failed_users);

                    let n_failed = failed_users.len();
                    let n = invite_list.n_invitees();
                    if n != n_failed {
                        // This should not be possible.
                        error!(
                            "The number of failed users does not match the number of remaining invitees: expected {n_failed}, got {n}"
                        );
                    }

                    // We don't use the count in the strings so we use separate gettext calls for
                    // singular and plural rather than using ngettext.
                    if n == 0 {
                        self.close();
                    } else if n == 1 {
                        let first_failed =
                            invite_list.first_invitee().map(|item| item.user()).unwrap();

                        toast!(
                            self.obj(),
                            gettext(
                                // Translators: Do NOT translate the content between '{' and '}', these
                                // are variable names.
                                "Could not invite {user} to {room}",
                            ),
                            @user = first_failed,
                            @room,
                            n,
                        );
                    } else {
                        toast!(
                            self.obj(),
                            ngettext(
                                // Translators: Do NOT translate the content between '{' and '}', these
                                // are variable names. The count is always greater than 1.
                                "Could not invite 1 user to {room}",
                                "Could not invite {n} users to {room}",
                                n as u32,
                            ),
                            @room,
                            n,
                        );
                    }
                }
            }

            self.invite_button.set_is_loading(false);
        }
    }
}

glib::wrapper! {
    /// Subpage to invite new members to a room.
    pub struct InviteSubpage(ObjectSubclass<imp::InviteSubpage>)
        @extends gtk::Widget, adw::NavigationPage,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl InviteSubpage {
    /// Construct a new `InviteSubpage` with the given room.
    pub fn new(room: &Room) -> Self {
        glib::Object::builder().property("room", room).build()
    }
}
