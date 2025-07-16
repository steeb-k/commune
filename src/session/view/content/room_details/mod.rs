// FIXME: AdwPreferencesWindow is deprecated but we cannot use
// AdwPreferencesDialog yet because we need to be able to open the media viewer.
#![allow(deprecated)]

use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{CompositeTemplate, glib, glib::clone};
use ruma::UserId;

mod addresses_subpage;
mod edit_details_subpage;
mod general_page;
mod history_viewer;
mod invite_subpage;
mod member_row;
mod members_page;
mod membership_subpage_item;
mod permissions;
mod room_upgrade_dialog;

use self::{
    addresses_subpage::AddressesSubpage,
    edit_details_subpage::EditDetailsSubpage,
    general_page::GeneralPage,
    history_viewer::{
        AudioHistoryViewer, FileHistoryViewer, HistoryViewerTimeline, VisualMediaHistoryViewer,
    },
    invite_subpage::InviteSubpage,
    member_row::MemberRow,
    members_page::MembersPage,
    membership_subpage_item::MembershipSubpageItem,
    permissions::PermissionsSubpage,
};
use crate::{
    components::UserPage,
    session::model::{MemberList, Room},
    toast,
};

/// The possible subpages of the room details.
#[derive(Debug, Hash, Eq, PartialEq, Clone, Copy, glib::Variant)]
pub(crate) enum SubpageName {
    /// The page to edit the name, topic and avatar of the room.
    EditDetails,
    /// The list of members of the room.
    Members,
    /// The form to invite new members.
    Invite,
    /// The history of visual media.
    VisualMediaHistory,
    /// The history of files.
    FileHistory,
    /// The history of audio.
    AudioHistory,
    /// The page to edit the public addresses of the room.
    Addresses,
    /// The page to edit the permissions of the room.
    Permissions,
}

mod imp {
    use std::{
        cell::{OnceCell, RefCell},
        collections::HashMap,
    };

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/session/view/content/room_details/mod.ui")]
    #[properties(wrapper_type = super::RoomDetails)]
    pub struct RoomDetails {
        /// The room to show the details for.
        #[property(get, set = Self::set_room, construct_only)]
        room: OnceCell<Room>,
        /// The list of members in the room.
        #[property(get)]
        members: OnceCell<MemberList>,
        /// The timeline for the history viewers.
        #[property(get)]
        timeline: OnceCell<HistoryViewerTimeline>,
        /// The general page.
        general_page: OnceCell<GeneralPage>,
        /// The subpages that are loaded.
        ///
        /// We keep them around to avoid reloading them if the user reopens the
        /// same subpage.
        subpages: RefCell<HashMap<SubpageName, adw::NavigationPage>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RoomDetails {
        const NAME: &'static str = "RoomDetails";
        type Type = super::RoomDetails;
        type ParentType = adw::PreferencesWindow;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);

            klass.install_action(
                "details.show-subpage",
                Some(&String::static_variant_type()),
                |obj, _, param| {
                    let subpage = param
                        .and_then(glib::Variant::get::<SubpageName>)
                        .expect("The parameter should be a valid subpage name");

                    obj.imp().show_subpage(subpage, false);
                },
            );

            klass.install_action(
                "details.show-member",
                Some(&String::static_variant_type()),
                |obj, _, param| {
                    let Some(user_id) = param
                        .and_then(glib::Variant::get::<String>)
                        .and_then(|s| UserId::parse(s).ok())
                    else {
                        return;
                    };

                    let member = obj.members().get_or_create(user_id);
                    let user_page = UserPage::new(&member);
                    user_page.connect_close(clone!(
                        #[weak]
                        obj,
                        move |_| {
                            obj.pop_subpage();
                            toast!(
                                obj,
                                gettext("The user is not in the room members list anymore"),
                            );
                        }
                    ));

                    obj.push_subpage(&user_page);
                },
            );

            klass.install_action("win.toggle-fullscreen", None, |obj, _, _| {
                if obj.is_fullscreen() {
                    obj.unfullscreen();
                } else {
                    obj.fullscreen();
                }
            });
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for RoomDetails {}

    impl WidgetImpl for RoomDetails {
        fn map(&self) {
            self.parent_map();

            self.general_page
                .get()
                .expect("general page is initialized")
                .unselect_topic();
        }
    }

    impl WindowImpl for RoomDetails {}
    impl AdwWindowImpl for RoomDetails {}
    impl PreferencesWindowImpl for RoomDetails {}

    impl RoomDetails {
        /// Set the room to show the details for.
        fn set_room(&self, room: Room) {
            let room = self.room.get_or_init(|| room);

            // Initialize the media history viewers timeline.
            self.timeline
                .set(HistoryViewerTimeline::new(room))
                .expect("timeline should be uninitialized");

            // Keep a strong reference to members list.
            self.members
                .set(room.get_or_create_members())
                .expect("members should be uninitialized");

            // Initialize the general page.
            let general_page = self
                .general_page
                .get_or_init(|| GeneralPage::new(room, self.members()));
            self.obj().add(general_page);
        }

        /// The list of members in the room.
        fn members(&self) -> &MemberList {
            self.members.get().expect("members should be initialized")
        }

        /// The timeline for the history viewers.
        fn timeline(&self) -> &HistoryViewerTimeline {
            self.timeline.get().expect("timeline should be initialized")
        }

        /// Show the subpage with the given name.
        pub(super) fn show_subpage(&self, name: SubpageName, is_initial: bool) {
            let room = self.room.get().expect("room should be initialized");

            let mut subpages = self.subpages.borrow_mut();
            let subpage = subpages.entry(name).or_insert_with(|| match name {
                SubpageName::EditDetails => EditDetailsSubpage::new(room).upcast(),
                SubpageName::Members => MembersPage::new(room, self.members()).upcast(),
                SubpageName::Invite => InviteSubpage::new(room).upcast(),
                SubpageName::VisualMediaHistory => {
                    VisualMediaHistoryViewer::new(self.timeline()).upcast()
                }
                SubpageName::FileHistory => FileHistoryViewer::new(self.timeline()).upcast(),
                SubpageName::AudioHistory => AudioHistoryViewer::new(self.timeline()).upcast(),
                SubpageName::Addresses => AddressesSubpage::new(room).upcast(),
                SubpageName::Permissions => PermissionsSubpage::new(&room.permissions()).upcast(),
            });

            if is_initial {
                subpage.set_can_pop(false);
            }

            self.obj().push_subpage(subpage);
        }
    }
}

glib::wrapper! {
    /// Preference Window to display and update room details.
    pub struct RoomDetails(ObjectSubclass<imp::RoomDetails>)
        @extends gtk::Widget, gtk::Window, adw::Window, gtk::Root, adw::PreferencesWindow,
        @implements gtk::Accessible;
}

impl RoomDetails {
    /// Construct a `RoomDetails` for the given room with the given parent
    /// window.
    pub fn new(parent_window: Option<&gtk::Window>, room: &Room) -> Self {
        glib::Object::builder()
            .property("transient-for", parent_window)
            .property("room", room)
            .build()
    }

    /// Show the given subpage as the initial page.
    pub(crate) fn show_initial_subpage(&self, name: SubpageName) {
        self.imp().show_subpage(name, true);
    }
}
