use crate::session::sidebar::FrctlRoom;
use gettextrs::gettext;
use gtk::subclass::prelude::*;
use gtk::{self, gio, glib, prelude::*};
use matrix_sdk::{identifiers::RoomId, Client};
use matrix_sdk::{room::Room, RoomType};

// TODO: do we also want the categorie `People` and a custom categorie support?
#[derive(Debug, Eq, PartialEq, Clone, Copy, glib::GEnum)]
#[repr(u32)]
#[genum(type_name = "CategoryName")]
pub enum CategoryName {
    Invited = 0,
    Favorite = 1,
    Normal = 2,
    LowPriority = 3,
    Left = 4,
}

impl CategoryName {
    pub fn get_room_type(&self) -> RoomType {
        match self {
            CategoryName::Invited => RoomType::Invited,
            CategoryName::Favorite => RoomType::Joined,
            CategoryName::Normal => RoomType::Joined,
            CategoryName::LowPriority => RoomType::Joined,
            CategoryName::Left => RoomType::Left,
        }
    }
}

impl Default for CategoryName {
    fn default() -> Self {
        CategoryName::Normal
    }
}

impl ToString for CategoryName {
    fn to_string(&self) -> String {
        match self {
            CategoryName::Invited => gettext("Invited"),
            CategoryName::Favorite => gettext("Favorite"),
            CategoryName::Normal => gettext("Rooms"),
            CategoryName::LowPriority => gettext("Low Priority"),
            CategoryName::Left => gettext("Historical"),
        }
    }
}

mod imp {
    use super::*;
    use gio::subclass::prelude::*;
    use once_cell::sync::OnceCell;
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;

    #[derive(Debug)]
    pub struct FrctlCategory {
        pub client: OnceCell<Client>,
        pub map: RefCell<HashMap<RoomId, (u32, FrctlRoom)>>,
        pub list: RefCell<Vec<RoomId>>,
        pub name: Cell<CategoryName>,
        pub expanded: Cell<bool>,
        pub selected: Cell<u32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FrctlCategory {
        const NAME: &'static str = "FrctlCategory";
        type Type = super::FrctlCategory;
        type ParentType = glib::Object;
        type Interfaces = (gio::ListModel, gtk::SelectionModel);

        fn new() -> Self {
            Self {
                client: OnceCell::new(),
                map: RefCell::new(HashMap::new()),
                list: RefCell::new(Vec::new()),
                name: Cell::new(CategoryName::default()),
                expanded: Cell::new(true),
                selected: Cell::new(u32::MAX),
            }
        }
    }

    impl ObjectImpl for FrctlCategory {
        fn properties() -> &'static [glib::ParamSpec] {
            use once_cell::sync::Lazy;
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![
                    glib::ParamSpec::enum_(
                        "display-name",
                        "Display Name",
                        "The name of this category",
                        CategoryName::static_type(),
                        CategoryName::default() as i32,
                        glib::ParamFlags::READWRITE,
                    ),
                    glib::ParamSpec::boolean(
                        "expanded",
                        "Expanded",
                        "Wheter this category is expanded or not",
                        true,
                        glib::ParamFlags::READWRITE,
                    ),
                ]
            });

            PROPERTIES.as_ref()
        }

        fn set_property(
            &self,
            _obj: &Self::Type,
            _id: usize,
            value: &glib::Value,
            pspec: &glib::ParamSpec,
        ) {
            match pspec.get_name() {
                "expanded" => {
                    let expanded: Option<bool> = value
                        .get()
                        .expect("type conformity checked by `Object::set_property`");
                    self.expanded.set(expanded.unwrap());
                }
                "display-name" => {
                    let name = value
                        .get()
                        .expect("type conformity checked by `Object::set_property`");
                    self.name.set(name.unwrap());
                }
                _ => unimplemented!(),
            }
        }

        fn get_property(
            &self,
            _obj: &Self::Type,
            _id: usize,
            pspec: &glib::ParamSpec,
        ) -> glib::Value {
            match pspec.get_name() {
                "display-name" => self.name.get().to_value(),
                "expanded" => self.expanded.get().to_value(),
                _ => unimplemented!(),
            }
        }
    }

    impl ListModelImpl for FrctlCategory {
        fn get_item_type(&self, _list_model: &Self::Type) -> glib::Type {
            FrctlRoom::static_type()
        }
        fn get_n_items(&self, _list_model: &Self::Type) -> u32 {
            self.list.borrow().len() as u32
        }
        fn get_item(&self, _list_model: &Self::Type, position: u32) -> Option<glib::Object> {
            let list = self.list.borrow();
            let room_id = list.get(position as usize);
            if let Some(room_id) = room_id {
                self.map
                    .borrow()
                    .get(&room_id)
                    .map(|(_, o)| o.clone().upcast::<glib::Object>())
            } else {
                None
            }
        }
    }
    impl SelectionModelImpl for FrctlCategory {
        fn get_selection_in_range(
            &self,
            _model: &Self::Type,
            _position: u32,
            _n_items: u32,
        ) -> gtk::Bitset {
            let result = gtk::Bitset::new_empty();
            if self.selected.get() != u32::MAX {
                result.add(self.selected.get());
            }
            result
        }

        fn is_selected(&self, _model: &Self::Type, position: u32) -> bool {
            self.selected.get() == position
        }

        fn select_item(&self, model: &Self::Type, position: u32, _unselect_rest: bool) -> bool {
            model.select(position);
            true
        }
    }
}

glib::wrapper! {
    pub struct FrctlCategory(ObjectSubclass<imp::FrctlCategory>)
        @implements gio::ListModel, gtk::SelectionModel;
}

// TODO: sort the rooms in FrctlCategory, i guess we want last active room first
impl FrctlCategory {
    pub fn new(client: Client, name: CategoryName) -> Self {
        let obj =
            glib::Object::new(&[("display-name", &name)]).expect("Failed to create FrctlCategory");
        // We don't need to set the client as a GObject property since it's used only internally
        let priv_ = imp::FrctlCategory::from_instance(&obj);
        priv_.client.set(client).unwrap();
        obj
    }

    pub fn select(&self, position: u32) {
        let priv_ = imp::FrctlCategory::from_instance(self);
        let old_position = priv_.selected.get();

        if position == old_position {
            return;
        }

        priv_.selected.set(position);

        if old_position == u32::MAX {
            self.selection_changed(position, 1);
        } else if position == u32::MAX {
            self.selection_changed(old_position, 1);
        } else if position < old_position {
            self.selection_changed(position, old_position - position + 1);
        } else {
            self.selection_changed(old_position, position - old_position + 1);
        }
    }

    pub fn unselect(&self) {
        self.select(u32::MAX);
    }

    pub fn update(&self, room_id: &RoomId) {
        let priv_ = imp::FrctlCategory::from_instance(self);
        let category_type = priv_.name.get().get_room_type();
        let client = priv_.client.get().unwrap();
        let room: Option<Room> = match category_type {
            RoomType::Invited => client.get_invited_room(room_id).map(Into::into),
            RoomType::Joined => client.get_joined_room(room_id).map(Into::into),
            RoomType::Left => client.get_left_room(room_id).map(Into::into),
        };

        let mut found = false;
        if let Some((_, room_obj)) = priv_.map.borrow().get(room_id) {
            if room.is_some() {
                room_obj.update();
                found = true;
            }
        }

        if found && room.is_none() {
            if let Some((position, _)) = priv_.map.borrow_mut().remove(&room_id.clone()) {
                priv_.list.borrow_mut().remove(position as usize);
                self.items_changed(position, 1, 0);
            }
        } else if !found {
            if let Some(room) = room {
                self.append(&room);
            }
        }
    }

    pub fn append(&self, room: &Room) {
        let priv_ = imp::FrctlCategory::from_instance(self);
        let room_id = room.room_id();
        let room_obj = FrctlRoom::new(room);
        let index = {
            let mut map = priv_.map.borrow_mut();
            let mut list = priv_.list.borrow_mut();
            let index = list.len();
            map.insert(room_id.clone(), (index as u32, room_obj));
            list.push(room_id.clone());
            index
        };
        self.items_changed(index as u32, 0, 1);
    }

    pub fn append_batch(&self, rooms: Vec<Room>) {
        let priv_ = imp::FrctlCategory::from_instance(self);
        let index = {
            let mut map = priv_.map.borrow_mut();
            let mut list = priv_.list.borrow_mut();
            let index = list.len();
            let mut position = index;
            for room in &rooms {
                let room_id = room.room_id();
                let room_obj = FrctlRoom::new(room);
                map.insert(room_id.clone(), (position as u32, room_obj));
                list.push(room_id.clone());
                position += 1;
            }
            index
        };
        self.items_changed(index as u32, 0, rooms.len() as u32);
    }
}
