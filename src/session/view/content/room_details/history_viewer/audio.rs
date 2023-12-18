use adw::{prelude::*, subclass::prelude::*};
use gtk::{glib, glib::clone, CompositeTemplate};

use super::{AudioRow, Timeline, TimelineFilter};
use crate::{session::model::Room, spawn};

const MIN_N_ITEMS: u32 = 20;

mod imp {
    use std::{cell::OnceCell, marker::PhantomData};

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/content/room_details/history_viewer/audio.ui"
    )]
    #[properties(wrapper_type = super::AudioHistoryViewer)]
    pub struct AudioHistoryViewer {
        /// The room to search for audio events.
        #[property(get = Self::room, set = Self::set_room, construct_only)]
        pub room: PhantomData<Room>,
        pub room_timeline: OnceCell<Timeline>,
        #[template_child]
        pub list_view: TemplateChild<gtk::ListView>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for AudioHistoryViewer {
        const NAME: &'static str = "ContentAudioHistoryViewer";
        type Type = super::AudioHistoryViewer;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            AudioRow::static_type();
            Self::bind_template(klass);

            klass.set_css_name("audiohistoryviewer");
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for AudioHistoryViewer {}

    impl WidgetImpl for AudioHistoryViewer {}
    impl NavigationPageImpl for AudioHistoryViewer {}

    impl AudioHistoryViewer {
        /// The room to search for audio events.
        fn room(&self) -> Room {
            self.room_timeline.get().unwrap().room()
        }

        /// Set the room to search for audio events.
        fn set_room(&self, room: Room) {
            let timeline = Timeline::new(&room, TimelineFilter::Audio);
            let model = gtk::NoSelection::new(Some(timeline.clone()));
            self.list_view.set_model(Some(&model));

            // Load an initial number of items
            spawn!(clone!(@weak self as imp, @weak timeline => async move {
                while timeline.n_items() < MIN_N_ITEMS {
                    if !timeline.load().await {
                        break;
                    }
                }

                let adj = imp.list_view.vadjustment().unwrap();
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
    /// A view presenting the list of audio events in a room.
    pub struct AudioHistoryViewer(ObjectSubclass<imp::AudioHistoryViewer>)
        @extends gtk::Widget, adw::NavigationPage;
}

impl AudioHistoryViewer {
    pub fn new(room: &Room) -> Self {
        glib::Object::builder().property("room", room).build()
    }
}
