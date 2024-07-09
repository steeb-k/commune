use adw::{prelude::*, subclass::prelude::*};
use gtk::{glib, glib::clone, CompositeTemplate};
use tracing::error;

use super::{AudioRow, HistoryViewerEvent, HistoryViewerEventType, HistoryViewerTimeline};
use crate::{
    components::LoadingRow, session::model::TimelineState, spawn, utils::BoundConstructOnlyObject,
};

/// The minimum number of items that should be loaded.
const MIN_N_ITEMS: u32 = 20;

mod imp {
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
        pub timeline: BoundConstructOnlyObject<HistoryViewerTimeline>,
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub list_view: TemplateChild<gtk::ListView>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for AudioHistoryViewer {
        const NAME: &'static str = "ContentAudioHistoryViewer";
        type Type = super::AudioHistoryViewer;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            Self::Type::bind_template_callbacks(klass);

            klass.set_css_name("audio-history-viewer");
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for AudioHistoryViewer {
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
                    loading_row.set_width_request(-1);
                    loading_row.set_height_request(-1);

                    list_item.set_child(Some(loading_row));
                } else if let Some(event) = item.and_downcast::<HistoryViewerEvent>() {
                    let audio_row =
                        if let Some(audio_row) = list_item.child().and_downcast::<AudioRow>() {
                            audio_row
                        } else {
                            let audio_row = AudioRow::new();
                            list_item.set_child(Some(&audio_row));

                            audio_row
                        };

                    audio_row.set_event(Some(event));
                }
            });

            self.list_view.set_factory(Some(&factory));
        }
    }

    impl WidgetImpl for AudioHistoryViewer {}
    impl NavigationPageImpl for AudioHistoryViewer {}

    impl AudioHistoryViewer {
        /// Set the timeline containing the audio events.
        fn set_timeline(&self, timeline: HistoryViewerTimeline) {
            let filter = gtk::CustomFilter::new(|obj| {
                obj.downcast_ref::<HistoryViewerEvent>()
                    .is_some_and(|e| e.event_type() == HistoryViewerEventType::Audio)
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
            self.list_view.set_model(Some(&model));

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

        /// Initialize the timeline.
        async fn init_timeline(&self) {
            let Some(model) = self.list_view.model() else {
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

            let adj = self.list_view.vadjustment().unwrap();
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
            let Some(model) = self.list_view.model() else {
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
    /// A view presenting the list of audio events in a room.
    pub struct AudioHistoryViewer(ObjectSubclass<imp::AudioHistoryViewer>)
        @extends gtk::Widget, adw::NavigationPage;
}

#[gtk::template_callbacks]
impl AudioHistoryViewer {
    pub fn new(timeline: &HistoryViewerTimeline) -> Self {
        glib::Object::builder()
            .property("timeline", timeline)
            .build()
    }

    /// Load more history.
    #[template_callback]
    async fn load_more(&self) {
        let timeline = self.imp().timeline.obj();
        timeline.load().await;
    }
}
