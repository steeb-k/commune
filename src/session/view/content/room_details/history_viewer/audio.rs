use adw::{prelude::*, subclass::prelude::*};
use gtk::{glib, glib::clone, CompositeTemplate};

use super::{AudioRow, HistoryViewerEvent, HistoryViewerEventType, HistoryViewerTimeline};
use crate::spawn;

const MIN_N_ITEMS: u32 = 20;

mod imp {
    use std::cell::OnceCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/content/room_details/history_viewer/audio.ui"
    )]
    #[properties(wrapper_type = super::AudioHistoryViewer)]
    pub struct AudioHistoryViewer {
        /// The timeline containing the audio events.
        #[property(get, set = Self::set_timeline, construct_only)]
        pub timeline: OnceCell<HistoryViewerTimeline>,
        #[template_child]
        pub list_view: TemplateChild<gtk::ListView>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for AudioHistoryViewer {
        const NAME: &'static str = "ContentAudioHistoryViewer";
        type Type = super::AudioHistoryViewer;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            AudioRow::ensure_type();

            Self::bind_template(klass);

            klass.set_css_name("audio-history-viewer");
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
        /// Set the timeline containing the audio events.
        fn set_timeline(&self, timeline: HistoryViewerTimeline) {
            let filter = gtk::CustomFilter::new(|obj| {
                obj.downcast_ref::<HistoryViewerEvent>()
                    .is_some_and(|e| e.event_type() == HistoryViewerEventType::Audio)
            });
            let filter_model = gtk::FilterListModel::new(Some(timeline.clone()), Some(filter));

            let model = gtk::NoSelection::new(Some(filter_model));
            self.list_view.set_model(Some(&model));

            // Load an initial number of items
            spawn!(clone!(@weak self as imp, @weak timeline => async move {
                while model.n_items() < MIN_N_ITEMS {
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

            self.timeline.set(timeline).unwrap();
        }
    }
}

glib::wrapper! {
    /// A view presenting the list of audio events in a room.
    pub struct AudioHistoryViewer(ObjectSubclass<imp::AudioHistoryViewer>)
        @extends gtk::Widget, adw::NavigationPage;
}

impl AudioHistoryViewer {
    pub fn new(timeline: &HistoryViewerTimeline) -> Self {
        glib::Object::builder()
            .property("timeline", timeline)
            .build()
    }
}
