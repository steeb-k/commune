use adw::{prelude::*, subclass::prelude::*};
use gtk::{glib, glib::clone, CompositeTemplate};
use ruma::events::AnyMessageLikeEventContent;
use tracing::error;

use super::{MediaItem, Timeline, TimelineFilter};
use crate::{
    session::{model::Room, view::MediaViewer},
    spawn,
};

const MIN_N_ITEMS: u32 = 50;

mod imp {
    use std::{cell::OnceCell, marker::PhantomData};

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/content/room_details/history_viewer/media.ui"
    )]
    #[properties(wrapper_type = super::MediaHistoryViewer)]
    pub struct MediaHistoryViewer {
        /// The room to search for media events.
        #[property(get = Self::room, set = Self::set_room, construct_only)]
        pub room: PhantomData<Room>,
        pub room_timeline: OnceCell<Timeline>,
        #[template_child]
        pub media_viewer: TemplateChild<MediaViewer>,
        #[template_child]
        pub grid_view: TemplateChild<gtk::GridView>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MediaHistoryViewer {
        const NAME: &'static str = "ContentMediaHistoryViewer";
        type Type = super::MediaHistoryViewer;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            MediaItem::static_type();
            Self::bind_template(klass);

            klass.set_css_name("mediahistoryviewer");
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for MediaHistoryViewer {}

    impl WidgetImpl for MediaHistoryViewer {}
    impl NavigationPageImpl for MediaHistoryViewer {}

    impl MediaHistoryViewer {
        /// The room to search for media events.
        fn room(&self) -> Room {
            self.room_timeline.get().unwrap().room()
        }

        /// Set the room to search for media events.
        fn set_room(&self, room: Room) {
            let timeline = Timeline::new(&room, TimelineFilter::Media);
            let model = gtk::NoSelection::new(Some(timeline.clone()));
            self.grid_view.set_model(Some(&model));

            // Load an initial number of items
            spawn!(clone!(@weak self as imp, @weak timeline => async move {
                while timeline.n_items() < MIN_N_ITEMS {
                    if !timeline.load().await {
                        break;
                    }
                }

                let adj = imp.grid_view.vadjustment().unwrap();
                adj.connect_value_notify(clone!(@weak timeline => move |adj| {
                    if adj.value() + adj.page_size() * 2.0 >= adj.upper() {
                        spawn!(async move { timeline.load().await; });
                    }
                }));
            }));

            self.room_timeline.set(timeline).unwrap();
        }
    }
}

glib::wrapper! {
    /// A view presenting the list of media (image or video) events in a room.
    pub struct MediaHistoryViewer(ObjectSubclass<imp::MediaHistoryViewer>)
        @extends gtk::Widget, adw::NavigationPage;
}

impl MediaHistoryViewer {
    pub fn new(room: &Room) -> Self {
        glib::Object::builder().property("room", room).build()
    }

    /// Show the given media item.
    pub fn show_media(&self, item: &MediaItem) {
        let imp = self.imp();
        let event = item.event().unwrap();

        let Some(AnyMessageLikeEventContent::RoomMessage(message)) = event.original_content()
        else {
            error!("Trying to open the media viewer with an event that is not a message");
            return;
        };

        imp.media_viewer.set_message(
            &event.room().unwrap(),
            event.matrix_event().0.event_id().into(),
            message.msgtype,
        );
        imp.media_viewer.reveal(item);
    }
}
