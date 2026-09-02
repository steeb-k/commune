use adw::{prelude::*, subclass::prelude::*};
use as_variant::as_variant;
use gettextrs::gettext;
use gtk::{glib, glib::clone};
use matrix_sdk_ui::timeline::TimelineItemContent;
use ruma::events::rtc::notification::CallIntent;

use super::{EventTimestamp, ReadReceiptsList};
use crate::{
    gettext_f,
    prelude::*,
    session::{CallOutcome, Calls, Event, JoinRuleValue, Member, sdp_has_video},
    utils::{BoundObject, BoundObjectWeakRef},
};

mod imp {
    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/session_view/room_history/call_row.ui")]
    #[properties(wrapper_type = super::CallRow)]
    pub struct CallRow {
        /// The call icon shown to the user.
        #[template_child]
        icon: TemplateChild<gtk::Image>,
        /// The text shown to the user.
        #[template_child]
        inner_label: TemplateChild<gtk::Label>,
        /// The `RtcNotification` event displayed by this widget.
        #[property(get, set = Self::set_event, explicit_notify)]
        event: BoundObject<Event>,
        /// The sender of the event that is presented.
        #[property(get = Self::sender, explicit_notify)]
        sender: BoundObjectWeakRef<Member>,
        /// The calls of the session, which are what says how a call ended.
        pub(super) calls: BoundObjectWeakRef<Calls>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CallRow {
        const NAME: &'static str = "CallRow";
        type Type = super::CallRow;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            EventTimestamp::ensure_type();
            ReadReceiptsList::ensure_type();

            Self::bind_template(klass);
            klass.set_css_name("call-row");
            klass.set_accessible_role(gtk::AccessibleRole::Group);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for CallRow {}

    impl WidgetImpl for CallRow {}
    impl BinImpl for CallRow {}

    impl CallRow {
        /// Set the event presented by this row.
        fn set_event(&self, event: Event) {
            let obj = self.obj();

            // Update CSS classes of the parent
            if let Some(row) = self.obj().parent() {
                row.add_css_class("has-icon");
            }

            // Listen for redactions, as they're allowed by the protocol, to
            // make sure we're always rendering reasonable state.
            self.event.disconnect_signals();
            let item_changed_handler = event.connect_item_changed(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_| {
                    imp.update_content();
                }
            ));
            self.event.set(event, vec![item_changed_handler]);

            self.sender.disconnect_signals();

            if let Some(sender) = self.sender() {
                let handler = sender.connect_disambiguated_name_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.update_content();
                    }
                ));

                self.sender.set(&sender, vec![handler]);
            }
            // What became of a call is not in the event; it is in what came
            // after it, which the session keeps. A row is redrawn when the
            // answer changes — a call that was ringing when the row was built
            // and has since been missed says so without a reload.
            self.calls.disconnect_signals();

            if let Some(calls) = self
                .event
                .obj()
                .and_then(|event| event.room().session())
                .map(|session| session.calls())
            {
                let handler = calls.connect_call_outcome_changed(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_, call_id| {
                        if imp.call_id().as_deref() == Some(call_id.as_str()) {
                            imp.update_content();
                        }
                    }
                ));

                self.calls.set(&calls, vec![handler]);
            }

            obj.notify_event();
            obj.notify_sender();

            self.update_content();
        }

        /// The ID of the call this row is about, if it is about a one-to-one
        /// call.
        fn call_id(&self) -> Option<String> {
            let event = self.event.obj()?;
            let invite = event.call_invite()?;

            Some(invite.call_id.as_str().to_owned())
        }

        /// The sender of the event that is presented.
        fn sender(&self) -> Option<Member> {
            self.event.obj().map(|event| event.sender())
        }

        /// Update the content for the current state.
        fn update_content(&self) {
            let Some(sender) = self.sender() else {
                return;
            };

            if let Some(event) = self.event.obj()
                && let Some(invite) = event.call_invite()
            {
                self.update_call_invite(&event, &sender, &invite);
                return;
            }

            let call_intent = self.event.obj().and_then(|event| {
                as_variant!(event.content(), TimelineItemContent::RtcNotification { call_intent, .. } => call_intent)?
            });
            if let Some(CallIntent::Video) = call_intent {
                let text = if sender.is_own_user() {
                    gettext("Outgoing video call.")
                } else {
                    gettext_f(
                        // Translators: Do NOT translate the content between '{' and '}', this
                        // is a variable name.
                        "Incoming video call from {user}. Use another client to answer.",
                        &[("user", &sender.disambiguated_name())],
                    )
                };

                self.inner_label.set_text(&text);
                self.icon.set_icon_name(Some("video-symbolic"));
            } else {
                let text = if sender.is_own_user() {
                    gettext("Outgoing call.")
                } else {
                    gettext_f(
                        // Translators: Do NOT translate the content between '{' and '}', this
                        // is a variable name.
                        "Incoming call from {user}. Use another client to answer.",
                        &[("user", &sender.disambiguated_name())],
                    )
                };

                self.inner_label.set_text(&text);
                self.icon.set_icon_name(Some("phone-right-facing-symbolic"));
            }
        }

        /// Say what happened to a one-to-one call.
        ///
        /// The invite is all the timeline has: the rest of the module's events
        /// are filtered out, and the SDK gives no item for them anyway. What
        /// became of the call comes from the session, which watched it happen
        /// — and for a call that happened before this client was running, it
        /// has nothing, so the row says a call was placed and no more.
        fn update_call_invite(
            &self,
            event: &Event,
            sender: &Member,
            invite: &ruma::events::call::invite::CallInviteEventContent,
        ) {
            let is_own = sender.is_own_user();
            let name = sender.disambiguated_name();
            // The offer, not a field: whether a call carries video is in its
            // SDP and nowhere else.
            let has_video = sdp_has_video(&invite.offer.sdp);

            let room = event.room();
            let calls = self.calls.obj();
            let call_id = invite.call_id.as_str();

            let is_ongoing = calls
                .as_ref()
                .and_then(Calls::active_call)
                .is_some_and(|call| call.call_id().is_some_and(|id| id.as_str() == call_id));
            let outcome = calls.as_ref().and_then(|calls| calls.outcome(call_id));

            // "As a starting point, it is RECOMMENDED that clients ignore call
            // invites in rooms with a join rule of public", which this client
            // does — and the same paragraph asks that the row say so, since
            // otherwise a call that never rang looks like one that was
            // ignored on purpose.
            let was_suppressed =
                !is_own && outcome.is_none() && room.join_rule().value() == JoinRuleValue::Public;

            let text = if was_suppressed {
                gettext_f(
                    // Translators: Do NOT translate the content between '{' and '}', this
                    // is a variable name.
                    "Call from {user}. Calls in public rooms are not rung for.",
                    &[("user", &name)],
                )
            } else if is_ongoing {
                gettext("Call in progress.")
            } else {
                match outcome {
                    Some(CallOutcome::Answered) => gettext("Call ended."),
                    Some(CallOutcome::Declined) => gettext("Call declined."),
                    Some(CallOutcome::Missed | CallOutcome::Ringing) => {
                        if is_own {
                            gettext("No answer.")
                        } else {
                            gettext_f(
                                // Translators: Do NOT translate the content between '{' and
                                // '}', this is a variable name.
                                "Missed call from {user}.",
                                &[("user", &name)],
                            )
                        }
                    }
                    // A call from before this client was running. Nothing here
                    // knows how it ended, and inventing an answer is worse
                    // than saying only what the invite says.
                    None => match (is_own, has_video) {
                        (true, true) => gettext("Outgoing video call."),
                        (true, false) => gettext("Outgoing call."),
                        (false, true) => gettext_f(
                            // Translators: Do NOT translate the content between '{' and '}',
                            // this is a variable name.
                            "Incoming video call from {user}.",
                            &[("user", &name)],
                        ),
                        (false, false) => gettext_f(
                            // Translators: Do NOT translate the content between '{' and '}',
                            // this is a variable name.
                            "Incoming call from {user}.",
                            &[("user", &name)],
                        ),
                    },
                }
            };

            self.inner_label.set_text(&text);
            self.icon.set_icon_name(Some(if has_video {
                "video-symbolic"
            } else {
                "phone-right-facing-symbolic"
            }));
        }
    }
}

glib::wrapper! {
    /// A row showing an `m.rtc.notification` event ([MSC4075]) in the timeline.
    ///
    /// [MSC4075]: https://github.com/matrix-org/matrix-spec-proposals/pull/4075
    pub struct CallRow(ObjectSubclass<imp::CallRow>)
        @extends gtk::Widget, adw::Bin,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl CallRow {
    pub fn new() -> Self {
        glib::Object::new()
    }
}

impl Default for CallRow {
    fn default() -> Self {
        Self::new()
    }
}
