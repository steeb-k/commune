use std::collections::BTreeMap;

use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{CompositeTemplate, gio, glib, glib::clone};
use ruma::{assign, events::room::ImageInfo};
use tracing::{debug, error};

mod image_row;

use self::image_row::PackImageRow;
use crate::{
    components::LoadingButton,
    gettext_f,
    session::{
        ImagePack, ImagePackSource, PackContent, PackImage, PackImageData, PackUsage,
        SHORTCODE_MAX_LEN, Session,
    },
    spawn_tokio, toast,
    utils::{
        SingleItemListModel,
        media::{FileInfo, image::ImageInfoLoader},
    },
};

/// The usages that the pack can be restricted to, in the order of the rows of
/// the combo.
const USAGES: &[Option<PackUsage>] = &[None, Some(PackUsage::Sticker), Some(PackUsage::Emoticon)];

/// The shortcode to fall back to when a file name has nothing usable in it.
const FALLBACK_SHORTCODE: &str = "image";

/// Build a shortcode from the name of a file.
///
/// Every character that the grammar does not allow becomes an underscore, so
/// that the result is at least recognizable.
fn shortcode_from_file_name(name: &str) -> String {
    let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);

    let shortcode = stem
        .chars()
        .take(SHORTCODE_MAX_LEN)
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();

    if shortcode.is_empty() {
        FALLBACK_SHORTCODE.to_owned()
    } else {
        shortcode
    }
}

mod imp {
    use std::cell::{Cell, OnceCell, RefCell};

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/components/image_pack_editor/mod.ui")]
    #[properties(wrapper_type = super::ImagePackEditor)]
    pub struct ImagePackEditor {
        #[template_child]
        save_button: TemplateChild<LoadingButton>,
        #[template_child]
        add_button: TemplateChild<LoadingButton>,
        #[template_child]
        name_entry: TemplateChild<adw::EntryRow>,
        #[template_child]
        attribution_entry: TemplateChild<adw::EntryRow>,
        #[template_child]
        usage_row: TemplateChild<adw::ComboRow>,
        #[template_child]
        images_group: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        placeholder_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        delete_group: TemplateChild<adw::PreferencesGroup>,
        /// The session that the pack belongs to.
        #[property(get, construct_only)]
        pub(super) session: OnceCell<Session>,
        /// Where the pack is, or will be, stored.
        pub(super) source: OnceCell<ImagePackSource>,
        /// Whether the pack does not exist yet.
        pub(super) is_new: Cell<bool>,
        /// The content that the pack was opened with.
        ///
        /// Only the parts that this editor does not present are kept from it,
        /// so that they are not dropped when the pack is saved.
        pub(super) content: RefCell<PackContent>,
        /// The rows of the images, which are what is saved.
        rows: RefCell<Vec<PackImageRow>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ImagePackEditor {
        const NAME: &'static str = "ImagePackEditor";
        type Type = super::ImagePackEditor;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            LoadingButton::ensure_type();

            Self::bind_template(klass);
            Self::bind_template_callbacks(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for ImagePackEditor {}

    impl WidgetImpl for ImagePackEditor {}
    impl NavigationPageImpl for ImagePackEditor {}

    #[gtk::template_callbacks]
    impl ImagePackEditor {
        /// Present the pack that this editor was built with.
        pub(super) fn init(&self, is_new: bool) {
            let content = self.content.borrow().clone();
            let obj = self.obj();

            obj.set_title(&if is_new {
                gettext("New Pack")
            } else {
                gettext("Edit Pack")
            });

            if let Some(display_name) = &content.pack.display_name {
                self.name_entry.set_text(display_name);
            }
            if let Some(attribution) = &content.pack.attribution {
                self.attribution_entry.set_text(attribution);
            }

            // A pack that declares both usages, or neither, is usable
            // everywhere, so both are the first row of the combo.
            let selected = match (
                content.pack.usage.contains(&PackUsage::Sticker),
                content.pack.usage.contains(&PackUsage::Emoticon),
            ) {
                (true, false) => 1,
                (false, true) => 2,
                _ => 0,
            };
            self.usage_row.set_selected(selected);

            // A pack that does not exist yet cannot be deleted.
            self.delete_group.set_visible(!is_new);

            let session = self.session.get().expect("session should be initialized");
            for (shortcode, data) in &content.images {
                let image = PackImage::new(shortcode.clone(), data.clone());
                self.add_row(&PackImageRow::new(session, &image));
            }

            self.update_placeholder();
        }

        /// Add the given row to the list of images.
        fn add_row(&self, row: &PackImageRow) {
            row.connect_removed(clone!(
                #[weak(rename_to = imp)]
                self,
                move |row| {
                    imp.remove_row(row);
                }
            ));

            self.images_group.add(row);
            self.rows.borrow_mut().push(row.clone());
            self.update_placeholder();
        }

        /// Remove the given row from the list of images.
        fn remove_row(&self, row: &PackImageRow) {
            self.images_group.remove(row);
            self.rows.borrow_mut().retain(|other| other != row);
            self.update_placeholder();
        }

        /// Update whether the placeholder of the list of images is presented.
        fn update_placeholder(&self) {
            self.placeholder_row
                .set_visible(self.rows.borrow().is_empty());
        }

        /// Choose images to add to the pack, and upload them.
        #[template_callback]
        async fn add_images(&self) {
            let obj = self.obj();

            let image_filter = gtk::FileFilter::new();
            image_filter.set_name(Some(&gettext("Images")));
            image_filter.add_mime_type("image/*");
            let filters = SingleItemListModel::new(Some(&image_filter));

            let dialog = gtk::FileDialog::builder()
                .title(gettext("Choose Images"))
                .modal(true)
                .accept_label(gettext("Choose"))
                .filters(&filters)
                .build();

            let files = match dialog
                .open_multiple_future(obj.root().and_downcast_ref::<gtk::Window>())
                .await
            {
                Ok(files) => files,
                Err(error) => {
                    if error.matches(gtk::DialogError::Dismissed) {
                        debug!("File dialog dismissed by user");
                    } else {
                        error!("Could not open image files: {error:?}");
                        toast!(obj, gettext("Could not open the files"));
                    }
                    return;
                }
            };

            self.add_button.set_is_loading(true);

            for file in files.iter::<glib::Object>() {
                let Some(file) = file.ok().and_downcast::<gio::File>() else {
                    continue;
                };

                self.upload_image(file).await;
            }

            self.add_button.set_is_loading(false);
        }

        /// Upload the given file and add it to the pack.
        ///
        /// The upload happens as soon as the file is chosen, rather than when
        /// the pack is saved, so that the image can be presented. Leaving the
        /// editor without saving therefore leaves the media on the homeserver,
        /// where nothing points to it.
        async fn upload_image(&self, file: gio::File) {
            let obj = self.obj();

            let name = file
                .basename()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();

            let info = match FileInfo::try_from_file(&file).await {
                Ok(info) => info,
                Err(error) => {
                    error!("Could not load the info of an image of a pack: {error}");
                    toast!(obj, could_not_add(&name));
                    return;
                }
            };

            let data = match file.load_contents_future().await {
                Ok((data, _)) => data,
                Err(error) => {
                    error!("Could not load an image of a pack: {error}");
                    toast!(obj, could_not_add(&name));
                    return;
                }
            };

            let base_image_info = ImageInfoLoader::from(file).load_info().await;
            let image_info = assign!(ImageInfo::new(), {
                width: base_image_info.width,
                height: base_image_info.height,
                size: info.size.map(Into::into),
                mimetype: Some(info.mime.to_string()),
            });

            let session = self.session.get().expect("session should be initialized");
            let client = session.client();
            let handle =
                spawn_tokio!(
                    async move { client.media().upload(&info.mime, data.into(), None).await }
                );

            let uri = match handle.await.expect("task was not aborted") {
                Ok(response) => response.content_uri,
                Err(error) => {
                    error!("Could not upload an image of a pack: {error}");
                    toast!(obj, could_not_add(&name));
                    return;
                }
            };

            let mut data = PackImageData::new(uri);
            data.info = Some(Box::new(image_info));

            let image = PackImage::new(self.unique_shortcode(&name), data);
            self.add_row(&PackImageRow::new(session, &image));
        }

        /// A shortcode built from the given file name that no row uses yet.
        fn unique_shortcode(&self, file_name: &str) -> String {
            let base = shortcode_from_file_name(file_name);
            let taken = self
                .rows
                .borrow()
                .iter()
                .map(PackImageRow::shortcode)
                .collect::<Vec<_>>();

            if !taken.contains(&base) {
                return base;
            }

            // The grammar allows a hundred characters, so make room for the
            // suffix rather than going over it.
            let base = base.chars().take(SHORTCODE_MAX_LEN - 8).collect::<String>();

            // Among the shortcodes that are taken, at most all of them can
            // collide, so one of this many suffixes is free.
            (2..=taken.len() + 2)
                .map(|index| format!("{base}_{index}"))
                .find(|shortcode| !taken.contains(shortcode))
                .expect("an unused shortcode should be found")
        }

        /// The images of the pack, if every shortcode is usable.
        fn images(&self) -> Option<BTreeMap<String, PackImageData>> {
            let rows = self.rows.borrow();
            let mut images = BTreeMap::new();
            let mut is_valid = true;

            for row in rows.iter() {
                let shortcode = row.shortcode();

                if !row.is_shortcode_valid() {
                    is_valid = false;
                    continue;
                }

                let duplicate = images.insert(shortcode, row.data()).is_some();
                row.set_shortcode_duplicate(duplicate);

                if duplicate {
                    is_valid = false;
                }
            }

            is_valid.then_some(images)
        }

        /// Save the pack.
        #[template_callback]
        async fn save(&self) {
            let obj = self.obj();

            if self.rows.borrow().is_empty() {
                toast!(obj, gettext("Add at least one image to the pack"));
                return;
            }

            let Some(images) = self.images() else {
                toast!(
                    obj,
                    gettext(
                        "Every shortcode must be different, and can only contain letters, digits, dashes and underscores"
                    )
                );
                return;
            };

            let mut content = self.content.borrow().clone();
            content.images = images;
            content.pack.display_name = non_empty(&self.name_entry.text());
            content.pack.attribution = non_empty(&self.attribution_entry.text());

            // The usages that we do not know about are not presented, and are
            // left where they were.
            content.pack.usage.remove(&PackUsage::Sticker);
            content.pack.usage.remove(&PackUsage::Emoticon);
            if let Some(usage) = USAGES
                .get(self.usage_row.selected() as usize)
                .and_then(Option::as_ref)
            {
                content.pack.usage.insert(usage.clone());
            }

            let source = self.source.get().expect("source should be initialized");
            let image_packs = self
                .session
                .get()
                .expect("session should be initialized")
                .image_packs();

            self.save_button.set_is_loading(true);
            let result = image_packs.save_pack(source, content).await;

            if result.is_ok() && self.is_new.get() {
                // A pack is only usable in the room it lives in, and a pack
                // that was just created lives in a room that exists for that,
                // so it would be usable nowhere the user meant.
                let _ = image_packs
                    .set_pack_enabled(source.room.room_id(), &source.state_key, true)
                    .await;
                self.is_new.set(false);
            }

            self.save_button.set_is_loading(false);

            if result.is_err() {
                toast!(obj, gettext("Could not save the pack"));
                return;
            }

            obj.pop();
        }

        /// Delete the pack, after asking for confirmation.
        #[template_callback]
        async fn delete(&self) {
            let obj = self.obj();

            let confirm_dialog = adw::AlertDialog::builder()
                .default_response("cancel")
                .heading(gettext("Delete Pack?"))
                .body(gettext(
                    "The pack will not be available in any room anymore. Its images are not removed from the server.",
                ))
                .build();
            confirm_dialog.add_responses(&[
                ("cancel", &gettext("Cancel")),
                ("delete", &gettext("Delete")),
            ]);
            confirm_dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);

            if confirm_dialog.choose_future(Some(&*obj)).await != "delete" {
                return;
            }

            let source = self.source.get().expect("source should be initialized");
            let result = self
                .session
                .get()
                .expect("session should be initialized")
                .image_packs()
                .delete_pack(source)
                .await;

            if result.is_err() {
                toast!(obj, gettext("Could not delete the pack"));
                return;
            }

            obj.pop();
        }
    }
}

/// The error to present when an image could not be added to a pack.
fn could_not_add(name: &str) -> String {
    gettext_f(
        // Translators: Do NOT translate the content between '{' and '}', this
        // is a variable name.
        "Could not add “{name}” to the pack",
        &[("name", name)],
    )
}

/// The given text, if it is not only whitespace.
fn non_empty(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

glib::wrapper! {
    /// A page to create or edit an image pack.
    pub struct ImagePackEditor(ObjectSubclass<imp::ImagePackEditor>)
        @extends gtk::Widget, adw::NavigationPage,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl ImagePackEditor {
    /// Create an `ImagePackEditor` for a new pack at the given source.
    pub(crate) fn create(session: &Session, source: ImagePackSource) -> Self {
        Self::build(session, source, PackContent::default(), true)
    }

    /// Create an `ImagePackEditor` for the given existing pack.
    pub(crate) fn edit(session: &Session, pack: &ImagePack) -> Self {
        Self::build(session, pack.source().clone(), pack.content(), false)
    }

    /// Create an `ImagePackEditor` with the given source and content.
    fn build(
        session: &Session,
        source: ImagePackSource,
        content: PackContent,
        is_new: bool,
    ) -> Self {
        let obj = glib::Object::builder::<Self>()
            .property("session", session)
            .build();

        let imp = obj.imp();
        imp.source.set(source).expect("source is not initialized");
        imp.content.replace(content);
        imp.is_new.set(is_new);
        imp.init(is_new);

        obj
    }

    /// Go back to the page that this one was opened from.
    fn pop(&self) {
        if let Some(view) = self
            .ancestor(adw::NavigationView::static_type())
            .and_downcast::<adw::NavigationView>()
        {
            view.pop();
        }
    }
}
