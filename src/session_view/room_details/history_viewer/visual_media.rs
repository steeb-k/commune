use adw::{prelude::*, subclass::prelude::*};
use gtk::{glib, glib::clone};
use tracing::error;

use super::{
    HistoryViewerEvent, HistoryViewerEventType, HistoryViewerTimeline, MAX_COLUMNS, MediaAge,
    VisualMediaItem, VisualMediaRow, VisualMediaRowItem, VisualMediaRowModel, n_columns_for_width,
};
use crate::{
    components::LoadingRow,
    prelude::*,
    session_view::MediaViewer,
    spawn,
    utils::{BoundConstructOnlyObject, LoadingState},
};

/// The minimum number of media that should be loaded.
const MIN_N_ITEMS: u32 = 50;

mod imp {
    use std::{
        cell::{Cell, OnceCell},
        ops::ControlFlow,
    };

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate, glib::Properties)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session_view/room_details/history_viewer/visual_media.ui"
    )]
    #[properties(wrapper_type = super::VisualMediaHistoryViewer)]
    pub struct VisualMediaHistoryViewer {
        #[template_child]
        media_viewer: TemplateChild<MediaViewer>,
        #[template_child]
        stack: TemplateChild<gtk::Stack>,
        #[template_child]
        list_view: TemplateChild<gtk::ListView>,
        /// The timeline containing the media events.
        #[property(get, set = Self::set_timeline, construct_only)]
        timeline: BoundConstructOnlyObject<HistoryViewerTimeline>,
        /// The media events of the timeline, with the loading item.
        media: OnceCell<gtk::FilterListModel>,
        /// The media events, as rows split into sections by age.
        rows: OnceCell<VisualMediaRowModel>,
        /// Whether the initial load has settled.
        ///
        /// Until it has, the view is put back on the most recent media every
        /// time items are added.
        is_initialized: Cell<bool>,
        /// The width of the list view that the rows were built for.
        row_width: Cell<i32>,
        /// Whether the rows are waiting to be rebuilt for a new width.
        is_row_width_queued: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for VisualMediaHistoryViewer {
        const NAME: &'static str = "ContentVisualMediaHistoryViewer";
        type Type = super::VisualMediaHistoryViewer;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            Self::bind_template_callbacks(klass);

            klass.set_css_name("visual-media-history-viewer");
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for VisualMediaHistoryViewer {
        fn constructed(&self) {
            self.parent_constructed();

            self.list_view.set_factory(Some(&Self::row_factory()));
            self.list_view
                .set_header_factory(Some(&Self::heading_factory()));
        }
    }

    impl WidgetImpl for VisualMediaHistoryViewer {
        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            self.parent_size_allocate(width, height, baseline);
            self.update_n_columns();
        }

        fn map(&self) {
            self.parent_map();

            // The headings are relative to the current date, so they are stale if the
            // viewer was opened before midnight and is opened again after it.
            if let Some(rows) = self.rows.get() {
                rows.refresh();
            }
        }
    }

    impl NavigationPageImpl for VisualMediaHistoryViewer {}

    #[gtk::template_callbacks]
    impl VisualMediaHistoryViewer {
        /// Construct the factory for the rows of media.
        fn row_factory() -> gtk::SignalListItemFactory {
            let factory = gtk::SignalListItemFactory::new();

            factory.connect_setup(|_, list_item| {
                let Some(list_item) = list_item.downcast_ref::<gtk::ListItem>() else {
                    error!("List item factory did not receive a list item: {list_item:?}");
                    return;
                };

                // The media are activated individually, the row is only a container.
                list_item.set_activatable(false);
                list_item.set_selectable(false);
                list_item.set_child(Some(&VisualMediaRowItem::new()));
            });

            factory.connect_bind(|_, list_item| {
                let Some(list_item) = list_item.downcast_ref::<gtk::ListItem>() else {
                    error!("List item factory did not receive a list item: {list_item:?}");
                    return;
                };

                let row_item = list_item.child_or_default::<VisualMediaRowItem>();
                row_item.set_row(list_item.item().and_downcast::<VisualMediaRow>());
            });

            factory
        }

        /// Construct the factory for the headings above the sections.
        fn heading_factory() -> gtk::SignalListItemFactory {
            let factory = gtk::SignalListItemFactory::new();

            factory.connect_setup(|_, list_header| {
                let Some(list_header) = list_header.downcast_ref::<gtk::ListHeader>() else {
                    error!("Header factory did not receive a list header: {list_header:?}");
                    return;
                };

                let label = gtk::Label::builder()
                    .xalign(0.0)
                    .ellipsize(gtk::pango::EllipsizeMode::End)
                    .css_classes(["heading"])
                    .build();

                list_header.set_child(Some(&label));
            });

            factory.connect_bind(|_, list_header| {
                let Some(list_header) = list_header.downcast_ref::<gtk::ListHeader>() else {
                    error!("Header factory did not receive a list header: {list_header:?}");
                    return;
                };
                let Some(label) = list_header.child().and_downcast::<gtk::Label>() else {
                    return;
                };

                let age = list_header
                    .item()
                    .and_downcast::<VisualMediaRow>()
                    .and_then(|row| row.age());

                // The loading item has no age when there is no media before it, and it
                // should not be given a heading of its own.
                label.set_visible(age.is_some());
                label.set_label(&age.map(MediaAge::label).unwrap_or_default());
            });

            factory
        }

        /// The media events of the timeline, with the loading item.
        fn media(&self) -> &gtk::FilterListModel {
            self.media.get_or_init(|| {
                let filter = gtk::CustomFilter::new(|obj| {
                    obj.downcast_ref::<HistoryViewerEvent>()
                        .is_some_and(|event| event.event_type() == HistoryViewerEventType::Media)
                        || obj.is::<LoadingRow>()
                });

                gtk::FilterListModel::new(None::<gtk::FilterListModel>, Some(filter))
            })
        }

        /// Set the timeline containing the media events.
        fn set_timeline(&self, timeline: HistoryViewerTimeline) {
            let media = self.media();
            media.set_model(Some(timeline.with_loading_item()));

            let rows = self.rows.get_or_init(|| {
                VisualMediaRowModel::new(|item| {
                    item.downcast_ref::<HistoryViewerEvent>()
                        .map(HistoryViewerEvent::timestamp)
                })
            });
            // The width of the view is not known until it has been allocated, and the
            // rows are built before that. The view is clamped to a width that fits the
            // largest number of columns, so that is the value that is the least likely
            // to need a correction.
            rows.set_n_columns(MAX_COLUMNS);
            rows.set_model(Some(media.clone()));

            rows.connect_items_changed(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_, _, _, _| {
                    imp.update_state();

                    if !imp.is_initialized.get() {
                        imp.scroll_to_start();
                    }
                }
            ));
            self.list_view
                .set_model(Some(&gtk::NoSelection::new(Some(rows.clone()))));

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
            self.load_more_items().await;
            self.scroll_to_start();
            self.is_initialized.set(true);

            let adj = self
                .list_view
                .vadjustment()
                .expect("GtkListView has a vadjustment");

            let load_more = clone!(
                #[weak(rename_to = imp)]
                self,
                move |_: &gtk::Adjustment| {
                    if imp.needs_more_items() {
                        spawn!(async move {
                            imp.load_more_items().await;
                        });
                    }
                }
            );
            // The upper bound is watched as well as the value: until the list view has
            // been allocated there is no way to tell whether the viewport is full, so
            // its allocation is what makes the question answerable.
            adj.connect_value_notify(load_more.clone());
            adj.connect_upper_notify(load_more);
        }

        /// Load more items in this viewer.
        #[template_callback]
        async fn load_more_items(&self) {
            self.timeline
                .obj()
                .load(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    #[upgrade_or]
                    ControlFlow::Break(()),
                    move || {
                        if imp.needs_more_items() {
                            ControlFlow::Continue(())
                        } else {
                            ControlFlow::Break(())
                        }
                    }
                ))
                .await;
        }

        /// Rebuild the rows for the current width of the list view.
        fn update_n_columns(&self) {
            let width = self.list_view.width();

            if width <= 0 || width == self.row_width.get() {
                return;
            }
            self.row_width.set(width);

            if self.is_row_width_queued.replace(true) {
                return;
            }

            // The rows cannot be rebuilt while the view presenting them is being
            // allocated, so it is done as soon as the allocation is over.
            glib::idle_add_local_once(clone!(
                #[weak(rename_to = imp)]
                self,
                move || {
                    imp.is_row_width_queued.set(false);

                    if let Some(rows) = imp.rows.get() {
                        rows.set_n_columns(n_columns_for_width(imp.row_width.get()));
                    }
                }
            ));
        }

        /// Scroll back to the most recent media.
        ///
        /// The list view anchors its scroll position on one of its rows, and
        /// the loading row is the last one, so filling the timeline for
        /// the first time drags the view down with it and the most recent
        /// media ends up off-screen.
        fn scroll_to_start(&self) {
            let Some(model) = self.list_view.model() else {
                return;
            };

            if model.n_items() == 0 {
                return;
            }

            // Wait until the next tick, to make sure that the GtkListView has created
            // the row before scrolling to it.
            glib::idle_add_local_once(clone!(
                #[weak(rename_to = imp)]
                self,
                move || {
                    imp.list_view
                        .scroll_to(0, gtk::ListScrollFlags::FOCUS, None);
                }
            ));
        }

        /// Whether this viewer needs more items.
        fn needs_more_items(&self) -> bool {
            // Make sure there is an initial number of media.
            if self.media().n_items() < MIN_N_ITEMS {
                return true;
            }

            let adj = self
                .list_view
                .vadjustment()
                .expect("GtkListView has a vadjustment");

            if adj.upper() <= 0.0 {
                // The list view has not been allocated yet, so the viewport size and
                // the content size are both unknown. Answering `true` here asks for
                // the whole room to be paginated before anything is on screen.
                return false;
            }

            adj.value() + adj.page_size() * 2.0 >= adj.upper()
        }

        /// Update this viewer for the current state.
        fn update_state(&self) {
            let media = self.media();
            let timeline = self.timeline.obj();

            let visible_child_name = match timeline.state() {
                LoadingState::Initial => "loading",
                LoadingState::Error => "error",
                LoadingState::Ready if media.n_items() == 0 => "empty",
                LoadingState::Loading => {
                    if media.n_items() == 0
                        || (media.n_items() == 1
                            && media.item(0).is_some_and(|item| item.is::<LoadingRow>()))
                    {
                        "loading"
                    } else {
                        "content"
                    }
                }
                LoadingState::Ready => "content",
            };
            self.stack.set_visible_child_name(visible_child_name);
        }

        /// Show the given media item in the media viewer.
        pub(super) fn show_media_viewer(&self, item: &VisualMediaItem) {
            let Some(event) = item.event() else {
                return;
            };
            let Some(room) = event.room() else {
                return;
            };

            let media_message = event
                .visual_media_message()
                .expect("visual media items should contain only visual message content");
            self.media_viewer
                .set_message(&room, media_message, Some(event.event_id()));
            self.media_viewer.reveal(item);
        }
    }
}

glib::wrapper! {
    /// A view presenting the list of visual media (image or video) events in a room.
    pub struct VisualMediaHistoryViewer(ObjectSubclass<imp::VisualMediaHistoryViewer>)
        @extends gtk::Widget, adw::NavigationPage,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl VisualMediaHistoryViewer {
    pub fn new(timeline: &HistoryViewerTimeline) -> Self {
        glib::Object::builder()
            .property("timeline", timeline)
            .build()
    }

    /// Show the given media item in the media viewer.
    pub(crate) fn show_media_viewer(&self, item: &VisualMediaItem) {
        self.imp().show_media_viewer(item);
    }
}
