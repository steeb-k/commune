use adw::{prelude::*, subclass::prelude::*};
use futures_channel::oneshot;
use gtk::glib;
use tracing::error;

use crate::{
    components::Avatar,
    prelude::*,
    session::{Room, RoomCategory, RoomCategoryFilter, Session},
};

mod imp {
    use std::cell::{OnceCell, RefCell};

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/components/dialogs/space_picker.ui")]
    #[properties(wrapper_type = super::SpacePickerDialog)]
    pub struct SpacePickerDialog {
        #[template_child]
        search_entry: TemplateChild<gtk::SearchEntry>,
        #[template_child]
        stack: TemplateChild<gtk::Stack>,
        #[template_child]
        list: TemplateChild<gtk::ListBox>,
        /// The current session.
        #[property(get, set = Self::set_session, construct_only)]
        session: glib::WeakRef<Session>,
        /// The space that was chosen, once one was.
        chosen: RefCell<Option<Room>>,
        /// The spaces to choose from, filtered and sorted.
        model: OnceCell<gtk::SortListModel>,
        /// The filter for the search term.
        search_filter: OnceCell<gtk::StringFilter>,
        /// The filtered model, kept so the exclusion can be re-applied.
        filtered: RefCell<Option<gtk::FilterListModel>>,
        /// The room to leave out of the list, if any.
        excluded: RefCell<Option<Room>>,
        /// The sender waiting for a choice.
        sender: RefCell<Option<oneshot::Sender<Option<Room>>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SpacePickerDialog {
        const NAME: &'static str = "SpacePickerDialog";
        type Type = super::SpacePickerDialog;
        type ParentType = adw::Dialog;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            Self::bind_template_callbacks(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for SpacePickerDialog {}

    impl WidgetImpl for SpacePickerDialog {}

    impl AdwDialogImpl for SpacePickerDialog {
        fn closed(&self) {
            // Whoever is waiting must be released whether a space was chosen or
            // the dialog was simply dismissed.
            let chosen = self.chosen.take();

            if let Some(sender) = self.sender.take() {
                let _ = sender.send(chosen);
            }

            self.parent_closed();
        }
    }

    #[gtk::template_callbacks]
    impl SpacePickerDialog {
        /// Set the current session.
        fn set_session(&self, session: &Session) {
            self.session.set(Some(session));

            let category_filter = RoomCategoryFilter::new();
            category_filter.set_expression(Some(Room::this_expression("category").upcast()));
            category_filter.set_room_category(RoomCategory::Space);

            let search_filter = gtk::StringFilter::builder()
                .expression(Room::this_expression("display-name"))
                .match_mode(gtk::StringFilterMatchMode::Substring)
                .ignore_case(true)
                .build();

            // A room cannot usefully be restricted to itself, and a space
            // cannot be put inside itself.
            let exclusion_filter = gtk::CustomFilter::new(glib::clone!(
                #[weak(rename_to = imp)]
                self,
                #[upgrade_or]
                true,
                move |item| {
                    let Some(excluded) = imp.excluded.borrow().clone() else {
                        return true;
                    };

                    item.downcast_ref::<Room>() != Some(&excluded)
                }
            ));

            let filter = gtk::EveryFilter::new();
            filter.append(category_filter);
            filter.append(search_filter.clone());
            filter.append(exclusion_filter);

            let by_name = gtk::StringSorter::builder()
                .expression(Room::this_expression("display-name"))
                .ignore_case(true)
                .build();

            let filtered = gtk::FilterListModel::new(Some(session.room_list()), Some(filter));
            let sorted = gtk::SortListModel::new(Some(filtered.clone()), Some(by_name));

            self.list.bind_model(Some(&sorted), |item| {
                let row = adw::ActionRow::builder().activatable(true).build();

                if let Some(room) = item.downcast_ref::<Room>() {
                    let avatar = Avatar::new();
                    avatar.set_size(32);
                    avatar.set_data(Some(room.avatar_data()));
                    row.add_prefix(&avatar);

                    room.bind_property("display-name", &row, "title")
                        .sync_create()
                        .build();

                    if let Some(alias) = room.aliases().alias_string().filter(|s| !s.is_empty()) {
                        row.set_subtitle(&alias);
                    }
                } else {
                    error!("Space picker list contains something else than a room: {item:?}");
                }

                row.upcast()
            });

            let _ = self.search_filter.set(search_filter);

            sorted.connect_items_changed(glib::clone!(
                #[weak(rename_to = imp)]
                self,
                move |_, _, _, _| {
                    imp.update_stack();
                }
            ));

            let _ = self.model.set(sorted);
            self.filtered.replace(Some(filtered));

            self.update_stack();
        }

        /// Set the room to leave out of the list.
        pub(super) fn set_excluded(&self, room: Option<&Room>) {
            self.excluded.replace(room.cloned());

            if let Some(filter) = self
                .filtered
                .borrow()
                .as_ref()
                .and_then(gtk::FilterListModel::filter)
            {
                filter.changed(gtk::FilterChange::Different);
            }
        }

        /// Wait for a space to be chosen, or for the dialog to be dismissed.
        pub(super) fn listen(&self) -> oneshot::Receiver<Option<Room>> {
            let (sender, receiver) = oneshot::channel();
            self.sender.replace(Some(sender));
            receiver
        }

        /// Update the list for the current search term.
        #[template_callback]
        fn update_search(&self) {
            let Some(filter) = self.search_filter.get() else {
                return;
            };

            let text = self.search_entry.text();
            filter.set_search(Some(text.as_str()).filter(|text| !text.is_empty()));

            self.update_stack();
        }

        /// Show the list, or the reason it is empty.
        fn update_stack(&self) {
            let is_searching = !self.search_entry.text().is_empty();
            let is_empty = self.list.first_child().is_none();

            let name = if !is_empty {
                "list"
            } else if is_searching {
                "no-matching"
            } else {
                "empty"
            };

            self.stack.set_visible_child_name(name);
        }

        /// Choose the space of the activated row.
        #[template_callback]
        fn row_activated(&self, row: &gtk::ListBoxRow) {
            // The rows are bound from the model, so a row's index in the list
            // is its position in it.
            let Some(room) = self
                .model
                .get()
                .and_then(|model| model.item(row.index().try_into().ok()?))
                .and_downcast::<Room>()
            else {
                return;
            };

            self.chosen.replace(Some(room));
            self.obj().close();
        }
    }
}

glib::wrapper! {
    /// A dialog to choose one of the spaces that were joined.
    pub struct SpacePickerDialog(ObjectSubclass<imp::SpacePickerDialog>)
        @extends gtk::Widget, adw::Dialog,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::ShortcutManager;
}

impl SpacePickerDialog {
    /// Ask the user to choose one of the spaces they have joined.
    ///
    /// `excluded` is left out of the list, for the cases where a room must not
    /// be offered itself.
    ///
    /// Returns `None` if the dialog was dismissed without a choice.
    pub(crate) async fn choose(
        parent: &impl IsA<gtk::Widget>,
        session: &Session,
        excluded: Option<&Room>,
    ) -> Option<Room> {
        let dialog = glib::Object::builder::<Self>()
            .property("session", session)
            .build();

        let imp = dialog.imp();
        imp.set_excluded(excluded);

        let receiver = imp.listen();
        dialog.present(Some(parent));

        receiver.await.ok().flatten()
    }
}
