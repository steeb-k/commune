mod general_page;
mod history_viewer;
mod invite_subpage;
mod members_page;

use std::convert::From;

use adw::{prelude::*, subclass::prelude::*};
use gtk::{glib, CompositeTemplate};

pub use self::{
    general_page::GeneralPage,
    history_viewer::{
        AudioHistoryViewer, FileHistoryViewer, HistoryViewerTimeline, MediaHistoryViewer,
    },
    invite_subpage::InviteSubpage,
    members_page::MembersPage,
};
use crate::session::model::Room;

#[derive(Debug, Hash, Eq, PartialEq, Clone, Copy, glib::Variant)]
pub enum SubpageName {
    Members,
    Invite,
    MediaHistory,
    FileHistory,
    AudioHistory,
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
        #[property(get, construct_only)]
        pub room: RefCell<Option<Room>>,
        /// The timeline for the history viewers.
        #[property(get = Self::timeline)]
        pub timeline: OnceCell<HistoryViewerTimeline>,
        /// The subpages that are loaded.
        ///
        /// We keep them around to avoid reloading them if the user reopens the
        /// same subpage.
        pub subpages: RefCell<HashMap<SubpageName, adw::NavigationPage>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RoomDetails {
        const NAME: &'static str = "RoomDetails";
        type Type = super::RoomDetails;
        type ParentType = adw::PreferencesWindow;

        fn class_init(klass: &mut Self::Class) {
            GeneralPage::static_type();
            Self::bind_template(klass);

            klass.install_action(
                "details.show-subpage",
                Some("s"),
                move |widget, _, param| {
                    let subpage = param
                        .and_then(|variant| variant.get::<SubpageName>())
                        .expect("The parameter should be a valid subpage name");

                    widget.show_subpage(subpage, false);
                },
            );

            klass.install_action("win.toggle-fullscreen", None, |obj, _, _| {
                if obj.is_fullscreened() {
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

    impl WidgetImpl for RoomDetails {}
    impl WindowImpl for RoomDetails {}
    impl AdwWindowImpl for RoomDetails {}
    impl PreferencesWindowImpl for RoomDetails {}

    impl RoomDetails {
        /// The timeline for the history viewers.
        fn timeline(&self) -> HistoryViewerTimeline {
            self.timeline
                .get_or_init(|| {
                    let room = self.room.borrow().clone().expect(
                        "timeline should not be requested before RoomDetails is constructed",
                    );
                    HistoryViewerTimeline::new(&room)
                })
                .clone()
        }
    }
}

glib::wrapper! {
    /// Preference Window to display and update room details.
    pub struct RoomDetails(ObjectSubclass<imp::RoomDetails>)
        @extends gtk::Widget, gtk::Window, adw::Window, gtk::Root, adw::PreferencesWindow, @implements gtk::Accessible;
}

impl RoomDetails {
    pub fn new(parent_window: &Option<gtk::Window>, room: &Room) -> Self {
        glib::Object::builder()
            .property("transient-for", parent_window)
            .property("room", room)
            .build()
    }

    /// Show the subpage with the given name.
    fn show_subpage(&self, name: SubpageName, is_initial: bool) {
        let Some(room) = self.room() else {
            return;
        };
        let imp = self.imp();

        let mut subpages = imp.subpages.borrow_mut();
        let subpage = subpages.entry(name).or_insert_with(|| match name {
            SubpageName::Members => MembersPage::new(&room).upcast(),
            SubpageName::Invite => InviteSubpage::new(&room).upcast(),
            SubpageName::MediaHistory => MediaHistoryViewer::new(&self.timeline()).upcast(),
            SubpageName::FileHistory => FileHistoryViewer::new(&self.timeline()).upcast(),
            SubpageName::AudioHistory => AudioHistoryViewer::new(&self.timeline()).upcast(),
        });

        if is_initial {
            subpage.set_can_pop(false);
        }

        self.push_subpage(subpage);
    }

    /// Show the given subpage as the initial page.
    pub fn show_initial_subpage(&self, name: SubpageName) {
        self.show_subpage(name, true);
    }
}
