use gtk::{gio, glib, glib::clone, prelude::*, subclass::prelude::*};

use crate::utils::BoundObject;

mod imp {
    use std::cell::{Cell, RefCell};

    use super::*;

    #[derive(Debug, glib::Properties)]
    #[properties(wrapper_type = super::FixedSelection)]
    pub struct FixedSelection {
        /// The underlying model.
        #[property(get, set = Self::set_model, explicit_notify, nullable)]
        model: BoundObject<gio::ListModel>,
        /// The position of the selected item.
        #[property(get, set = Self::set_selected, explicit_notify, default = gtk::INVALID_LIST_POSITION)]
        selected: Cell<u32>,
        /// The selected item.
        #[property(get, set = Self::set_selected_item, explicit_notify, nullable)]
        selected_item: RefCell<Option<glib::Object>>,
    }

    impl Default for FixedSelection {
        fn default() -> Self {
            Self {
                model: Default::default(),
                selected: Cell::new(gtk::INVALID_LIST_POSITION),
                selected_item: Default::default(),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FixedSelection {
        const NAME: &'static str = "FixedSelection";
        type Type = super::FixedSelection;
        type Interfaces = (gio::ListModel, gtk::SelectionModel);
    }

    #[glib::derived_properties]
    impl ObjectImpl for FixedSelection {}

    impl ListModelImpl for FixedSelection {
        fn item_type(&self) -> glib::Type {
            glib::Object::static_type()
        }

        fn n_items(&self) -> u32 {
            self.model.obj().map(|m| m.n_items()).unwrap_or_default()
        }

        fn item(&self, position: u32) -> Option<glib::Object> {
            self.model.obj()?.item(position)
        }
    }

    impl SelectionModelImpl for FixedSelection {
        fn selection_in_range(&self, _position: u32, _n_items: u32) -> gtk::Bitset {
            let bitset = gtk::Bitset::new_empty();
            let selected = self.selected.get();

            if selected != gtk::INVALID_LIST_POSITION {
                bitset.add(selected);
            }

            bitset
        }

        fn is_selected(&self, position: u32) -> bool {
            self.selected.get() == position
        }
    }

    impl FixedSelection {
        /// Set the underlying model.
        fn set_model(&self, model: Option<gio::ListModel>) {
            let prev_model = self.model.obj();

            if prev_model == model {
                return;
            }

            let prev_n_items = prev_model
                .as_ref()
                .map(ListModelExt::n_items)
                .unwrap_or_default();
            let n_items = model
                .as_ref()
                .map(ListModelExt::n_items)
                .unwrap_or_default();

            self.model.disconnect_signals();

            let obj = self.obj();
            let _guard = obj.freeze_notify();

            if let Some(model) = model {
                let items_changed_handler = model.connect_items_changed(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |m, p, r, a| {
                        imp.items_changed_cb(m, p, r, a);
                    }
                ));

                self.model.set(model, vec![items_changed_handler]);
            }

            if self.selected.get() != gtk::INVALID_LIST_POSITION {
                self.selected.replace(gtk::INVALID_LIST_POSITION);
                obj.notify_selected();
            }
            if self.selected_item.borrow().is_some() {
                self.selected_item.replace(None);
                obj.notify_selected_item();
            }

            if prev_n_items > 0 || n_items > 0 {
                obj.items_changed(0, prev_n_items, n_items);
            }

            obj.notify_model();
        }

        /// Set the selected item by its position.
        fn set_selected(&self, position: u32) {
            let prev_selected = self.selected.get();
            if prev_selected == position {
                return;
            }

            let selected_item = self.model.obj().and_then(|m| m.item(position));

            let selected = if selected_item.is_none() {
                gtk::INVALID_LIST_POSITION
            } else {
                position
            };

            if prev_selected == selected {
                return;
            }
            let obj = self.obj();

            self.selected.replace(selected);
            self.selected_item.replace(selected_item);

            if prev_selected == gtk::INVALID_LIST_POSITION {
                obj.selection_changed(selected, 1);
            } else if selected == gtk::INVALID_LIST_POSITION {
                obj.selection_changed(prev_selected, 1);
            } else if selected < prev_selected {
                obj.selection_changed(selected, prev_selected - selected + 1);
            } else {
                obj.selection_changed(prev_selected, selected - prev_selected + 1);
            }

            obj.notify_selected();
            obj.notify_selected_item();
        }

        /// Set the selected item.
        fn set_selected_item(&self, item: Option<glib::Object>) {
            if *self.selected_item.borrow() == item {
                return;
            }
            let obj = self.obj();

            let prev_selected = self.selected.get();
            let mut selected = gtk::INVALID_LIST_POSITION;

            if item.is_some() {
                if let Some(model) = self.model.obj() {
                    for i in 0..model.n_items() {
                        let current_item = model.item(i);
                        if current_item == item {
                            selected = i;
                            break;
                        }
                    }
                }
            }

            self.selected_item.replace(item);

            if prev_selected != selected {
                self.selected.replace(selected);

                if prev_selected == gtk::INVALID_LIST_POSITION {
                    obj.selection_changed(selected, 1);
                } else if selected == gtk::INVALID_LIST_POSITION {
                    obj.selection_changed(prev_selected, 1);
                } else if selected < prev_selected {
                    obj.selection_changed(selected, prev_selected - selected + 1);
                } else {
                    obj.selection_changed(prev_selected, selected - prev_selected + 1);
                }
                obj.notify_selected();
            }

            obj.notify_selected_item();
        }

        /// Handle when items changed in the underlying model.
        fn items_changed_cb(
            &self,
            model: &gio::ListModel,
            position: u32,
            removed: u32,
            added: u32,
        ) {
            let obj = self.obj();
            let _guard = obj.freeze_notify();

            let selected = self.selected.get();
            let selected_item = self.selected_item.borrow().clone();

            if selected_item.is_none() || selected < position {
                // unchanged
            } else if selected != gtk::INVALID_LIST_POSITION && selected >= position + removed {
                self.selected.set(selected + added - removed);
                obj.notify_selected();
            } else {
                let mut found = false;

                for i in position..(position + added) {
                    let item = model.item(i);

                    if item == selected_item {
                        if selected != i {
                            // The position of the item changed.
                            self.selected.set(i);
                            obj.notify_selected();
                        }

                        found = true;
                        break;
                    }
                }

                if !found {
                    // The item is no longer in the model.
                    self.selected.set(gtk::INVALID_LIST_POSITION);
                    obj.notify_selected();
                }
            }

            obj.items_changed(position, removed, added);
        }
    }
}

glib::wrapper! {
    /// A `GtkSelectionModel` that keeps track of the selected item even if its
    /// position changes or it is removed from the list.
    pub struct FixedSelection(ObjectSubclass<imp::FixedSelection>)
        @implements gio::ListModel, gtk::SelectionModel;
}

impl FixedSelection {
    /// Construct a new `FixedSelection` with the given model.
    pub fn new(model: Option<&impl IsA<gio::ListModel>>) -> Self {
        glib::Object::builder().property("model", model).build()
    }
}

impl Default for FixedSelection {
    fn default() -> Self {
        Self::new(None::<&gio::ListModel>)
    }
}
