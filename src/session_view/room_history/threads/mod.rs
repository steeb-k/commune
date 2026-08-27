use adw::{prelude::*, subclass::prelude::*};
use gtk::{gio, glib, glib::clone};
use tracing::error;

mod row;

use self::row::RoomHistoryThreadsRow;
use crate::{
    prelude::*,
    session::{Room, ThreadList, ThreadListEntry},
    utils::{BoundObject, LoadingState},
};

/// The minimum number of threads to load, so that the list can be scrolled.
const MIN_N_THREADS: u32 = 20;

mod imp {
    use std::{cell::RefCell, sync::LazyLock};

    use glib::subclass::{InitializingObject, Signal};

    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/session_view/room_history/threads/mod.ui")]
    #[properties(wrapper_type = super::RoomHistoryThreads)]
    pub struct RoomHistoryThreads {
        #[template_child]
        stack: TemplateChild<gtk::Stack>,
        #[template_child]
        list_view: TemplateChild<gtk::ListView>,
        /// The room the threads of which are presented.
        #[property(get, set = Self::set_room, explicit_notify, nullable)]
        room: RefCell<Option<Room>>,
        /// The thread list of the current room.
        thread_list: BoundObject<ThreadList>,
        /// The list of threads and the handler watching its changes.
        list_handler: RefCell<Option<(gio::ListStore, glib::SignalHandlerId)>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RoomHistoryThreads {
        const NAME: &'static str = "RoomHistoryThreads";
        type Type = super::RoomHistoryThreads;
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
    impl ObjectImpl for RoomHistoryThreads {
        fn signals() -> &'static [Signal] {
            static SIGNALS: LazyLock<Vec<Signal>> = LazyLock::new(|| {
                vec![
                    Signal::builder("thread-activated")
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
            self.thread_list.disconnect_signals();
            self.disconnect_list();
        }
    }

    impl WidgetImpl for RoomHistoryThreads {
        fn map(&self) {
            self.parent_map();

            // A GtkStack maps only its visible child, so this runs each time
            // somebody asks to see the threads of this room — never on the way
            // past, which would be a request per room for nothing. The list is
            // rebuilt rather than kept: the SDK service only rewrites the
            // threads it has already fetched, so a thread rooted since the
            // last look would otherwise never join the list.
            self.rebuild_thread_list();
        }
    }
    impl BinImpl for RoomHistoryThreads {}

    #[gtk::template_callbacks]
    impl RoomHistoryThreads {
        /// Initialize the list view.
        fn init_list_view(&self) {
            let factory = gtk::SignalListItemFactory::new();
            factory.connect_bind(|_, list_item| {
                let Some(list_item) = list_item.downcast_ref::<gtk::ListItem>() else {
                    error!("List item factory did not receive a list item: {list_item:?}");
                    return;
                };

                let Some(entry) = list_item.item().and_downcast::<ThreadListEntry>() else {
                    error!("List item factory did not receive a thread");
                    list_item.set_child(None::<&gtk::Widget>);
                    return;
                };

                let child = list_item.child_or_default::<RoomHistoryThreadsRow>();
                child.set_entry(Some(entry));
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
                    imp.load_more_threads_if_needed();
                }
            ));
            adj.connect_upper_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_| {
                    imp.load_more_threads_if_needed();
                }
            ));
        }

        /// Set the room the threads of which are presented.
        fn set_room(&self, room: Option<Room>) {
            if *self.room.borrow() == room {
                return;
            }

            self.thread_list.disconnect_signals();
            self.disconnect_list();
            self.list_view.set_model(None::<&gtk::NoSelection>);
            self.room.replace(room);

            if self.obj().is_mapped() {
                self.rebuild_thread_list();
            }

            self.update_view();
            self.obj().notify_room();
        }

        /// Disconnect the signals of the current list of threads.
        fn disconnect_list(&self) {
            if let Some((list, handler)) = self.list_handler.take() {
                list.disconnect(handler);
            }
        }

        /// Build the thread list of the current room from a fresh request,
        /// dropping any list built on an earlier look, and start loading it.
        fn rebuild_thread_list(&self) {
            self.thread_list.disconnect_signals();
            self.disconnect_list();
            self.list_view.set_model(None::<&gtk::NoSelection>);

            let room = self.room.borrow().clone();
            let Some(room) = room else {
                return;
            };

            let thread_list = ThreadList::new(&room);

            let list = thread_list.list();
            let items_changed_handler = list.connect_items_changed(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_, _, _, _| {
                    imp.update_view();
                    imp.load_more_threads_if_needed();
                }
            ));
            self.list_handler
                .replace(Some((list.clone(), items_changed_handler)));

            let state_handler = thread_list.connect_loading_state_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_| {
                    imp.update_view();
                    imp.load_more_threads_if_needed();
                }
            ));

            self.list_view
                .set_model(Some(&gtk::NoSelection::new(Some(list))));

            thread_list.load_more();
            self.thread_list.set(thread_list, vec![state_handler]);

            self.update_view();
        }

        /// Load more threads if the list is not full enough.
        fn load_more_threads_if_needed(&self) {
            let Some(thread_list) = self.thread_list.obj() else {
                return;
            };

            if !thread_list.can_load_more() {
                return;
            }

            if self.needs_more_threads() {
                thread_list.load_more();
            }
        }

        /// Whether more threads are needed to fill the list.
        fn needs_more_threads(&self) -> bool {
            let Some(model) = self.list_view.model() else {
                return false;
            };

            // Make sure there is an initial number of threads, so that the
            // list can be scrolled at all.
            if model.n_items() < MIN_N_THREADS {
                return true;
            }

            let adj = self
                .list_view
                .vadjustment()
                .expect("GtkListView has a vadjustment");
            adj.value() + adj.page_size() * 2.0 >= adj.upper()
        }

        /// Retry loading the threads after an error.
        #[template_callback]
        fn retry(&self) {
            if let Some(thread_list) = self.thread_list.obj() {
                thread_list.load_more();
            }
        }

        /// Handle the activation of a row.
        #[template_callback]
        fn row_activated(&self, position: u32) {
            let Some(entry) = self
                .list_view
                .model()
                .and_then(|model| model.item(position))
                .and_downcast::<ThreadListEntry>()
            else {
                error!("Could not find activated thread");
                return;
            };

            self.obj()
                .emit_by_name::<()>("thread-activated", &[&entry.root_event_id().as_str()]);
        }

        /// Update the visible page.
        fn update_view(&self) {
            let Some(thread_list) = self.thread_list.obj() else {
                self.stack.set_visible_child_name("loading");
                return;
            };

            let has_threads = !thread_list.is_empty();

            let name = match thread_list.loading_state() {
                // Threads that arrive after the first ones must not send the
                // list back to the spinner.
                _ if has_threads => "results",
                LoadingState::Error => "error",
                LoadingState::Loading | LoadingState::Initial => "loading",
                LoadingState::Ready => "empty",
            };

            self.stack.set_visible_child_name(name);
        }
    }
}

glib::wrapper! {
    /// A view presenting the threads of a room.
    pub struct RoomHistoryThreads(ObjectSubclass<imp::RoomHistoryThreads>)
        @extends gtk::Widget, adw::Bin,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl RoomHistoryThreads {
    /// Connect to the signal emitted when a thread is activated.
    pub(crate) fn connect_thread_activated<F: Fn(&Self, String) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_closure(
            "thread-activated",
            true,
            glib::closure_local!(move |obj: Self, root_event_id: String| {
                f(&obj, root_event_id);
            }),
        )
    }
}
