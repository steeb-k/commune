use adw::subclass::prelude::*;
use gtk::{glib, glib::clone, prelude::*, CompositeTemplate};

use crate::session::model::MessageState;

/// The number of seconds for which we show the icon acknowledging that the
/// message was sent.
const SENT_VISIBLE_SECONDS: u32 = 3;

mod imp {
    use std::cell::Cell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/content/room_history/message_row/message_state_stack.ui"
    )]
    #[properties(wrapper_type = super::MessageStateStack)]
    pub struct MessageStateStack {
        /// The state that is currently displayed.
        #[property(get, set = Self::set_state, explicit_notify, builder(MessageState::default()))]
        pub state: Cell<MessageState>,
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MessageStateStack {
        const NAME: &'static str = "MessageStateStack";
        type Type = super::MessageStateStack;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for MessageStateStack {}

    impl WidgetImpl for MessageStateStack {}
    impl BinImpl for MessageStateStack {}

    impl MessageStateStack {
        /// Set the state to display.
        pub fn set_state(&self, state: MessageState) {
            let prev_state = self.state.get();

            if prev_state == state {
                return;
            }

            let obj = self.obj();
            let stack = &*self.stack;

            match state {
                MessageState::None => {
                    if matches!(
                        prev_state,
                        MessageState::Sending
                            | MessageState::RecoverableError
                            | MessageState::PermanentError
                    ) {
                        // Show the sent icon.
                        stack.set_visible_child_name("sent");

                        glib::timeout_add_seconds_local_once(
                            SENT_VISIBLE_SECONDS,
                            clone!(
                                #[weak]
                                obj,
                                move || {
                                    obj.set_visible(false);
                                }
                            ),
                        );
                    } else {
                        obj.set_visible(false);
                    }
                }
                MessageState::Sending => {
                    stack.set_visible_child_name("sending");
                    obj.set_visible(true);
                }
                MessageState::RecoverableError => {
                    stack.set_visible_child_name("warning");
                    obj.set_visible(true);
                }
                MessageState::PermanentError => {
                    stack.set_visible_child_name("error");
                    obj.set_visible(true);
                }
                MessageState::Edited => {
                    if matches!(
                        prev_state,
                        MessageState::Sending
                            | MessageState::RecoverableError
                            | MessageState::PermanentError
                    ) {
                        // Show the sent icon.
                        stack.set_visible_child_name("sent");

                        glib::timeout_add_seconds_local_once(
                            SENT_VISIBLE_SECONDS,
                            clone!(
                                #[weak]
                                stack,
                                move || {
                                    stack.set_visible_child_name("edited");
                                }
                            ),
                        );
                    } else {
                        stack.set_visible_child_name("edited");
                        obj.set_visible(true);
                    }
                }
            }

            self.state.set(state);
            obj.notify_state();
        }
    }
}

glib::wrapper! {
    /// A stack to display the different message states.
    pub struct MessageStateStack(ObjectSubclass<imp::MessageStateStack>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl MessageStateStack {
    /// Create a new `MessageStateStack`.
    pub fn new() -> Self {
        glib::Object::new()
    }
}
