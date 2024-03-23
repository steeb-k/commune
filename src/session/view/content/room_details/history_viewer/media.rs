use adw::{prelude::*, subclass::prelude::*};
use gtk::{glib, glib::clone, CompositeTemplate};

use super::{HistoryViewerEvent, HistoryViewerEventType, HistoryViewerTimeline, MediaItem};
use crate::{session::view::MediaViewer, spawn};

const MIN_N_ITEMS: u32 = 50;

mod imp {
    use std::cell::OnceCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/content/room_details/history_viewer/media.ui"
    )]
    #[properties(wrapper_type = super::MediaHistoryViewer)]
    pub struct MediaHistoryViewer {
        /// The timeline containing the media events.
        #[property(get, set = Self::set_timeline, construct_only)]
        pub timeline: OnceCell<HistoryViewerTimeline>,
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
            MediaItem::ensure_type();

            Self::bind_template(klass);

            klass.set_css_name("media-history-viewer");
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
        /// Set the timeline containing the media events.
        fn set_timeline(&self, timeline: HistoryViewerTimeline) {
            let filter = gtk::CustomFilter::new(|obj| {
                obj.downcast_ref::<HistoryViewerEvent>()
                    .is_some_and(|e| e.event_type() == HistoryViewerEventType::Media)
            });
            let filter_model = gtk::FilterListModel::new(Some(timeline.clone()), Some(filter));

            let model = gtk::NoSelection::new(Some(filter_model));
            self.grid_view.set_model(Some(&model));

            // Load an initial number of items.
            spawn!(
                clone!(@weak self as imp, @weak timeline, @weak model => async move {
                    while model.n_items() < MIN_N_ITEMS {
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
                })
            );

            self.timeline.set(timeline).unwrap();
        }
    }
}

glib::wrapper! {
    /// A view presenting the list of media (image or video) events in a room.
    pub struct MediaHistoryViewer(ObjectSubclass<imp::MediaHistoryViewer>)
        @extends gtk::Widget, adw::NavigationPage;
}

impl MediaHistoryViewer {
    pub fn new(timeline: &HistoryViewerTimeline) -> Self {
        glib::Object::builder()
            .property("timeline", timeline)
            .build()
    }

    /// Show the given media item.
    pub fn show_media(&self, item: &MediaItem) {
        let Some(event) = item.event() else {
            return;
        };
        let Some(room) = event.room() else {
            return;
        };

        let imp = self.imp();
        imp.media_viewer
            .set_message(&room, event.event_id(), event.message_content());
        imp.media_viewer.reveal(item);
    }
}
