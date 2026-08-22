use std::time::Duration;

use adw::subclass::prelude::*;
use gtk::{
    CompositeTemplate, glib,
    glib::{clone, closure_local},
    prelude::*,
};
use tracing::warn;

use super::gif_button::GifButton;
use crate::{
    spawn, spawn_tokio,
    utils::klipy::{self, Gif, SelectedGif},
};

/// How long we wait after the last keystroke before searching.
///
/// Every search is a request to a third party, so this is longer than a
/// local filter would need.
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(500);
/// How close to the bottom of the results the view must be, in pixels, before
/// the next page is requested.
const LOAD_MORE_THRESHOLD: f64 = 200.0;
/// How many previews are downloaded at a time.
///
/// About a row's worth. The host that serves them is slow, and asking it for a
/// whole page at once means the whole page arrives at the end rather than a
/// row at a time.
const CONCURRENT_PREVIEW_LOADS: usize = 4;

mod imp {
    use std::{
        cell::{Cell, RefCell},
        collections::{HashSet, VecDeque},
        sync::LazyLock,
    };

    use glib::subclass::Signal;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session_view/room_history/message_toolbar/sticker_picker/gif_page.ui"
    )]
    pub struct GifPage {
        #[template_child]
        search_entry: TemplateChild<gtk::SearchEntry>,
        #[template_child]
        stack: TemplateChild<gtk::Stack>,
        #[template_child]
        scrolled_window: TemplateChild<gtk::ScrolledWindow>,
        #[template_child]
        flow_box: TemplateChild<gtk::FlowBox>,
        #[template_child]
        load_more_spinner: TemplateChild<gtk::Spinner>,
        /// The search that the presented results are for.
        ///
        /// Empty means that nothing has been searched for yet.
        query: RefCell<String>,
        /// The last page that was loaded, 1-indexed. Zero means none.
        page: Cell<u32>,
        /// Whether the API said that there is another page to load.
        has_next: Cell<bool>,
        /// Whether a request is in flight.
        is_loading: Cell<bool>,
        /// The identifier of the current search.
        ///
        /// It is bumped every time the query changes, so that the answer to a
        /// request that was overtaken by a newer one is discarded.
        generation: Cell<u64>,
        /// The pending debounce of the search entry.
        search_timeout: RefCell<Option<glib::SourceId>>,
        /// The identifiers of the GIFs that are already presented.
        ///
        /// The API can repeat a GIF between two pages, and a repeat is jarring
        /// when scrolling.
        presented: RefCell<HashSet<i64>>,
        /// The buttons whose preview has not been downloaded yet.
        pending_previews: RefCell<VecDeque<GifButton>>,
        /// How many previews are being downloaded right now.
        previews_in_flight: Cell<usize>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for GifPage {
        const NAME: &'static str = "GifPage";
        type Type = super::GifPage;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            Self::bind_template_callbacks(klass);
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for GifPage {
        fn signals() -> &'static [Signal] {
            static SIGNALS: LazyLock<Vec<Signal>> = LazyLock::new(|| {
                vec![
                    Signal::builder("gif-selected")
                        .param_types([SelectedGif::static_type()])
                        .build(),
                ]
            });
            SIGNALS.as_ref()
        }

        fn constructed(&self) {
            self.parent_constructed();

            // Load the next page as the bottom of the results comes into view.
            self.scrolled_window
                .vadjustment()
                .connect_value_changed(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |adjustment| {
                        let remaining =
                            adjustment.upper() - adjustment.value() - adjustment.page_size();

                        if remaining <= LOAD_MORE_THRESHOLD {
                            imp.load_more();
                        }
                    }
                ));
        }

        fn dispose(&self) {
            if let Some(source) = self.search_timeout.take() {
                source.remove();
            }
        }
    }

    impl WidgetImpl for GifPage {}

    impl BinImpl for GifPage {}

    #[gtk::template_callbacks]
    impl GifPage {
        /// Handle a change of the search entry, debounced.
        #[template_callback]
        fn search_changed(&self) {
            if let Some(source) = self.search_timeout.take() {
                source.remove();
            }

            let query = self.search_entry.text().trim().to_owned();
            if *self.query.borrow() == query {
                return;
            }

            // Bump the generation right away so that the answers to the requests
            // of the previous query are discarded even before the new one starts.
            self.generation.set(self.generation.get() + 1);

            if query.is_empty() {
                // Nothing is asked of the service until the user searches for
                // something.
                self.query.replace(query);
                self.clear();
                self.stack.set_visible_child_name("start");
                return;
            }

            let source = glib::timeout_add_local_once(
                SEARCH_DEBOUNCE,
                clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move || {
                        imp.search_timeout.take();
                        imp.query.replace(query);
                        imp.reload();
                    }
                ),
            );
            self.search_timeout.replace(Some(source));
        }

        /// Load the current query again, from its first page.
        #[template_callback]
        fn retry(&self) {
            self.reload();
        }

        /// Forget the presented results and load the first page of the current
        /// query.
        fn reload(&self) {
            if self.query.borrow().is_empty() {
                self.clear();
                self.stack.set_visible_child_name("start");
                return;
            }

            self.generation.set(self.generation.get() + 1);
            self.page.set(0);
            self.has_next.set(true);
            self.is_loading.set(false);
            self.clear();
            self.stack.set_visible_child_name("loading");

            self.load_next_page();
        }

        /// Load the next page, if there is one and we are not already loading.
        fn load_more(&self) {
            if self.page.get() == 0 {
                // The first page has not landed yet; it will not be skipped.
                return;
            }

            self.load_next_page();
        }

        /// Load the page that follows the last one that was loaded.
        fn load_next_page(&self) {
            if self.is_loading.get() || !self.has_next.get() {
                return;
            }

            self.is_loading.set(true);
            self.load_more_spinner.set_visible(self.page.get() > 0);

            let generation = self.generation.get();
            let query = self.query.borrow().clone();
            let page = self.page.get() + 1;

            spawn!(clone!(
                #[weak(rename_to = imp)]
                self,
                async move {
                    let handle = spawn_tokio!(async move { klipy::search(&query, page).await });
                    let result = handle.await.expect("task should not be aborted");

                    // A newer query was started while this one was in flight.
                    if imp.generation.get() != generation {
                        return;
                    }

                    imp.is_loading.set(false);
                    imp.load_more_spinner.set_visible(false);

                    match result {
                        Ok(page_result) => imp.present(page, page_result),
                        Err(error) => {
                            warn!("Could not load GIFs: {error}");

                            // A failure to load more results does not throw away the
                            // results that are already presented.
                            if imp.page.get() == 0 {
                                imp.stack.set_visible_child_name("error");
                            }
                        }
                    }
                }
            ));
        }

        /// Present the given page of results.
        fn present(&self, page: u32, result: klipy::GifPage) {
            self.page.set(page);
            self.has_next.set(result.has_next);

            {
                let mut presented = self.presented.borrow_mut();
                let mut pending = self.pending_previews.borrow_mut();

                for gif in &result.gifs {
                    if presented.insert(gif.id) {
                        let button = self.build_gif(gif);
                        self.flow_box.append(&button);
                        pending.push_back(button);
                    }
                }
            }

            self.pump_previews();

            if self.flow_box.first_child().is_none() {
                // The API can answer with a page that held nothing but sponsored
                // items, which we drop. Ask for the next one rather than claiming
                // that there is nothing.
                if result.has_next {
                    self.load_next_page();
                } else {
                    self.stack.set_visible_child_name("empty");
                }

                return;
            }

            self.stack.set_visible_child_name("results");
        }

        /// Build the button for the given GIF.
        fn build_gif(&self, gif: &Gif) -> GifButton {
            let button = GifButton::new(gif);

            button.connect_clicked(clone!(
                #[weak(rename_to = imp)]
                self,
                move |button| {
                    let Some(selection) = button.gif().to_selection() else {
                        return;
                    };

                    imp.obj().emit_by_name::<()>("gif-selected", &[&selection]);
                }
            ));

            button
        }

        /// Download the previews that are still pending, a few at a time.
        ///
        /// Each one that lands starts the next, so the results fill in
        /// progressively instead of appearing all at once at the end.
        fn pump_previews(&self) {
            while self.previews_in_flight.get() < CONCURRENT_PREVIEW_LOADS {
                let Some(button) = self.pending_previews.borrow_mut().pop_front() else {
                    return;
                };

                self.previews_in_flight
                    .set(self.previews_in_flight.get() + 1);

                let generation = self.generation.get();
                spawn!(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    async move {
                        button.load().await;

                        // These results were thrown away while this was downloading.
                        if imp.generation.get() != generation {
                            return;
                        }

                        imp.previews_in_flight
                            .set(imp.previews_in_flight.get().saturating_sub(1));
                        imp.pump_previews();
                    }
                ));
            }
        }

        /// Remove the presented results.
        fn clear(&self) {
            self.presented.borrow_mut().clear();
            self.pending_previews.borrow_mut().clear();
            self.previews_in_flight.set(0);

            while let Some(child) = self.flow_box.first_child() {
                self.flow_box.remove(&child);
            }
        }
    }
}

glib::wrapper! {
    /// The GIF search of the sticker picker.
    pub struct GifPage(ObjectSubclass<imp::GifPage>)
        @extends gtk::Widget, adw::Bin,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl GifPage {
    /// Connect to the signal emitted when a GIF is selected.
    pub(crate) fn connect_gif_selected<F: Fn(&Self, SelectedGif) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_closure(
            "gif-selected",
            true,
            closure_local!(move |obj: Self, gif: SelectedGif| {
                f(&obj, gif);
            }),
        )
    }
}
