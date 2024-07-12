use gtk::{gio, glib, glib::clone, prelude::*, subclass::prelude::*};

use crate::utils::BoundObject;

mod imp {
    use std::cell::{Cell, RefCell};

    use super::*;

    #[derive(Debug, glib::Properties)]
    #[properties(wrapper_type = super::Selection)]
    pub struct Selection {
        /// The underlying model.
        #[property(get, set = Self::set_model, explicit_notify, nullable)]
        pub model: BoundObject<gio::ListModel>,
        /// The position of the selected item.
        #[property(get, set = Self::set_selected, explicit_notify, default = gtk::INVALID_LIST_POSITION)]
        pub selected: Cell<u32>,
        /// The selected item.
        #[property(get, set = Self::set_selected_item, explicit_notify, nullable)]
        pub selected_item: RefCell<Option<glib::Object>>,
    }

    impl Default for Selection {
        fn default() -> Self {
            Self {
                model: Default::default(),
                selected: Cell::new(gtk::INVALID_LIST_POSITION),
                selected_item: Default::default(),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Selection {
        const NAME: &'static str = "SidebarSelection";
        type Type = super::Selection;
        type Interfaces = (gio::ListModel, gtk::SelectionModel);
    }

    #[glib::derived_properties]
    impl ObjectImpl for Selection {}

    impl ListModelImpl for Selection {
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

    impl SelectionModelImpl for Selection {
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

    impl Selection {
        /// Set the underlying model.
        fn set_model(&self, model: Option<gio::ListModel>) {
            let obj = self.obj();
            let _guard = obj.freeze_notify();

            let model = model.map(|m| m.clone().upcast());

            let old_model = self.model.obj();
            if old_model == model {
                return;
            }

            let n_items_before = old_model.map(|model| model.n_items()).unwrap_or(0);
            self.model.disconnect_signals();

            if let Some(model) = model {
                let items_changed_handler = model.connect_items_changed(clone!(
                    #[weak]
                    obj,
                    move |m, p, r, a| {
                        obj.items_changed_cb(m, p, r, a);
                    }
                ));

                self.model.set(model.clone(), vec![items_changed_handler]);
                obj.items_changed_cb(&model, 0, n_items_before, model.n_items());
            } else {
                if self.selected.get() != gtk::INVALID_LIST_POSITION {
                    self.selected.replace(gtk::INVALID_LIST_POSITION);
                    obj.notify_selected();
                }
                if self.selected_item.borrow().is_some() {
                    self.selected_item.replace(None);
                    obj.notify_selected_item();
                }

                obj.items_changed(0, n_items_before, 0);
            }

            obj.notify_model();
        }

        /// Set the selected item by its position.
        fn set_selected(&self, position: u32) {
            let old_selected = self.selected.get();
            if old_selected == position {
                return;
            }

            let selected_item = self.model.obj().and_then(|m| m.item(position));

            let selected = if selected_item.is_none() {
                gtk::INVALID_LIST_POSITION
            } else {
                position
            };

            if old_selected == selected {
                return;
            }
            let obj = self.obj();

            self.selected.replace(selected);
            self.selected_item.replace(selected_item);

            if old_selected == gtk::INVALID_LIST_POSITION {
                obj.selection_changed(selected, 1);
            } else if selected == gtk::INVALID_LIST_POSITION {
                obj.selection_changed(old_selected, 1);
            } else if selected < old_selected {
                obj.selection_changed(selected, old_selected - selected + 1);
            } else {
                obj.selection_changed(old_selected, selected - old_selected + 1);
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

            let old_selected = self.selected.get();
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

            if old_selected != selected {
                self.selected.replace(selected);

                if old_selected == gtk::INVALID_LIST_POSITION {
                    obj.selection_changed(selected, 1);
                } else if selected == gtk::INVALID_LIST_POSITION {
                    obj.selection_changed(old_selected, 1);
                } else if selected < old_selected {
                    obj.selection_changed(selected, old_selected - selected + 1);
                } else {
                    obj.selection_changed(old_selected, selected - old_selected + 1);
                }
                obj.notify_selected();
            }

            obj.notify_selected_item();
        }
    }
}

glib::wrapper! {
    /// A `GtkSelectionModel` that keeps track of the selected item even if its position changes or it is removed from the list.
    pub struct Selection(ObjectSubclass<imp::Selection>)
        @implements gio::ListModel, gtk::SelectionModel;
}

impl Selection {
    pub fn new<P: IsA<gio::ListModel>>(model: Option<&P>) -> Selection {
        let model = model.map(|m| m.clone().upcast());
        glib::Object::builder().property("model", &model).build()
    }

    fn items_changed_cb(&self, model: &gio::ListModel, position: u32, removed: u32, added: u32) {
        let imp = self.imp();

        let _guard = self.freeze_notify();

        let selected = self.selected();
        let selected_item = self.selected_item();

        if selected_item.is_none() || selected < position {
            // unchanged
        } else if selected != gtk::INVALID_LIST_POSITION && selected >= position + removed {
            imp.selected.replace(selected + added - removed);
            self.notify_selected();
        } else {
            for i in 0..=added {
                if i == added {
                    // the item really was deleted
                    imp.selected.replace(gtk::INVALID_LIST_POSITION);
                    self.notify_selected();
                } else {
                    let item = model.item(position + i);
                    if item == selected_item {
                        // the item moved
                        if selected != position + i {
                            imp.selected.replace(position + i);
                            self.notify_selected();
                        }
                        break;
                    }
                }
            }
        }

        self.items_changed(position, removed, added);
    }
}

impl Default for Selection {
    fn default() -> Self {
        Self::new(gio::ListModel::NONE)
    }
}
