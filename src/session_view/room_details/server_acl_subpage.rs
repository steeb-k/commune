use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{glib, glib::clone};
use matrix_sdk::event_handler::EventHandlerDropGuard;
use ruma::{
    ServerName,
    events::{
        StateEventType, SyncStateEvent,
        room::{power_levels::PowerLevelAction, server_acl::RoomServerAclEventContent},
    },
};

use crate::{
    components::{
        EntryAddRow, LoadingButton, RemovableRow, UnsavedChangesResponse,
        confirm_exclude_own_server_dialog, unsaved_changes_dialog,
    },
    gettext_f,
    prelude::*,
    session::Room,
    spawn, toast,
    utils::BoundObjectWeakRef,
};

mod imp {
    use std::cell::{Cell, RefCell};

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/session_view/room_details/server_acl_subpage.ui")]
    #[properties(wrapper_type = super::ServerAclSubpage)]
    pub struct ServerAclSubpage {
        #[template_child]
        save_button: TemplateChild<LoadingButton>,
        #[template_child]
        allowed_list: TemplateChild<gtk::ListBox>,
        #[template_child]
        denied_list: TemplateChild<gtk::ListBox>,
        #[template_child]
        allowed_add_row: TemplateChild<EntryAddRow>,
        #[template_child]
        denied_add_row: TemplateChild<EntryAddRow>,
        #[template_child]
        ip_literals_row: TemplateChild<adw::SwitchRow>,
        #[template_child]
        error_revealer: TemplateChild<gtk::Revealer>,
        #[template_child]
        error_label: TemplateChild<gtk::Label>,
        /// The presented room.
        #[property(get, set = Self::set_room, explicit_notify, nullable)]
        room: BoundObjectWeakRef<Room>,
        /// Whether the ACL was changed by the user.
        #[property(get)]
        changed: Cell<bool>,
        /// The ACL currently in the room, as far as we know.
        ///
        /// `None` means the room has no `m.room.server_acl` at all, which is
        /// not the same as one that allows nothing.
        remote: RefCell<Option<RoomServerAclEventContent>>,
        /// The servers allowed by the ACL being edited.
        allowed: RefCell<Vec<String>>,
        /// The servers denied by the ACL being edited.
        denied: RefCell<Vec<String>>,
        permissions_handler: RefCell<Option<glib::SignalHandlerId>>,
        acl_drop_guard: RefCell<Option<EventHandlerDropGuard>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ServerAclSubpage {
        const NAME: &'static str = "RoomDetailsServerAclSubpage";
        type Type = super::ServerAclSubpage;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            EntryAddRow::ensure_type();
            RemovableRow::ensure_type();

            Self::bind_template(klass);
            Self::bind_template_callbacks(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for ServerAclSubpage {
        fn constructed(&self) {
            self.parent_constructed();

            self.allowed_list.append(&*self.allowed_add_row);
            self.denied_list.append(&*self.denied_add_row);
        }

        fn dispose(&self) {
            self.disconnect_signals();
        }
    }

    impl WidgetImpl for ServerAclSubpage {}
    impl NavigationPageImpl for ServerAclSubpage {}

    #[gtk::template_callbacks]
    impl ServerAclSubpage {
        /// Set the presented room.
        fn set_room(&self, room: Option<&Room>) {
            let Some(room) = room else {
                // Just ignore when room is missing.
                return;
            };

            self.disconnect_signals();

            let permissions_handler = room.permissions().connect_changed(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_| {
                    imp.update_editability();
                }
            ));
            self.permissions_handler.replace(Some(permissions_handler));

            self.room.set(room, vec![]);

            self.watch_server_acl();
            self.update_editability();

            spawn!(clone!(
                #[weak(rename_to = imp)]
                self,
                async move {
                    imp.load().await;
                }
            ));

            self.obj().notify_room();
        }

        /// Watch for changes of the ACL in the room.
        ///
        /// The ACL is not part of the SDK's room info, so it is not covered by
        /// the room's own update stream.
        fn watch_server_acl(&self) {
            let Some(room) = self.room.obj() else {
                return;
            };
            let matrix_room = room.matrix_room();

            let obj_weak = glib::SendWeakRef::from(self.obj().downgrade());
            let handle = matrix_room.add_event_handler(
                move |_event: SyncStateEvent<RoomServerAclEventContent>| {
                    let obj_weak = obj_weak.clone();
                    async move {
                        let ctx = glib::MainContext::default();
                        ctx.spawn(async move {
                            spawn!(async move {
                                if let Some(obj) = obj_weak.upgrade() {
                                    obj.imp().load().await;
                                }
                            });
                        });
                    }
                },
            );

            let drop_guard = matrix_room.client().event_handler_drop_guard(handle);
            self.acl_drop_guard.replace(Some(drop_guard));
        }

        /// Load the ACL of the room.
        ///
        /// Edits in progress are kept: only the remote side is refreshed, so
        /// the page does not lose what was typed into it.
        async fn load(&self) {
            let Some(room) = self.room.obj() else {
                return;
            };

            let Ok(remote) = room.server_acl().await else {
                toast!(self.obj(), gettext("Could not load server access list"));
                return;
            };

            self.remote.replace(remote);

            if self.changed.get() {
                self.update_changed();
            } else {
                self.reset();
            }
        }

        /// Reset the edited ACL to the one currently in the room.
        fn reset(&self) {
            let acl = self
                .remote
                .borrow()
                .clone()
                .unwrap_or_else(unrestricted_acl);

            self.allowed.replace(acl.allow);
            self.denied.replace(acl.deny);
            self.ip_literals_row.set_active(acl.allow_ip_literals);

            self.rebuild_lists();
            self.save_button.set_is_loading(false);
            self.update_changed();
        }

        /// Whether we can change the ACL of the room.
        fn can_change(&self) -> bool {
            let Some(room) = self.room.obj() else {
                return false;
            };

            room.permissions()
                .is_allowed_to(PowerLevelAction::SendState(StateEventType::RoomServerAcl))
        }

        /// Update the page for whether the ACL can be changed.
        fn update_editability(&self) {
            let can_change = self.can_change();

            self.allowed_add_row.set_visible(can_change);
            self.denied_add_row.set_visible(can_change);
            self.save_button.set_visible(can_change);

            self.rebuild_lists();
            self.update_changed();
        }

        /// The ACL as it is currently edited in this page.
        fn draft(&self) -> RoomServerAclEventContent {
            RoomServerAclEventContent::new(
                self.ip_literals_row.is_active(),
                self.allowed.borrow().clone(),
                self.denied.borrow().clone(),
            )
        }

        /// Rebuild both lists of servers from the edited ACL.
        fn rebuild_lists(&self) {
            let can_change = self.can_change();

            rebuild_list(
                &self.allowed_list,
                &self.allowed.borrow(),
                can_change,
                &gettext("Remove allowed server"),
                clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |server| {
                        imp.remove_server(ServerList::Allowed, &server);
                    }
                ),
            );
            rebuild_list(
                &self.denied_list,
                &self.denied.borrow(),
                can_change,
                &gettext("Remove blocked server"),
                clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |server| {
                        imp.remove_server(ServerList::Denied, &server);
                    }
                ),
            );
        }

        /// Add the server in the given entry to the given list.
        fn add_server(&self, list: ServerList, row: &EntryAddRow) {
            let server = row.text().trim().to_owned();

            if server.is_empty() {
                return;
            }

            {
                let mut servers = match list {
                    ServerList::Allowed => self.allowed.borrow_mut(),
                    ServerList::Denied => self.denied.borrow_mut(),
                };

                if servers.iter().any(|s| s.eq_ignore_ascii_case(&server)) {
                    // It is already in the list, just clear the entry.
                    row.set_text("");
                    return;
                }

                servers.push(server);
            }

            row.set_text("");

            self.rebuild_lists();
            self.update_changed();
        }

        /// Add the server in the entry to the allowed servers.
        #[template_callback]
        fn add_allowed_server(&self) {
            self.add_server(ServerList::Allowed, &self.allowed_add_row);
        }

        /// Add the server in the entry to the denied servers.
        #[template_callback]
        fn add_denied_server(&self) {
            self.add_server(ServerList::Denied, &self.denied_add_row);
        }

        /// Remove the given server from the given list.
        fn remove_server(&self, list: ServerList, server: &str) {
            {
                let mut servers = match list {
                    ServerList::Allowed => self.allowed.borrow_mut(),
                    ServerList::Denied => self.denied.borrow_mut(),
                };

                servers.retain(|s| s != server);
            }

            self.rebuild_lists();
            self.update_changed();
        }

        /// Update whether the ACL was changed by the user.
        #[template_callback]
        fn update_changed(&self) {
            let changed = if self.can_change() {
                let remote = self
                    .remote
                    .borrow()
                    .clone()
                    .unwrap_or_else(unrestricted_acl);

                !acls_are_equal(&self.draft(), &remote)
            } else {
                false
            };

            self.changed.set(changed);
            self.obj().notify_changed();

            self.update_error();
        }

        /// Show the reason why the edited ACL cannot be sent, if there is one.
        fn update_error(&self) {
            let show_error = self.changed.get() && self.allowed.borrow().is_empty();

            if show_error {
                self.error_label.set_label(&gettext(
                    "At least one allowed server is required. An empty list shuts every homeserver out of the room, which cannot be undone from here.",
                ));
            }

            self.error_revealer.set_reveal_child(show_error);
        }

        /// Save the changes of this page.
        ///
        /// Returns `true` if there was nothing to do or if the new ACL was
        /// sent.
        async fn try_save(&self) -> bool {
            if !self.changed.get() {
                // Nothing to do.
                return true;
            }

            let Some(room) = self.room.obj() else {
                return false;
            };

            let acl = self.draft();
            let own_member = room.own_member();
            let own_server = own_member.user_id().server_name();

            match check_acl(&acl, own_server) {
                Some(AclProblem::NoServerAllowed) => {
                    // This one is fatal, so it is refused rather than confirmed.
                    self.update_error();
                    return false;
                }
                Some(AclProblem::OwnServerExcluded) => {
                    let confirmed =
                        confirm_exclude_own_server_dialog(own_server.as_str(), &*self.obj()).await;

                    if !confirmed {
                        return false;
                    }
                }
                None => {}
            }

            self.save_button.set_is_loading(true);

            let result = room.set_server_acl(acl).await;

            self.save_button.set_is_loading(false);

            if result.is_err() {
                toast!(self.obj(), gettext("Could not change server access"));
                return false;
            }

            true
        }

        /// Save the changes of this page.
        #[template_callback]
        async fn save(&self) {
            self.try_save().await;
        }

        /// Go back to the previous page in the room details.
        ///
        /// If there are changes in the page, ask the user to confirm.
        #[template_callback]
        async fn go_back(&self) {
            let obj = self.obj();
            let mut reset_after = false;

            if self.changed.get() {
                match unsaved_changes_dialog(&*obj).await {
                    UnsavedChangesResponse::Save => {
                        if !self.try_save().await {
                            // Stay on the page so the problem can be fixed.
                            return;
                        }
                    }
                    UnsavedChangesResponse::Discard => reset_after = true,
                    UnsavedChangesResponse::Cancel => return,
                }
            }

            let _ = obj.activate_action("navigation.pop", None);

            if reset_after {
                self.reset();
            }
        }

        /// Disconnect all the signal handlers.
        fn disconnect_signals(&self) {
            if let Some(room) = self.room.obj()
                && let Some(handler) = self.permissions_handler.take()
            {
                room.permissions().disconnect(handler);
            }

            self.acl_drop_guard.take();
            self.room.disconnect_signals();
        }
    }
}

glib::wrapper! {
    /// Subpage to edit which servers can take part in a room.
    pub struct ServerAclSubpage(ObjectSubclass<imp::ServerAclSubpage>)
        @extends gtk::Widget, adw::NavigationPage,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl ServerAclSubpage {
    /// Construct a new `ServerAclSubpage` for the given room.
    pub fn new(room: &Room) -> Self {
        glib::Object::builder().property("room", room).build()
    }
}

/// Which of the two lists of servers is being acted on.
#[derive(Debug, Clone, Copy)]
enum ServerList {
    Allowed,
    Denied,
}

/// Rebuild the given list box with a row per server.
///
/// The add row is expected to be the last child, and is left in place.
fn rebuild_list(
    list: &gtk::ListBox,
    servers: &[String],
    can_change: bool,
    remove_tooltip: &str,
    on_remove: impl Fn(String) + Clone + 'static,
) {
    while let Some(child) = list.first_child() {
        if child.downcast_ref::<EntryAddRow>().is_some() {
            break;
        }

        list.remove(&child);
    }

    for (position, server) in (0i32..).zip(servers.iter()) {
        let row: gtk::Widget = if can_change {
            let row = RemovableRow::new();
            row.set_title(server);
            row.set_remove_button_tooltip_text(Some(remove_tooltip));
            row.set_remove_button_accessible_label(Some(gettext_f(
                // Translators: Do NOT translate the content between '{' and '}', this is a
                // variable name.
                "Remove {server}",
                &[("server", server)],
            )));

            let server = server.clone();
            let on_remove = on_remove.clone();
            row.connect_remove(move |_| {
                on_remove(server.clone());
            });

            row.upcast()
        } else {
            let row = adw::ActionRow::new();
            row.set_selectable(false);
            row.set_title(server);

            row.upcast()
        };

        list.insert(&row, position);
    }
}

/// The ACL of a room that has no `m.room.server_acl` event: nothing is
/// restricted.
fn unrestricted_acl() -> RoomServerAclEventContent {
    RoomServerAclEventContent::new(true, vec!["*".to_owned()], Vec::new())
}

/// Whether the two given ACLs would have the same effect.
fn acls_are_equal(lhs: &RoomServerAclEventContent, rhs: &RoomServerAclEventContent) -> bool {
    lhs.allow_ip_literals == rhs.allow_ip_literals && lhs.allow == rhs.allow && lhs.deny == rhs.deny
}

/// What is wrong with an ACL, if anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AclProblem {
    /// No server is allowed, so the room is unusable by everyone.
    NoServerAllowed,
    /// Our own server would be shut out of the room.
    OwnServerExcluded,
}

/// Check the given ACL for the problems we warn about, from the worst down.
fn check_acl(acl: &RoomServerAclEventContent, own_server: &ServerName) -> Option<AclProblem> {
    if acl.allow.is_empty() {
        return Some(AclProblem::NoServerAllowed);
    }

    if !acl.is_allowed(own_server) {
        return Some(AclProblem::OwnServerExcluded);
    }

    None
}

#[cfg(test)]
mod tests {
    use ruma::server_name;

    use super::*;

    #[test]
    fn an_empty_allow_list_is_the_fatal_problem() {
        let acl = RoomServerAclEventContent::new(true, Vec::new(), vec!["evil.example".to_owned()]);

        assert_eq!(
            check_acl(&acl, server_name!("example.org")),
            Some(AclProblem::NoServerAllowed)
        );
    }

    #[test]
    fn an_empty_allow_list_wins_over_our_own_server() {
        // Both problems apply, and the fatal one must be the one reported,
        // because it is the one we refuse rather than confirm.
        let acl = RoomServerAclEventContent::new(true, Vec::new(), vec!["example.org".to_owned()]);

        assert_eq!(
            check_acl(&acl, server_name!("example.org")),
            Some(AclProblem::NoServerAllowed)
        );
    }

    #[test]
    fn denying_our_own_server_is_a_warning() {
        let acl = RoomServerAclEventContent::new(
            true,
            vec!["*".to_owned()],
            vec!["example.org".to_owned()],
        );

        assert_eq!(
            check_acl(&acl, server_name!("example.org")),
            Some(AclProblem::OwnServerExcluded)
        );
    }

    #[test]
    fn not_allowing_our_own_server_is_a_warning() {
        let acl =
            RoomServerAclEventContent::new(true, vec!["other.example".to_owned()], Vec::new());

        assert_eq!(
            check_acl(&acl, server_name!("example.org")),
            Some(AclProblem::OwnServerExcluded)
        );
    }

    #[test]
    fn an_ip_literal_of_our_own_is_excluded_by_the_switch() {
        let acl = RoomServerAclEventContent::new(false, vec!["*".to_owned()], Vec::new());

        assert_eq!(
            check_acl(&acl, server_name!("1.1.1.1")),
            Some(AclProblem::OwnServerExcluded)
        );
        assert_eq!(check_acl(&acl, server_name!("example.org")), None);
    }

    #[test]
    fn a_wildcard_covers_our_own_subdomain() {
        let acl = RoomServerAclEventContent::new(
            false,
            vec!["*.example.org".to_owned()],
            vec!["evil.example".to_owned()],
        );

        assert_eq!(check_acl(&acl, server_name!("matrix.example.org")), None);
        assert_eq!(
            check_acl(&acl, server_name!("example.org")),
            Some(AclProblem::OwnServerExcluded)
        );
    }

    #[test]
    fn a_room_without_an_acl_is_not_restricted() {
        let acl = unrestricted_acl();

        assert_eq!(check_acl(&acl, server_name!("example.org")), None);
        assert_eq!(check_acl(&acl, server_name!("1.1.1.1")), None);
    }

    #[test]
    fn the_unrestricted_baseline_is_not_a_change() {
        // Opening the page on a room with no ACL must not offer to save
        // anything, otherwise every visit would write an event.
        assert!(acls_are_equal(&unrestricted_acl(), &unrestricted_acl()));

        let with_a_deny = RoomServerAclEventContent::new(
            true,
            vec!["*".to_owned()],
            vec!["evil.example".to_owned()],
        );
        assert!(!acls_are_equal(&with_a_deny, &unrestricted_acl()));
    }

    #[test]
    fn the_ip_literal_switch_alone_is_a_change() {
        let mut acl = unrestricted_acl();
        acl.allow_ip_literals = false;

        assert!(!acls_are_equal(&acl, &unrestricted_acl()));
    }
}
