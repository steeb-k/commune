use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{glib, glib::clone};

use crate::{
    components::Avatar,
    prelude::*,
    session::{Call, CallEndReason, CallState, Calls},
    spawn,
};

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/session_view/call_view/mod.ui")]
    #[properties(wrapper_type = super::CallView)]
    pub struct CallView {
        #[template_child]
        stage: TemplateChild<gtk::Stack>,
        #[template_child]
        person_page: TemplateChild<adw::StatusPage>,
        #[template_child]
        remote_avatar: TemplateChild<Avatar>,
        #[template_child]
        remote_picture: TemplateChild<gtk::Picture>,
        #[template_child]
        self_view_revealer: TemplateChild<gtk::Revealer>,
        #[template_child]
        self_picture: TemplateChild<gtk::Picture>,
        #[template_child]
        controls: TemplateChild<gtk::Stack>,
        #[template_child]
        microphone_button: TemplateChild<gtk::ToggleButton>,
        #[template_child]
        camera_button: TemplateChild<gtk::ToggleButton>,
        /// The calls of the session.
        #[property(get, set = Self::set_calls, construct_only)]
        pub(super) calls: RefCell<Option<Calls>>,
        /// The call being shown.
        #[property(get)]
        pub(super) call: RefCell<Option<Call>>,
        /// What the call is doing, in a few words.
        #[property(get)]
        pub(super) status: RefCell<String>,
        /// The tick that keeps the duration honest.
        pub(super) duration_tick: RefCell<Option<glib::SourceId>>,
        /// The handlers on the call being shown.
        pub(super) call_handlers: RefCell<Vec<glib::SignalHandlerId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CallView {
        const NAME: &'static str = "CallView";
        type Type = super::CallView;
        type ParentType = adw::Window;

        fn class_init(klass: &mut Self::Class) {
            Avatar::ensure_type();

            Self::bind_template(klass);
            Self::bind_template_callbacks(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for CallView {
        fn dispose(&self) {
            if let Some(source) = self.duration_tick.take() {
                source.remove();
            }
        }
    }

    impl WidgetImpl for CallView {}

    impl WindowImpl for CallView {
        fn close_request(&self) -> glib::Propagation {
            // Closing the window is not a way to leave somebody hanging on a
            // call that is still running: it hangs up, which is what the person
            // on the other end needs to be told.
            if let Some(call) = self.call.borrow().clone()
                && !call.state().is_ended()
            {
                call.hangup();
            }

            self.parent_close_request()
        }
    }

    impl AdwWindowImpl for CallView {}

    #[gtk::template_callbacks]
    impl CallView {
        /// Set the calls of the session and follow the one that is happening.
        fn set_calls(&self, calls: Option<Calls>) {
            let Some(calls) = calls else {
                return;
            };

            calls.connect_active_call_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |calls| {
                    imp.set_call(calls.active_call());
                }
            ));

            let call = calls.active_call();
            self.calls.replace(Some(calls));
            self.set_call(call);
        }

        /// Show the given call.
        fn set_call(&self, call: Option<Call>) {
            let obj = self.obj();

            if let Some(previous) = self.call.borrow().as_ref() {
                for handler in self.call_handlers.take() {
                    previous.disconnect(handler);
                }
            }

            let Some(call) = call else {
                // The call ended and was forgotten. Whatever it ended as is
                // already on screen; leave it there for the person to read.
                self.update_controls();
                return;
            };

            let handlers = vec![
                call.connect_state_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.update();
                    }
                )),
                call.connect_remote_member_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.update_person();
                    }
                )),
                call.connect_remote_paintable_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.update_video();
                    }
                )),
                call.connect_local_paintable_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.update_video();
                    }
                )),
                call.connect_is_remote_camera_muted_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.update_video();
                    }
                )),
                call.connect_is_microphone_muted_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.update_buttons();
                    }
                )),
                call.connect_is_camera_muted_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.update_buttons();
                        imp.update_video();
                    }
                )),
            ];

            self.call.replace(Some(call));
            self.call_handlers.replace(handlers);

            obj.notify_call();
            self.update();
            self.update_person();
        }

        /// The name of the person on the other end.
        fn remote_name(&self) -> String {
            self.call
                .borrow()
                .as_ref()
                .and_then(Call::remote_member)
                .map_or_else(|| gettext("Call"), |member| member.display_name())
        }

        /// Refresh everything that depends on the state of the call.
        fn update(&self) {
            self.update_status();
            self.update_controls();
            self.update_buttons();
            self.update_video();
            self.update_duration_tick();
        }

        /// Refresh the avatar and the name.
        ///
        /// The window's own title is what the compositor shows in a task list,
        /// so it is the person's name too — set here rather than bound, since a
        /// property cannot be bound to itself on the same object.
        fn update_person(&self) {
            let member = self.call.borrow().as_ref().and_then(Call::remote_member);
            let name = self.remote_name();

            self.remote_avatar
                .set_data(member.as_ref().map(PillSourceExt::avatar_data));
            self.person_page.set_title(&name);
            self.obj().set_title(Some(&name));
        }

        /// Say what the call is doing.
        fn update_status(&self) {
            let borrowed = self.call.borrow();
            let Some(call) = borrowed.as_ref() else {
                return;
            };

            let status = match call.state() {
                CallState::Ringing => gettext("Incoming call"),
                CallState::Dialing => gettext("Calling…"),
                CallState::Connecting => gettext("Connecting…"),
                CallState::Connected => Self::duration_text(call),
                CallState::Ended => match call.end_reason() {
                    CallEndReason::HungUp => gettext("Call ended"),
                    CallEndReason::Declined => gettext("Call declined"),
                    CallEndReason::NotAnswered => gettext("No answer"),
                    CallEndReason::AnsweredElsewhere => gettext("Answered on another device"),
                    CallEndReason::NoConnection => gettext("Could not connect"),
                    CallEndReason::MediaFailed => {
                        gettext("Could not use the microphone or the camera")
                    }
                    CallEndReason::Failed => gettext("Call failed"),
                },
            };
            drop(borrowed);

            if *self.status.borrow() == status {
                return;
            }

            self.status.replace(status);
            self.obj().notify_status();
        }

        /// How long the call has been running, as a sentence.
        fn duration_text(call: &Call) -> String {
            let connected_at = call.connected_at();

            if connected_at == 0 {
                return gettext("Connected");
            }

            let now = glib::DateTime::now_utc()
                .map(|now| now.to_unix().unsigned_abs())
                .unwrap_or_default();
            let seconds = now.saturating_sub(connected_at);

            let minutes = seconds / 60;
            let seconds = seconds % 60;

            format!("{minutes}∶{seconds:02}")
        }

        /// Keep the duration ticking while the call is up.
        fn update_duration_tick(&self) {
            let connected = self
                .call
                .borrow()
                .as_ref()
                .is_some_and(|call| call.state() == CallState::Connected);

            if !connected {
                if let Some(source) = self.duration_tick.take() {
                    source.remove();
                }
                return;
            }

            if self.duration_tick.borrow().is_some() {
                return;
            }

            let source = glib::timeout_add_seconds_local(
                1,
                clone!(
                    #[weak(rename_to = imp)]
                    self,
                    #[upgrade_or]
                    glib::ControlFlow::Break,
                    move || {
                        imp.update_status();
                        glib::ControlFlow::Continue
                    }
                ),
            );
            self.duration_tick.replace(Some(source));
        }

        /// Show the controls that go with the state of the call.
        fn update_controls(&self) {
            let name = match self.call.borrow().as_ref().map(Call::state) {
                Some(CallState::Ringing) => "incoming",
                Some(CallState::Ended) | None => "ended",
                Some(_) => "in-call",
            };

            self.controls.set_visible_child_name(name);
        }

        /// Set the two toggle buttons to what they actually do right now.
        fn update_buttons(&self) {
            let borrowed = self.call.borrow();
            let Some(call) = borrowed.as_ref() else {
                return;
            };

            let microphone_muted = call.is_microphone_muted();
            self.microphone_button.set_active(microphone_muted);
            self.microphone_button.set_icon_name(if microphone_muted {
                "microphone-disabled-symbolic"
            } else {
                "audio-input-microphone-symbolic"
            });
            self.microphone_button
                .set_tooltip_text(Some(&if microphone_muted {
                    gettext("Unmute Microphone")
                } else {
                    gettext("Mute Microphone")
                }));

            // A call with no video section has no camera button: turning off
            // something that was never on is a control that does nothing.
            let has_video = call.has_video() && call.local_paintable().is_some();
            self.camera_button.set_visible(has_video);

            let camera_muted = call.is_camera_muted();
            self.camera_button.set_active(camera_muted);
            self.camera_button.set_icon_name(if camera_muted {
                "camera-disabled-symbolic"
            } else {
                "camera-web-symbolic"
            });
            self.camera_button.set_tooltip_text(Some(&if camera_muted {
                gettext("Turn On Camera")
            } else {
                gettext("Turn Off Camera")
            }));
        }

        /// Show video where there is video, and a face where there is not.
        fn update_video(&self) {
            let borrowed = self.call.borrow();
            let Some(call) = borrowed.as_ref() else {
                self.stage.set_visible_child_name("person");
                self.self_view_revealer.set_reveal_child(false);
                return;
            };

            let remote_paintable = call.remote_paintable();

            // The other end tells us when they mute their camera, and the spec
            // asks that it be honoured locally rather than shown as a frozen
            // frame or a black rectangle.
            let show_remote_video = remote_paintable.is_some()
                && !call.is_remote_camera_muted()
                && matches!(call.state(), CallState::Connecting | CallState::Connected);

            self.remote_picture.set_paintable(remote_paintable.as_ref());
            self.stage
                .set_visible_child_name(if show_remote_video { "video" } else { "person" });

            let local_paintable = call.local_paintable();
            self.self_picture.set_paintable(local_paintable.as_ref());
            self.self_view_revealer.set_reveal_child(
                local_paintable.is_some() && !call.is_camera_muted() && !call.state().is_ended(),
            );
        }

        #[template_callback]
        fn accept(&self) {
            let Some(calls) = self.calls.borrow().clone() else {
                return;
            };

            spawn!(async move {
                calls.accept_active_call().await;
            });
        }

        #[template_callback]
        fn decline(&self) {
            if let Some(call) = self.call.borrow().as_ref() {
                call.reject();
            }
        }

        #[template_callback]
        fn hangup(&self) {
            if let Some(call) = self.call.borrow().as_ref() {
                call.hangup();
            }
        }

        #[template_callback]
        fn toggle_microphone(&self) {
            if let Some(call) = self.call.borrow().as_ref() {
                call.set_is_microphone_muted(self.microphone_button.is_active());
            }
        }

        #[template_callback]
        fn toggle_camera(&self) {
            if let Some(call) = self.call.borrow().as_ref() {
                call.set_is_camera_muted(self.camera_button.is_active());
            }
        }

        #[template_callback]
        fn close_window(&self) {
            self.obj().close();
        }
    }
}

glib::wrapper! {
    /// The window of a voice or video call.
    pub struct CallView(ObjectSubclass<imp::CallView>)
        @extends gtk::Widget, gtk::Window, adw::Window,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Native,
            gtk::Root, gtk::ShortcutManager;
}

impl CallView {
    /// Construct a `CallView` for the calls of the given session.
    pub(crate) fn new(calls: &Calls) -> Self {
        glib::Object::builder().property("calls", calls).build()
    }
}
