use gtk::{gio, glib, glib::clone, prelude::*, subclass::prelude::*};

use crate::{
    components::PillSource,
    session::{EmoticonSource, ImagePacks, PackImage, PackUsage, Room},
    spawn,
};

mod imp {
    use std::{
        cell::{OnceCell, RefCell},
        collections::HashMap,
    };

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::CompletionEmoticonList)]
    pub struct CompletionEmoticonList {
        /// The room whose emoticons are completed.
        #[property(get, set = Self::set_room, explicit_notify, nullable)]
        room: RefCell<Option<Room>>,
        /// Every emoticon that can be sent in the room.
        emoticons: OnceCell<gio::ListStore>,
        /// The filter on the shortcode.
        pub(super) search_filter: gtk::StringFilter,
        /// The emoticons matching the current search.
        #[property(get)]
        list: gtk::FilterListModel,
        image_packs_handler: RefCell<Option<(ImagePacks, glib::SignalHandlerId)>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CompletionEmoticonList {
        const NAME: &'static str = "ContentCompletionEmoticonList";
        type Type = super::CompletionEmoticonList;
    }

    #[glib::derived_properties]
    impl ObjectImpl for CompletionEmoticonList {
        fn constructed(&self) {
            self.parent_constructed();

            // A shortcode is completed from its beginning.
            self.search_filter
                .set_expression(Some(&PillSource::this_expression("display-name")));
            self.search_filter
                .set_match_mode(gtk::StringFilterMatchMode::Prefix);
            self.search_filter.set_ignore_case(true);

            self.list.set_model(Some(self.emoticons()));
            self.list.set_filter(Some(&self.search_filter));
        }

        fn dispose(&self) {
            if let Some((image_packs, handler)) = self.image_packs_handler.take() {
                image_packs.disconnect(handler);
            }
        }
    }

    impl CompletionEmoticonList {
        /// Every emoticon that can be sent in the room.
        fn emoticons(&self) -> &gio::ListStore {
            self.emoticons
                .get_or_init(gio::ListStore::new::<EmoticonSource>)
        }

        /// Set the room whose emoticons are completed.
        fn set_room(&self, room: Option<Room>) {
            if *self.room.borrow() == room {
                return;
            }

            if let Some((image_packs, handler)) = self.image_packs_handler.take() {
                image_packs.disconnect(handler);
            }

            if let Some(image_packs) = room
                .as_ref()
                .and_then(Room::session)
                .map(|s| s.image_packs())
            {
                let handler = image_packs.connect_changed(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.load();
                    }
                ));
                self.image_packs_handler
                    .replace(Some((image_packs, handler)));
            }

            self.room.replace(room);
            self.load();

            self.obj().notify_room();
        }

        /// Load the emoticons of the current room.
        fn load(&self) {
            spawn!(clone!(
                #[weak(rename_to = imp)]
                self,
                async move {
                    imp.load_inner().await;
                }
            ));
        }

        /// Load the emoticons of the current room.
        async fn load_inner(&self) {
            let Some(room) = self.room.borrow().clone() else {
                self.emoticons().remove_all();
                return;
            };
            let Some(session) = room.session() else {
                self.emoticons().remove_all();
                return;
            };

            let packs = session
                .image_packs()
                .packs_for_room(&room, Some(&PackUsage::Emoticon))
                .await;

            // The room might have changed while we were loading.
            if self.room.borrow().as_ref() != Some(&room) {
                return;
            }

            let mut images = Vec::new();
            for pack in packs {
                let pack_images = pack.images();

                for position in 0..pack_images.n_items() {
                    let Some(image) = pack_images.item(position).and_downcast::<PackImage>() else {
                        continue;
                    };

                    images.push((pack.display_name(), image));
                }
            }

            // Several packs can define the same shortcode, and the
            // specification asks for those to be told apart by the name of
            // their pack rather than silently resolved.
            let mut counts = HashMap::<String, usize>::new();
            for (_, image) in &images {
                *counts.entry(image.shortcode()).or_default() += 1;
            }

            let emoticons = images
                .iter()
                .map(|(pack_name, image)| {
                    let is_ambiguous = counts.get(&image.shortcode()).is_some_and(|c| *c > 1);
                    EmoticonSource::new(&session, image, is_ambiguous.then_some(pack_name.as_str()))
                })
                .collect::<Vec<_>>();

            self.emoticons()
                .splice(0, self.emoticons().n_items(), &emoticons);
        }
    }
}

glib::wrapper! {
    /// The emoticons that can be completed in the composer.
    pub struct CompletionEmoticonList(ObjectSubclass<imp::CompletionEmoticonList>);
}

impl CompletionEmoticonList {
    /// Set the search term to filter the emoticons with.
    pub(super) fn set_search_term(&self, term: Option<&str>) {
        self.imp().search_filter.set_search(term);
    }
}

impl Default for CompletionEmoticonList {
    fn default() -> Self {
        glib::Object::new()
    }
}
