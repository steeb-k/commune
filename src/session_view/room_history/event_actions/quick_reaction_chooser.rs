use adw::{prelude::*, subclass::prelude::*};
use gtk::{
    glib,
    glib::{clone, closure_local},
};

use crate::{
    session::{GlobalAccountData, QUICK_REACTIONS_LEN, ReactionList},
    utils::BoundObject,
};

/// Where each quick reaction sits in the grid.
///
/// The last cell of the second row belongs to the "More Reactions" button.
const QUICK_REACTION_CELLS: &[(i32, i32); QUICK_REACTIONS_LEN] =
    &[(0, 0), (1, 0), (2, 0), (3, 0), (0, 1), (1, 1), (2, 1)];

mod imp {

    use std::{cell::RefCell, collections::HashMap, sync::LazyLock};

    use glib::subclass::{InitializingObject, Signal};

    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate, glib::Properties)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session_view/room_history/event_actions/quick_reaction_chooser.ui"
    )]
    #[properties(wrapper_type = super::QuickReactionChooser)]
    pub struct QuickReactionChooser {
        #[template_child]
        reaction_grid: TemplateChild<gtk::Grid>,
        /// The list of reactions of the event for which this chooser is
        /// presented.
        #[property(get, set = Self::set_reactions, explicit_notify, nullable)]
        reactions: BoundObject<ReactionList>,
        reaction_bindings: RefCell<HashMap<String, glib::Binding>>,
        /// The emoji currently presented, in grid order.
        keys: RefCell<Vec<String>>,
        account_data: BoundObject<GlobalAccountData>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for QuickReactionChooser {
        const NAME: &'static str = "QuickReactionChooser";
        type Type = super::QuickReactionChooser;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            Self::bind_template_callbacks(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for QuickReactionChooser {
        fn signals() -> &'static [Signal] {
            static SIGNALS: LazyLock<Vec<Signal>> =
                LazyLock::new(|| vec![Signal::builder("more-reactions-activated").build()]);
            SIGNALS.as_ref()
        }

        fn constructed(&self) {
            self.parent_constructed();

            self.build_buttons();
        }

        fn dispose(&self) {
            self.account_data.disconnect_signals();
            self.reactions.disconnect_signals();
        }
    }

    impl WidgetImpl for QuickReactionChooser {}
    impl BinImpl for QuickReactionChooser {}

    #[gtk::template_callbacks]
    impl QuickReactionChooser {
        /// Set the list of reactions of the event for which this chooser is
        /// presented.
        fn set_reactions(&self, reactions: Option<ReactionList>) {
            let prev_reactions = self.reactions.obj();

            if prev_reactions == reactions {
                return;
            }

            self.reactions.disconnect_signals();
            for (_, binding) in self.reaction_bindings.borrow_mut().drain() {
                binding.unbind();
            }

            // Reset the state of the buttons.
            for (column, row) in QUICK_REACTION_CELLS {
                if let Some(button) = self
                    .reaction_grid
                    .child_at(*column, *row)
                    .and_downcast::<gtk::ToggleButton>()
                {
                    button.set_active(false);
                }
            }

            if let Some(reactions) = reactions {
                let signal_handler = reactions.connect_items_changed(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_, _, _, _| {
                        imp.update_reactions();
                    }
                ));

                // The chooser outlives the events it is shown for, so the
                // account data is only bound the first time one arrives.
                if self.account_data.obj().is_none() {
                    let account_data = reactions.user().session().global_account_data();
                    let changed_handler = account_data.connect_recent_emoji_changed(clone!(
                        #[weak(rename_to = imp)]
                        self,
                        move |_| {
                            imp.build_buttons();
                            imp.update_reactions();
                        }
                    ));

                    self.account_data.set(account_data, vec![changed_handler]);
                    self.build_buttons();
                }

                self.reactions.set(reactions, vec![signal_handler]);
            }

            self.update_reactions();
        }

        /// Build the quick reaction buttons from the emoji to present.
        fn build_buttons(&self) {
            let keys = match self.account_data.obj() {
                Some(account_data) => account_data.quick_reactions(),
                // Before a session is known, the defaults are all we have.
                None => GlobalAccountData::default_quick_reactions(),
            };

            if *self.keys.borrow() == keys {
                return;
            }

            for (_, binding) in self.reaction_bindings.borrow_mut().drain() {
                binding.unbind();
            }

            let grid = &self.reaction_grid;
            for ((column, row), key) in QUICK_REACTION_CELLS.iter().zip(&keys) {
                if let Some(previous) = grid.child_at(*column, *row) {
                    grid.remove(&previous);
                }

                let button = gtk::ToggleButton::builder()
                    .label(key)
                    .action_name("event.toggle-reaction")
                    .action_target(&key.to_variant())
                    .css_classes(["flat", "circular"])
                    .build();
                button.connect_clicked(|button| {
                    button.activate_action("context-menu.close", None).unwrap();
                });
                grid.attach(&button, *column, *row, 1, 1);
            }

            self.keys.replace(keys);
        }

        /// Update the state of the quick reactions.
        fn update_reactions(&self) {
            let mut reaction_bindings = self.reaction_bindings.borrow_mut();
            let reactions = self.reactions.obj();
            let keys = self.keys.borrow();

            for ((column, row), key) in QUICK_REACTION_CELLS.iter().zip(&*keys) {
                if let Some(reaction) = reactions
                    .as_ref()
                    .and_then(|reactions| reactions.reaction_group_by_key(key))
                {
                    if reaction_bindings.get(key).is_none() {
                        let button = self.reaction_grid.child_at(*column, *row).unwrap();
                        let binding = reaction
                            .bind_property("has-own-user", &button, "active")
                            .sync_create()
                            .build();
                        reaction_bindings.insert(key.clone(), binding);
                    }
                } else if let Some(binding) = reaction_bindings.remove(key) {
                    if let Some(button) = self
                        .reaction_grid
                        .child_at(*column, *row)
                        .and_downcast::<gtk::ToggleButton>()
                    {
                        button.set_active(false);
                    }

                    binding.unbind();
                }
            }
        }

        /// Handle when the "More reactions" button is activated.
        #[template_callback]
        fn more_reactions_activated(&self) {
            self.obj()
                .emit_by_name::<()>("more-reactions-activated", &[]);
        }
    }
}

glib::wrapper! {
    /// A widget displaying quick reactions and taking its state from a [`ReactionList`].
    pub struct QuickReactionChooser(ObjectSubclass<imp::QuickReactionChooser>)
        @extends gtk::Widget, adw::Bin,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl QuickReactionChooser {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Connect to the signal emitted when the "More reactions" button is
    /// activated.
    pub fn connect_more_reactions_activated<F: Fn(&Self) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_closure(
            "more-reactions-activated",
            true,
            closure_local!(move |obj: Self| {
                f(&obj);
            }),
        )
    }
}

impl Default for QuickReactionChooser {
    fn default() -> Self {
        Self::new()
    }
}
