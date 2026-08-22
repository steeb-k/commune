use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{gio, glib, glib::clone};
use tracing::error;

mod row;

use self::row::RoomHistorySearchRow;
use crate::{
    prelude::*,
    session::{Room, RoomSearch, RoomSearchResult},
    utils::{BoundObject, LoadingState},
};

/// The minimum number of results to load, so that the list can be scrolled.
const MIN_N_RESULTS: u32 = 20;

mod imp {
    use std::{cell::RefCell, sync::LazyLock};

    use glib::subclass::{InitializingObject, Signal};

    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/session_view/room_history/search/mod.ui")]
    #[properties(wrapper_type = super::RoomHistorySearch)]
    pub struct RoomHistorySearch {
        #[template_child]
        stack: TemplateChild<gtk::Stack>,
        #[template_child]
        no_results_page: TemplateChild<adw::StatusPage>,
        #[template_child]
        empty_reindex_button: TemplateChild<gtk::Button>,
        #[template_child]
        no_results_reindex_button: TemplateChild<gtk::Button>,
        #[template_child]
        list_view: TemplateChild<gtk::ListView>,
        /// The room the messages of which are searched.
        #[property(get, set = Self::set_room, explicit_notify, nullable)]
        room: RefCell<Option<Room>>,
        /// The search of the current room.
        search: BoundObject<RoomSearch>,
        /// The list of results and the handler watching its changes.
        list_handler: RefCell<Option<(gio::ListStore, glib::SignalHandlerId)>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RoomHistorySearch {
        const NAME: &'static str = "RoomHistorySearch";
        type Type = super::RoomHistorySearch;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            Self::bind_template_callbacks(klass);

            klass.set_accessible_role(gtk::AccessibleRole::Group);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for RoomHistorySearch {
        fn signals() -> &'static [Signal] {
            static SIGNALS: LazyLock<Vec<Signal>> = LazyLock::new(|| {
                vec![
                    Signal::builder("result-activated")
                        .param_types([String::static_type()])
                        .build(),
                ]
            });
            SIGNALS.as_ref()
        }

        fn constructed(&self) {
            self.parent_constructed();

            self.init_list_view();
        }

        fn dispose(&self) {
            self.search.disconnect_signals();
            self.disconnect_list();
        }
    }

    impl WidgetImpl for RoomHistorySearch {}
    impl BinImpl for RoomHistorySearch {}

    #[gtk::template_callbacks]
    impl RoomHistorySearch {
        /// Initialize the list view.
        fn init_list_view(&self) {
            let factory = gtk::SignalListItemFactory::new();
            factory.connect_bind(|_, list_item| {
                let Some(list_item) = list_item.downcast_ref::<gtk::ListItem>() else {
                    error!("List item factory did not receive a list item: {list_item:?}");
                    return;
                };

                let Some(result) = list_item.item().and_downcast::<RoomSearchResult>() else {
                    error!("List item factory did not receive a search result");
                    list_item.set_child(None::<&gtk::Widget>);
                    return;
                };

                let child = list_item.child_or_default::<RoomHistorySearchRow>();
                child.set_result(Some(result));
            });
            self.list_view.set_factory(Some(&factory));

            let adj = self
                .list_view
                .vadjustment()
                .expect("GtkListView has a vadjustment");
            adj.connect_value_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_| {
                    imp.load_more_results_if_needed();
                }
            ));
            adj.connect_upper_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_| {
                    imp.load_more_results_if_needed();
                }
            ));
        }

        /// Set the room the messages of which are searched.
        fn set_room(&self, room: Option<Room>) {
            if *self.room.borrow() == room {
                return;
            }

            self.search.disconnect_signals();
            self.room.replace(room.clone());

            self.disconnect_list();

            if let Some(room) = room {
                let search = RoomSearch::new(&room);

                let list = search.list();
                let items_changed_handler = list.connect_items_changed(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_, _, _, _| {
                        imp.update_view();
                        imp.load_more_results_if_needed();
                    }
                ));
                self.list_handler
                    .replace(Some((list.clone(), items_changed_handler)));

                let state_handler = search.connect_loading_state_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.update_view();
                        imp.load_more_results_if_needed();
                    }
                ));

                self.list_view
                    .set_model(Some(&gtk::NoSelection::new(Some(list))));

                self.search.set(search, vec![state_handler]);
            } else {
                self.list_view.set_model(None::<&gtk::NoSelection>);
            }

            self.update_view();
            self.obj().notify_room();
        }

        /// Disconnect the signals of the current list of results.
        fn disconnect_list(&self) {
            if let Some((list, handler)) = self.list_handler.take() {
                list.disconnect(handler);
            }
        }

        /// Set the term to search.
        pub(super) fn set_search_term(&self, search_term: &str) {
            let Some(search) = self.search.obj() else {
                return;
            };

            search.set_search_term(search_term);
        }

        /// Load more results if the list is not full enough.
        fn load_more_results_if_needed(&self) {
            let Some(search) = self.search.obj() else {
                return;
            };

            if !search.can_load_more() {
                return;
            }

            if self.needs_more_results() {
                search.load_more();
            }
        }

        /// Whether more results are needed to fill the list.
        fn needs_more_results(&self) -> bool {
            let Some(model) = self.list_view.model() else {
                return false;
            };

            // Make sure there is an initial number of results, so that the list can be
            // scrolled at all.
            if model.n_items() < MIN_N_RESULTS {
                return true;
            }

            let adj = self
                .list_view
                .vadjustment()
                .expect("GtkListView has a vadjustment");
            adj.value() + adj.page_size() * 2.0 >= adj.upper()
        }

        /// Retry the current search after an error.
        #[template_callback]
        fn retry(&self) {
            if let Some(search) = self.search.obj() {
                search.load_more();
            }
        }

        /// Add the messages that are loaded in the room to its search index.
        #[template_callback]
        fn reindex(&self) {
            if let Some(search) = self.search.obj() {
                search.reindex();
            }
        }

        /// Handle the activation of a result.
        #[template_callback]
        fn result_activated(&self, position: u32) {
            let Some(result) = self
                .list_view
                .model()
                .and_then(|model| model.item(position))
                .and_downcast::<RoomSearchResult>()
            else {
                return;
            };

            self.obj().emit_result_activated(result.event_id().as_ref());
        }

        /// Update the view for the current state.
        pub(super) fn update_view(&self) {
            let Some(search) = self.search.obj() else {
                self.stack.set_visible_child_name("empty");
                return;
            };

            if search.search_term().is_empty() {
                // Nothing is searched yet, but the index can still be busy being
                // rebuilt, which is worth showing.
                let visible_child_name = if search.loading_state() == LoadingState::Loading {
                    "loading"
                } else {
                    "empty"
                };
                self.stack.set_visible_child_name(visible_child_name);
                self.update_reindex_buttons();
                return;
            }

            let has_results = !search.is_empty();

            let visible_child_name = match search.loading_state() {
                LoadingState::Error if !has_results => "error",
                LoadingState::Loading | LoadingState::Initial if !has_results => "loading",
                _ if has_results => "results",
                _ => "no-results",
            };

            let is_encrypted = self.update_reindex_buttons();

            if visible_child_name == "no-results" {
                // An encrypted room can only be searched locally, which cannot find
                // messages that this device never received.
                let description = if is_encrypted {
                    gettext(
                        "Messages of encrypted rooms are searched on this device, so messages \
                         received before it joined the room cannot be found",
                    )
                } else {
                    gettext("No message of this room matches the search")
                };
                self.no_results_page.set_description(Some(&description));
            }

            self.stack.set_visible_child_name(visible_child_name);
        }

        /// Update the visibility of the buttons rebuilding the search index,
        /// and return whether the room is encrypted.
        ///
        /// Only the local index can be added to, and it is only used for an
        /// encrypted room. Searching any other room goes to the server, which
        /// needs nothing from this device.
        fn update_reindex_buttons(&self) -> bool {
            let is_encrypted = self.room.borrow().as_ref().is_some_and(Room::is_encrypted);

            self.empty_reindex_button.set_visible(is_encrypted);
            self.no_results_reindex_button.set_visible(is_encrypted);

            is_encrypted
        }
    }
}

glib::wrapper! {
    /// A view presenting the messages of a room matching a search.
    pub struct RoomHistorySearch(ObjectSubclass<imp::RoomHistorySearch>)
        @extends gtk::Widget, adw::Bin,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl RoomHistorySearch {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Set the term to search.
    pub(crate) fn set_search_term(&self, search_term: &str) {
        self.imp().set_search_term(search_term);
    }

    /// Emit the signal that a result was activated.
    fn emit_result_activated(&self, event_id: &str) {
        self.emit_by_name::<()>("result-activated", &[&event_id]);
    }

    /// Connect to the signal emitted when a result is activated.
    pub(crate) fn connect_result_activated<F: Fn(&Self, String) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_closure(
            "result-activated",
            true,
            glib::closure_local!(move |obj: Self, event_id: String| {
                f(&obj, event_id);
            }),
        )
    }
}

impl Default for RoomHistorySearch {
    fn default() -> Self {
        Self::new()
    }
}
