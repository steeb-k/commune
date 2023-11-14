use adw::subclass::prelude::*;
use gettextrs::gettext;
use gtk::{glib, glib::clone, prelude::*, CompositeTemplate};

use crate::session::model::MessageState;

mod imp {
    use std::cell::Cell;

    use glib::subclass::InitializingObject;
    use once_cell::sync::Lazy;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/content/room_history/message_row/message_state_stack.ui"
    )]
    pub struct MessageStateStack {
        /// The state that is currently displayed.
        pub state: Cell<MessageState>,
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub error_image: TemplateChild<gtk::Image>,
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

    impl ObjectImpl for MessageStateStack {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![glib::ParamSpecEnum::builder::<MessageState>("state")
                    .explicit_notify()
                    .build()]
            });

            PROPERTIES.as_ref()
        }

        fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
            let obj = self.obj();

            match pspec.name() {
                "state" => {
                    obj.set_state(value.get().unwrap());
                }
                _ => unimplemented!(),
            }
        }

        fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            let obj = self.obj();

            match pspec.name() {
                "state" => obj.state().to_value(),
                _ => unimplemented!(),
            }
        }
    }
    impl WidgetImpl for MessageStateStack {}
    impl BinImpl for MessageStateStack {}
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

    /// The state that is currently displayed.
    pub fn state(&self) -> MessageState {
        self.imp().state.get()
    }

    /// Set the state to display.
    pub fn set_state(&self, state: MessageState) {
        let prev_state = self.state();

        if prev_state == state {
            return;
        }

        let imp = self.imp();
        let stack = &*imp.stack;
        match state {
            MessageState::None => {
                if matches!(
                    prev_state,
                    MessageState::Sending | MessageState::Error | MessageState::Cancelled
                ) {
                    // Show the sent icon for 2 seconds.
                    stack.set_visible_child_name("sent");

                    glib::timeout_add_seconds_local_once(
                        2,
                        clone!(@weak self as obj => move || {
                            obj.set_visible(false);
                        }),
                    );
                } else {
                    self.set_visible(false);
                }
            }
            MessageState::Sending => {
                stack.set_visible_child_name("sending");
                self.set_visible(true);
            }
            MessageState::Error => {
                imp.error_image
                    .set_tooltip_text(Some(&gettext("Could not send the message")));
                stack.set_visible_child_name("error");
                self.set_visible(true);
            }
            MessageState::Cancelled => {
                imp.error_image
                    .set_tooltip_text(Some(&gettext("An error occurred with the sending queue")));
                stack.set_visible_child_name("error");
                self.set_visible(true);
            }
            MessageState::Edited => {
                if matches!(
                    prev_state,
                    MessageState::Sending | MessageState::Error | MessageState::Cancelled
                ) {
                    // Show the sent icon for 2 seconds.
                    stack.set_visible_child_name("sent");

                    glib::timeout_add_seconds_local_once(
                        2,
                        clone!(@weak stack => move || {
                            stack.set_visible_child_name("edited");
                        }),
                    );
                } else {
                    stack.set_visible_child_name("edited");
                    self.set_visible(true);
                }
            }
        }

        imp.state.set(state);
        self.notify("state");
    }
}
