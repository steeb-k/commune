use adw::prelude::*;
use gtk::{gdk, gio, glib, glib::clone, pango, subclass::prelude::*, CompositeTemplate};
use tracing::error;

use crate::utils::BoundObjectWeakRef;

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/content/room_details/addresses_subpage/completion_popover.ui"
    )]
    #[properties(wrapper_type = super::CompletionPopover)]
    pub struct CompletionPopover {
        #[template_child]
        pub list: TemplateChild<gtk::ListBox>,
        /// The parent entry to autocomplete.
        #[property(get, set = Self::set_entry, explicit_notify, nullable)]
        pub entry: BoundObjectWeakRef<gtk::Editable>,
        /// The key controller added to the parent entry.
        entry_controller: RefCell<Option<gtk::EventControllerKey>>,
        entry_binding: RefCell<Option<glib::Binding>>,
        /// The list model to use for completion.
        ///
        /// Only supports `GtkStringObject` items.
        #[property(get, set = Self::set_model, explicit_notify, nullable)]
        pub model: RefCell<Option<gio::ListModel>>,
        /// The string filter.
        #[property(get)]
        pub filter: gtk::StringFilter,
        /// The filtered list model.
        #[property(get)]
        pub filtered_list: gtk::FilterListModel,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CompletionPopover {
        const NAME: &'static str = "RoomDetailsAddressesSubpageCompletionPopover";
        type Type = super::CompletionPopover;
        type ParentType = gtk::Popover;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            Self::Type::bind_template_callbacks(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for CompletionPopover {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();

            self.filter
                .set_expression(Some(gtk::StringObject::this_expression("string")));
            self.filtered_list.set_filter(Some(&self.filter));

            self.filtered_list.connect_items_changed(clone!(
                #[weak]
                obj,
                move |_, _, _, _| {
                    obj.update_completion();
                }
            ));

            self.list.bind_model(Some(&self.filtered_list), |item| {
                let Some(item) = item.downcast_ref::<gtk::StringObject>() else {
                    error!("Completion has item that is not a GtkStringObject");
                    return adw::Bin::new().upcast();
                };

                let label = gtk::Label::builder()
                    .label(item.string())
                    .ellipsize(pango::EllipsizeMode::End)
                    .halign(gtk::Align::Start)
                    .build();

                gtk::ListBoxRow::builder().child(&label).build().upcast()
            });
        }

        fn dispose(&self) {
            if let Some(entry) = self.entry.obj() {
                if let Some(controller) = self.entry_controller.take() {
                    entry.remove_controller(&controller);
                }
            }

            if let Some(binding) = self.entry_binding.take() {
                binding.unbind();
            }
        }
    }

    impl WidgetImpl for CompletionPopover {}
    impl PopoverImpl for CompletionPopover {}

    impl CompletionPopover {
        /// Set the parent entry to autocomplete.
        fn set_entry(&self, entry: Option<&gtk::Editable>) {
            let prev_entry = self.entry.obj();

            if prev_entry.as_ref() == entry {
                return;
            }
            let obj = self.obj();

            if let Some(entry) = prev_entry {
                if let Some(controller) = self.entry_controller.take() {
                    entry.remove_controller(&controller);
                }

                obj.unparent();
            }
            if let Some(binding) = self.entry_binding.take() {
                binding.unbind();
            }
            self.entry.disconnect_signals();

            if let Some(entry) = entry {
                let key_events = gtk::EventControllerKey::new();
                key_events.connect_key_pressed(clone!(
                    #[weak]
                    obj,
                    #[upgrade_or]
                    glib::Propagation::Proceed,
                    move |_, key, _, modifier| {
                        if modifier.is_empty() {
                            if obj.is_visible() {
                                let imp = obj.imp();
                                if matches!(
                                    key,
                                    gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::ISO_Enter
                                ) {
                                    // Activate completion.
                                    obj.activate_selected_row();
                                    return glib::Propagation::Stop;
                                } else if matches!(key, gdk::Key::Up | gdk::Key::KP_Up) {
                                    // Move up, if possible.
                                    let idx = obj.selected_row_index().unwrap_or_default();
                                    if idx > 0 {
                                        obj.select_row_at_index(Some(idx - 1));
                                    }
                                    return glib::Propagation::Stop;
                                } else if matches!(key, gdk::Key::Down | gdk::Key::KP_Down) {
                                    // Move down, if possible.
                                    let new_idx = if let Some(idx) = obj.selected_row_index() {
                                        idx + 1
                                    } else {
                                        0
                                    };
                                    let max = imp.filtered_list.n_items() as usize;

                                    if new_idx < max {
                                        obj.select_row_at_index(Some(new_idx));
                                    }
                                    return glib::Propagation::Stop;
                                } else if matches!(key, gdk::Key::Escape) {
                                    // Close.
                                    obj.popdown();
                                    return glib::Propagation::Stop;
                                }
                            } else if matches!(key, gdk::Key::Tab) {
                                obj.update_completion();
                                return glib::Propagation::Stop;
                            }
                        }
                        glib::Propagation::Proceed
                    }
                ));

                entry.add_controller(key_events.clone());
                self.entry_controller.replace(Some(key_events));

                let search_binding = entry
                    .bind_property("text", &self.filter, "search")
                    .sync_create()
                    .build();
                self.entry_binding.replace(Some(search_binding));

                let changed_handler = entry.connect_changed(clone!(
                    #[weak]
                    obj,
                    move |_| {
                        obj.update_completion();
                    }
                ));

                let state_flags_handler = entry.connect_state_flags_changed(clone!(
                    #[weak]
                    obj,
                    move |_, _| {
                        obj.update_completion();
                    }
                ));

                obj.set_parent(entry);
                self.entry
                    .set(entry, vec![changed_handler, state_flags_handler]);
            }

            self.obj().notify_entry();
        }

        /// Set the list model to use for completion.
        fn set_model(&self, model: Option<gio::ListModel>) {
            if *self.model.borrow() == model {
                return;
            }

            self.filtered_list.set_model(model.as_ref());

            self.model.replace(model);
            self.obj().notify_model();
        }
    }
}

glib::wrapper! {
    /// A popover to auto-complete strings for a `gtk::Editable`.
    pub struct CompletionPopover(ObjectSubclass<imp::CompletionPopover>)
        @extends gtk::Widget, gtk::Popover, @implements gtk::Accessible;
}

#[gtk::template_callbacks]
impl CompletionPopover {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Update completion.
    fn update_completion(&self) {
        let Some(entry) = self.entry() else {
            return;
        };

        let imp = self.imp();
        let n_items = imp.filtered_list.n_items();

        // Always hide the popover if it's empty.
        if n_items == 0 {
            if self.is_visible() {
                self.popdown();
            }

            return;
        }

        // Always hide the popover if it has a single item that is exactly the text of
        // the entry.
        if n_items == 1 {
            if let Some(item) = imp
                .filtered_list
                .item(0)
                .and_downcast::<gtk::StringObject>()
            {
                if item.string() == entry.text() {
                    if self.is_visible() {
                        self.popdown();
                    }

                    return;
                }
            }
        }

        // Only show the popover if the entry is focused.
        let entry_has_focus = entry.state_flags().contains(gtk::StateFlags::FOCUS_WITHIN);
        if entry_has_focus {
            if !self.is_visible() {
                self.popup();
            }
        } else if self.is_visible() {
            self.popdown();
        }
    }

    fn selected_row_index(&self) -> Option<usize> {
        let imp = self.imp();

        let selected_text = self.selected_text()?;

        imp.filtered_list.iter::<glib::Object>().position(|o| {
            o.ok()
                .and_downcast::<gtk::StringObject>()
                .is_some_and(|o| o.string() == selected_text)
        })
    }

    fn select_row_at_index(&self, idx: Option<usize>) {
        let imp = self.imp();

        if self.selected_row_index() == idx || idx >= Some(imp.filtered_list.n_items() as usize) {
            return;
        }

        let imp = self.imp();

        if let Some(row) =
            idx.and_then(|idx| imp.list.row_at_index(idx.try_into().unwrap_or(i32::MAX)))
        {
            imp.list.select_row(Some(&row));
        } else {
            imp.list.select_row(None::<&gtk::ListBoxRow>);
        }
    }

    /// The text of the selected row, if any.
    pub fn selected_text(&self) -> Option<glib::GString> {
        Some(
            self.imp()
                .list
                .selected_row()?
                .child()?
                .downcast_ref::<gtk::Label>()?
                .label(),
        )
    }

    /// Activate the selected row.
    ///
    /// Returns `true` if the row was activated.
    pub fn activate_selected_row(&self) -> bool {
        if !self.is_visible() {
            return false;
        }
        let Some(entry) = self.entry() else {
            return false;
        };

        let Some(selected_text) = self.selected_text() else {
            return false;
        };

        if selected_text == entry.text() {
            // Activating the row would have no effect.
            return false;
        }

        let Some(row) = self.imp().list.selected_row() else {
            return false;
        };

        row.activate();
        true
    }

    /// Handle a row being activated.
    #[template_callback]
    fn row_activated(&self, row: &gtk::ListBoxRow) {
        let Some(label) = row.child().and_downcast::<gtk::Label>() else {
            return;
        };
        let Some(entry) = self.entry() else {
            return;
        };

        entry.set_text(&label.label());

        self.popdown();
        entry.grab_focus();
    }
}

impl Default for CompletionPopover {
    fn default() -> Self {
        Self::new()
    }
}
