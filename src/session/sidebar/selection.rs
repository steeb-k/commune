use gtk::{gio, glib, glib::clone, prelude::*, subclass::prelude::*};

use crate::session::room::Room;

mod imp {
    use super::*;
    use once_cell::sync::Lazy;
    use std::cell::{Cell, RefCell};

    #[derive(Debug, Default)]
    pub struct Selection {
        pub model: RefCell<Option<gio::ListModel>>,
        pub selected: Cell<u32>,
        pub selected_room: RefCell<Option<Room>>,
        pub signal_handler: RefCell<Option<glib::SignalHandlerId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Selection {
        const NAME: &'static str = "SidebarSelection";
        type Type = super::Selection;
        type ParentType = glib::Object;
        type Interfaces = (gio::ListModel, gtk::SelectionModel);

        fn new() -> Self {
            Self {
                selected: Cell::new(gtk::INVALID_LIST_POSITION),
                ..Default::default()
            }
        }
    }

    impl ObjectImpl for Selection {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![
                    glib::ParamSpec::new_object(
                        "model",
                        "Model",
                        "The model being managed",
                        gio::ListModel::static_type(),
                        glib::ParamFlags::READWRITE | glib::ParamFlags::EXPLICIT_NOTIFY,
                    ),
                    glib::ParamSpec::new_uint(
                        "selected",
                        "Selected",
                        "The position of the selected item",
                        0,
                        u32::MAX,
                        gtk::INVALID_LIST_POSITION,
                        glib::ParamFlags::READWRITE | glib::ParamFlags::EXPLICIT_NOTIFY,
                    ),
                    glib::ParamSpec::new_object(
                        "selected-room",
                        "Selected Room",
                        "The selected room",
                        Room::static_type(),
                        glib::ParamFlags::READWRITE | glib::ParamFlags::EXPLICIT_NOTIFY,
                    ),
                ]
            });

            PROPERTIES.as_ref()
        }

        fn set_property(
            &self,
            obj: &Self::Type,
            _id: usize,
            value: &glib::Value,
            pspec: &glib::ParamSpec,
        ) {
            match pspec.name() {
                "model" => {
                    let model: Option<gio::ListModel> = value.get().unwrap();
                    obj.set_model(model.as_ref());
                }
                "selected" => {
                    let selected = value.get().unwrap();
                    obj.set_selected(selected);
                }
                "selected-room" => {
                    let selected_room = value.get().unwrap();
                    obj.set_selected_room(selected_room);
                }
                _ => unimplemented!(),
            }
        }

        fn property(&self, obj: &Self::Type, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "model" => obj.model().to_value(),
                "selected" => obj.selected().to_value(),
                "selected-room" => obj.selected_room().to_value(),
                _ => unimplemented!(),
            }
        }
    }

    impl ListModelImpl for Selection {
        fn item_type(&self, _list_model: &Self::Type) -> glib::Type {
            gtk::TreeListRow::static_type()
        }
        fn n_items(&self, _list_model: &Self::Type) -> u32 {
            self.model
                .borrow()
                .as_ref()
                .map(|m| m.n_items())
                .unwrap_or(0)
        }
        fn item(&self, _list_model: &Self::Type, position: u32) -> Option<glib::Object> {
            self.model.borrow().as_ref().and_then(|m| m.item(position))
        }
    }

    impl SelectionModelImpl for Selection {
        fn selection_in_range(
            &self,
            _model: &Self::Type,
            _position: u32,
            _n_items: u32,
        ) -> gtk::Bitset {
            let bitset = gtk::Bitset::new_empty();
            let selected = self.selected.get();

            if selected != gtk::INVALID_LIST_POSITION {
                bitset.add(selected);
            }

            bitset
        }

        fn is_selected(&self, _model: &Self::Type, position: u32) -> bool {
            self.selected.get() == position
        }
    }
}

glib::wrapper! {
    pub struct Selection(ObjectSubclass<imp::Selection>)
        @implements gio::ListModel, gtk::SelectionModel;
}

impl Selection {
    pub fn new<P: IsA<gio::ListModel>>(model: Option<&P>) -> Selection {
        let model = model.map(|m| m.clone().upcast::<gio::ListModel>());
        glib::Object::new(&[("model", &model)]).expect("Failed to create Selection")
    }

    pub fn model(&self) -> Option<gio::ListModel> {
        let priv_ = imp::Selection::from_instance(self);
        priv_.model.borrow().clone()
    }

    pub fn selected(&self) -> u32 {
        let priv_ = imp::Selection::from_instance(self);
        priv_.selected.get()
    }

    pub fn selected_room(&self) -> Option<Room> {
        let priv_ = imp::Selection::from_instance(self);
        priv_.selected_room.borrow().clone()
    }

    pub fn set_model<P: IsA<gio::ListModel>>(&self, model: Option<&P>) {
        let priv_ = imp::Selection::from_instance(self);

        let _guard = self.freeze_notify();

        let model = model.map(|m| m.clone().upcast::<gio::ListModel>());

        let old_model = self.model();
        if old_model == model {
            return;
        }

        let n_items_before = old_model
            .map(|model| {
                if let Some(id) = priv_.signal_handler.take() {
                    model.disconnect(id);
                }
                model.n_items()
            })
            .unwrap_or(0);

        if let Some(model) = model {
            priv_
                .signal_handler
                .replace(Some(model.connect_items_changed(
                    clone!(@weak self as obj => move |m, p, r, a| {
                            obj.items_changed_cb(m, p, r, a);
                    }),
                )));

            self.items_changed_cb(&model, 0, n_items_before, model.n_items());

            priv_.model.replace(Some(model));
        } else {
            priv_.model.replace(None);

            if self.selected() != gtk::INVALID_LIST_POSITION {
                priv_.selected.replace(gtk::INVALID_LIST_POSITION);
                self.notify("selected");
            }
            if self.selected_room().is_some() {
                priv_.selected_room.replace(None);
                self.notify("selected-room");
            }

            self.items_changed(0, n_items_before, 0);
        }

        self.notify("model");
    }

    pub fn set_selected(&self, position: u32) {
        let priv_ = imp::Selection::from_instance(self);

        let old_selected = self.selected();
        if old_selected == position {
            return;
        }

        let selected_room = self
            .model()
            .and_then(|m| m.item(position))
            .and_then(|o| o.downcast::<gtk::TreeListRow>().ok())
            .and_then(|r| r.item())
            .and_then(|o| o.downcast::<Room>().ok());
        let selected = if selected_room.is_none() {
            gtk::INVALID_LIST_POSITION
        } else {
            position
        };

        if old_selected == selected {
            return;
        }

        priv_.selected.replace(selected);
        priv_.selected_room.replace(selected_room);

        if old_selected == gtk::INVALID_LIST_POSITION {
            self.selection_changed(selected, 1);
        } else if selected == gtk::INVALID_LIST_POSITION {
            self.selection_changed(old_selected, 1);
        } else if selected < old_selected {
            self.selection_changed(selected, old_selected - selected + 1);
        } else {
            self.selection_changed(old_selected, selected - old_selected + 1);
        }

        self.notify("selected");
        self.notify("selected-room");
    }

    pub fn set_selected_room(&self, room: Option<Room>) {
        let priv_ = imp::Selection::from_instance(self);

        let selected_room = self.selected_room();
        if selected_room == room {
            return;
        }

        let old_selected = self.selected();

        let mut selected = gtk::INVALID_LIST_POSITION;

        if room.is_some() {
            if let Some(model) = self.model() {
                for i in 0..model.n_items() {
                    let r = model
                        .item(i)
                        .and_then(|o| o.downcast::<gtk::TreeListRow>().ok())
                        .and_then(|r| r.item())
                        .and_then(|o| o.downcast::<Room>().ok());
                    if r == room {
                        selected = i;
                        break;
                    }
                }
            }
        }

        priv_.selected_room.replace(room);

        if old_selected != selected {
            priv_.selected.replace(selected);

            if old_selected == gtk::INVALID_LIST_POSITION {
                self.selection_changed(selected, 1);
            } else if selected == gtk::INVALID_LIST_POSITION {
                self.selection_changed(old_selected, 1);
            } else if selected < old_selected {
                self.selection_changed(selected, old_selected - selected + 1);
            } else {
                self.selection_changed(old_selected, selected - old_selected + 1);
            }
            self.notify("selected");
        }

        self.notify("selected-room");
    }

    fn items_changed_cb(&self, model: &gio::ListModel, position: u32, removed: u32, added: u32) {
        let priv_ = imp::Selection::from_instance(self);

        let _guard = self.freeze_notify();

        let selected = self.selected();
        let selected_room = self.selected_room();

        if selected_room.is_none() || selected < position {
            // unchanged
        } else if selected != gtk::INVALID_LIST_POSITION && selected >= position + removed {
            priv_.selected.replace(selected + added - removed);
            self.notify("selected");
        } else {
            for i in 0..=added {
                if i == added {
                    // the item really was deleted
                    priv_.selected.replace(gtk::INVALID_LIST_POSITION);
                    self.notify("selected");
                } else {
                    let room = model
                        .item(position + i)
                        .and_then(|o| o.downcast::<gtk::TreeListRow>().ok())
                        .and_then(|r| r.item())
                        .and_then(|o| o.downcast::<Room>().ok());
                    if room == selected_room {
                        // the item moved
                        if selected != position + i {
                            priv_.selected.replace(position + i);
                            self.notify("selected");
                        }
                        break;
                    }
                }
            }
        }

        self.items_changed(position, removed, added);
    }
}
