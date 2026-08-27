use adw::{prelude::*, subclass::prelude::*};
use gtk::{gio, glib, glib::clone};
use tracing::error;

mod row;

use self::row::RoomHistoryPinnedRow;
use crate::{
    prelude::*,
    session::{Event, Room, Timeline},
    utils::{BoundObject, LoadingState},
};

mod imp {
    use std::{
        cell::{OnceCell, RefCell},
        sync::LazyLock,
    };

    use glib::subclass::{InitializingObject, Signal};

    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/session_view/room_history/pinned/mod.ui")]
    #[properties(wrapper_type = super::RoomHistoryPinned)]
    pub struct RoomHistoryPinned {
        #[template_child]
        stack: TemplateChild<gtk::Stack>,
        #[template_child]
        list_view: TemplateChild<gtk::ListView>,
        /// The room the pinned messages of which are presented.
        #[property(get, set = Self::set_room, explicit_notify, nullable)]
        room: RefCell<Option<Room>>,
        /// The timeline of the pinned events of the current room.
        timeline: BoundObject<Timeline>,
        /// The list of pinned events, without the virtual items the SDK adds.
        events: OnceCell<gtk::FilterListModel>,
        /// The handler watching the list of pinned events.
        items_changed_handler: RefCell<Option<glib::SignalHandlerId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RoomHistoryPinned {
        const NAME: &'static str = "RoomHistoryPinned";
        type Type = super::RoomHistoryPinned;
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
    impl ObjectImpl for RoomHistoryPinned {
        fn signals() -> &'static [Signal] {
            static SIGNALS: LazyLock<Vec<Signal>> = LazyLock::new(|| {
                vec![
                    Signal::builder("event-activated")
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
            self.timeline.disconnect_signals();
            self.disconnect_list();
        }
    }

    impl WidgetImpl for RoomHistoryPinned {
        fn map(&self) {
            self.parent_map();

            // A GtkStack maps only its visible child, so this is the first time
            // anybody has asked to see the pinned messages of this room. Most
            // rooms are opened and never pinned in; building their pinned
            // timeline on the way past would be a timeline each, for nothing.
            self.ensure_timeline();
        }
    }
    impl BinImpl for RoomHistoryPinned {}

    #[gtk::template_callbacks]
    impl RoomHistoryPinned {
        /// The list of pinned events, without the virtual items the SDK adds.
        fn events(&self) -> &gtk::FilterListModel {
            self.events.get_or_init(|| {
                // A pinned timeline carries date dividers like any other. They
                // say nothing here, where every row already shows its date.
                let filter = gtk::CustomFilter::new(ObjectExt::is::<Event>);
                gtk::FilterListModel::new(None::<gio::ListModel>, Some(filter))
            })
        }

        /// Initialize the list view.
        fn init_list_view(&self) {
            let factory = gtk::SignalListItemFactory::new();
            factory.connect_bind(|_, list_item| {
                let Some(list_item) = list_item.downcast_ref::<gtk::ListItem>() else {
                    error!("List item factory did not receive a list item: {list_item:?}");
                    return;
                };

                let Some(event) = list_item.item().and_downcast::<Event>() else {
                    error!("List item factory did not receive a pinned event");
                    list_item.set_child(None::<&gtk::Widget>);
                    return;
                };

                let child = list_item.child_or_default::<RoomHistoryPinnedRow>();
                child.set_event(Some(event));
            });
            self.list_view.set_factory(Some(&factory));

            let events = self.events().clone();
            let items_changed_handler = events.connect_items_changed(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_, _, _, _| {
                    imp.update_view();
                }
            ));
            self.items_changed_handler
                .replace(Some(items_changed_handler));

            self.list_view
                .set_model(Some(&gtk::NoSelection::new(Some(events))));
        }

        /// Disconnect the handler watching the list of pinned events.
        fn disconnect_list(&self) {
            if let Some(handler) = self.items_changed_handler.take() {
                self.events().disconnect(handler);
            }
        }

        /// Set the room the pinned messages of which are presented.
        fn set_room(&self, room: Option<Room>) {
            if *self.room.borrow() == room {
                return;
            }

            self.timeline.disconnect_signals();
            self.events().set_model(None::<&gio::ListModel>);
            self.room.replace(room);

            if self.obj().is_mapped() {
                self.ensure_timeline();
            }

            self.update_view();
            self.obj().notify_room();
        }

        /// Build the timeline of the pinned events of the current room, if it
        /// is not built already.
        fn ensure_timeline(&self) {
            if self.timeline.obj().is_some() {
                return;
            }
            let room = self.room.borrow().clone();
            let Some(room) = room else {
                return;
            };

            let timeline = Timeline::new_pinned(&room);
            self.events().set_model(Some(&timeline.items()));

            let state_handler = timeline.connect_state_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_| {
                    imp.update_view();
                }
            ));

            self.timeline.set(timeline, vec![state_handler]);

            self.update_view();
        }

        /// Update the visible page.
        fn update_view(&self) {
            let Some(timeline) = self.timeline.obj() else {
                self.stack.set_visible_child_name("empty");
                return;
            };

            let name = if self.events().n_items() > 0 {
                // Events that arrive after the first ones must not send the
                // list back to the spinner.
                "results"
            } else if matches!(
                timeline.state(),
                LoadingState::Initial | LoadingState::Loading
            ) {
                "loading"
            } else {
                "empty"
            };

            self.stack.set_visible_child_name(name);
        }

        /// Handle the activation of a row.
        #[template_callback]
        fn row_activated(&self, position: u32) {
            let Some(event) = self.events().item(position).and_downcast::<Event>() else {
                error!("Could not find activated pinned event");
                return;
            };
            let Some(event_id) = event.event_id() else {
                error!("Activated pinned event does not have an event ID");
                return;
            };

            self.obj()
                .emit_by_name::<()>("event-activated", &[&event_id.as_str()]);
        }
    }
}

glib::wrapper! {
    /// A view presenting the messages pinned in a room.
    pub struct RoomHistoryPinned(ObjectSubclass<imp::RoomHistoryPinned>)
        @extends gtk::Widget, adw::Bin,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl RoomHistoryPinned {
    /// Connect to the signal emitted when a pinned event is activated.
    pub(crate) fn connect_event_activated<F: Fn(&Self, String) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_closure(
            "event-activated",
            true,
            glib::closure_local!(move |obj: Self, event_id: String| {
                f(&obj, event_id);
            }),
        )
    }
}
