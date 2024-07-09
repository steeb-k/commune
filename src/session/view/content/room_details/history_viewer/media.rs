use adw::{prelude::*, subclass::prelude::*};
use gtk::{glib, glib::clone, CompositeTemplate};
use tracing::error;

use super::{HistoryViewerEvent, HistoryViewerEventType, HistoryViewerTimeline, MediaItem};
use crate::{
    components::LoadingRow,
    session::{model::TimelineState, view::MediaViewer},
    spawn,
    utils::BoundConstructOnlyObject,
};

/// The minimum number of items that should be loaded.
const MIN_N_ITEMS: u32 = 50;
/// The minimum size requested by an item.
const SIZE_REQUEST: i32 = 150;

mod imp {
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
        pub timeline: BoundConstructOnlyObject<HistoryViewerTimeline>,
        #[template_child]
        pub media_viewer: TemplateChild<MediaViewer>,
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub grid_view: TemplateChild<gtk::GridView>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MediaHistoryViewer {
        const NAME: &'static str = "ContentMediaHistoryViewer";
        type Type = super::MediaHistoryViewer;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            Self::Type::bind_template_callbacks(klass);

            klass.set_css_name("media-history-viewer");
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for MediaHistoryViewer {
        fn constructed(&self) {
            self.parent_constructed();

            let factory = gtk::SignalListItemFactory::new();

            factory.connect_bind(move |_, list_item| {
                let Some(list_item) = list_item.downcast_ref::<gtk::ListItem>() else {
                    error!("List item factory did not receive a list item: {list_item:?}");
                    return;
                };

                list_item.set_activatable(false);
                list_item.set_selectable(false);
            });
            factory.connect_bind(move |_, list_item| {
                let Some(list_item) = list_item.downcast_ref::<gtk::ListItem>() else {
                    error!("List item factory did not receive a list item: {list_item:?}");
                    return;
                };

                let item = list_item.item();

                if let Some(loading_row) = item
                    .and_downcast_ref::<LoadingRow>()
                    .filter(|_| !list_item.child().is_some_and(|c| c.is::<LoadingRow>()))
                {
                    loading_row.unparent();
                    loading_row.set_width_request(SIZE_REQUEST);
                    loading_row.set_height_request(SIZE_REQUEST);

                    list_item.set_child(Some(loading_row));
                } else if let Some(event) = item.and_downcast::<HistoryViewerEvent>() {
                    let media_item =
                        if let Some(media_item) = list_item.child().and_downcast::<MediaItem>() {
                            media_item
                        } else {
                            let media_item = MediaItem::new();
                            media_item.set_width_request(SIZE_REQUEST);
                            media_item.set_height_request(SIZE_REQUEST);

                            list_item.set_child(Some(&media_item));

                            media_item
                        };

                    media_item.set_event(Some(event));
                }
            });

            self.grid_view.set_factory(Some(&factory));
        }
    }

    impl WidgetImpl for MediaHistoryViewer {}
    impl NavigationPageImpl for MediaHistoryViewer {}

    impl MediaHistoryViewer {
        /// Set the timeline containing the media events.
        fn set_timeline(&self, timeline: HistoryViewerTimeline) {
            let filter = gtk::CustomFilter::new(|obj| {
                obj.downcast_ref::<HistoryViewerEvent>()
                    .is_some_and(|e| e.event_type() == HistoryViewerEventType::Media)
                    || obj.is::<LoadingRow>()
            });
            let filter_model =
                gtk::FilterListModel::new(Some(timeline.with_loading_item().clone()), Some(filter));

            let model = gtk::NoSelection::new(Some(filter_model));
            model.connect_items_changed(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_, _, _, _| {
                    imp.update_state();
                }
            ));
            self.grid_view.set_model(Some(&model));

            let timeline_state_handler = timeline.connect_state_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_| {
                    imp.update_state();
                }
            ));
            self.timeline.set(timeline, vec![timeline_state_handler]);
            self.update_state();

            spawn!(clone!(
                #[weak(rename_to = imp)]
                self,
                async move {
                    imp.init_timeline().await;
                }
            ));
        }

        /// Initialize the timeline
        async fn init_timeline(&self) {
            let Some(model) = self.grid_view.model() else {
                return;
            };
            let timeline = self.timeline.obj();
            let obj = self.obj();

            // Load an initial number of items.
            while model.n_items() < MIN_N_ITEMS {
                if !timeline.load().await {
                    break;
                }
            }

            let adj = self.grid_view.vadjustment().unwrap();
            adj.connect_value_notify(clone!(
                #[weak]
                obj,
                move |adj| {
                    if adj.value() + adj.page_size() * 2.0 >= adj.upper() {
                        spawn!(async move {
                            obj.load_more().await;
                        });
                    }
                }
            ));
        }

        /// Update this viewer for the current state.
        fn update_state(&self) {
            let Some(model) = self.grid_view.model() else {
                return;
            };
            let timeline = self.timeline.obj();

            match timeline.state() {
                TimelineState::Initial | TimelineState::Loading if model.n_items() == 0 => {
                    self.stack.set_visible_child_name("loading");
                }
                TimelineState::Error => {
                    self.stack.set_visible_child_name("error");
                }
                TimelineState::Complete if model.n_items() == 0 => {
                    self.stack.set_visible_child_name("empty");
                }
                _ => {
                    self.stack.set_visible_child_name("content");
                }
            }
        }
    }
}

glib::wrapper! {
    /// A view presenting the list of media (image or video) events in a room.
    pub struct MediaHistoryViewer(ObjectSubclass<imp::MediaHistoryViewer>)
        @extends gtk::Widget, adw::NavigationPage;
}

#[gtk::template_callbacks]
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

    /// Load more history.
    #[template_callback]
    async fn load_more(&self) {
        let timeline = self.imp().timeline.obj();
        timeline.load().await;
    }
}
