use adw::{prelude::*, subclass::prelude::*};
use gtk::{CompositeTemplate, glib, glib::clone, pango};

use crate::{components::LoadingBin, utils::BoundObject};

mod imp {
    use std::{
        cell::{Cell, RefCell},
        marker::PhantomData,
    };

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/components/rows/combo_loading_row.ui")]
    #[properties(wrapper_type = super::ComboLoadingRow)]
    pub struct ComboLoadingRow {
        #[template_child]
        loading_bin: TemplateChild<LoadingBin>,
        #[template_child]
        popover: TemplateChild<gtk::Popover>,
        #[template_child]
        list: TemplateChild<gtk::ListBox>,
        /// The string model to build the list.
        #[property(get, set = Self::set_string_model, explicit_notify, nullable)]
        string_model: BoundObject<gtk::StringList>,
        /// The position of the selected string.
        #[property(get, default = gtk::INVALID_LIST_POSITION)]
        selected: Cell<u32>,
        /// The selected string.
        #[property(get, set = Self::set_selected_string, explicit_notify, nullable)]
        selected_string: RefCell<Option<String>>,
        /// Whether the row is loading.
        #[property(get = Self::is_loading, set = Self::set_is_loading)]
        is_loading: PhantomData<bool>,
        /// Whether the row is read-only.
        #[property(get, set = Self::set_read_only, explicit_notify)]
        read_only: Cell<bool>,
        selected_handlers: RefCell<Vec<glib::SignalHandlerId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ComboLoadingRow {
        const NAME: &'static str = "ComboLoadingRow";
        type Type = super::ComboLoadingRow;
        type ParentType = adw::ActionRow;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            Self::bind_template_callbacks(klass);

            klass.set_accessible_role(gtk::AccessibleRole::ComboBox);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for ComboLoadingRow {}

    impl WidgetImpl for ComboLoadingRow {}
    impl ListBoxRowImpl for ComboLoadingRow {}
    impl PreferencesRowImpl for ComboLoadingRow {}

    impl ActionRowImpl for ComboLoadingRow {
        fn activate(&self) {
            if !self.is_loading() {
                self.popover.popup();
            }
        }
    }

    #[gtk::template_callbacks]
    impl ComboLoadingRow {
        /// Set the string model to build the list.
        fn set_string_model(&self, model: Option<gtk::StringList>) {
            if self.string_model.obj() == model {
                return;
            }
            let obj = self.obj();

            for handler in self.selected_handlers.take() {
                obj.disconnect(handler);
            }
            self.string_model.disconnect_signals();

            self.list.bind_model(
                model.as_ref(),
                clone!(
                    #[weak]
                    obj,
                    #[upgrade_or_else]
                    || { gtk::ListBoxRow::new().upcast() },
                    move |item| {
                        let Some(item) = item.downcast_ref::<gtk::StringObject>() else {
                            return gtk::ListBoxRow::new().upcast();
                        };

                        let string = item.string();
                        let child = gtk::Box::new(gtk::Orientation::Horizontal, 6);

                        let label = gtk::Label::builder()
                            .xalign(0.0)
                            .ellipsize(pango::EllipsizeMode::End)
                            .max_width_chars(40)
                            .valign(gtk::Align::Center)
                            .label(string)
                            .build();
                        child.append(&label);

                        let icon = gtk::Image::builder()
                            .accessible_role(gtk::AccessibleRole::Presentation)
                            .icon_name("object-select-symbolic")
                            .build();

                        let selected_handler = obj.connect_selected_string_notify(clone!(
                            #[weak]
                            label,
                            #[weak]
                            icon,
                            move |obj| {
                                let is_selected =
                                    obj.selected_string().is_some_and(|s| s == label.label());
                                let opacity = if is_selected { 1.0 } else { 0.0 };
                                icon.set_opacity(opacity);
                            }
                        ));
                        obj.imp()
                            .selected_handlers
                            .borrow_mut()
                            .push(selected_handler);

                        let is_selected = obj.selected_string().is_some_and(|s| s == label.label());
                        let opacity = if is_selected { 1.0 } else { 0.0 };
                        icon.set_opacity(opacity);
                        child.append(&icon);

                        gtk::ListBoxRow::builder().child(&child).build().upcast()
                    }
                ),
            );

            if let Some(model) = model {
                let items_changed_handler = model.connect_items_changed(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_, _, _, _| {
                        imp.update_selected();
                    }
                ));

                self.string_model.set(model, vec![items_changed_handler]);
            }

            self.update_selected();
            obj.notify_string_model();
        }

        /// Set whether the row is loading.
        fn set_selected_string(&self, string: Option<String>) {
            if *self.selected_string.borrow() == string {
                return;
            }
            let obj = self.obj();

            obj.set_subtitle(string.as_deref().unwrap_or_default());
            self.selected_string.replace(string);

            self.update_selected();
            obj.notify_selected_string();
        }

        /// Update the position of the selected string.
        fn update_selected(&self) {
            let mut selected = gtk::INVALID_LIST_POSITION;

            if let Some((string_model, selected_string)) = self
                .string_model
                .obj()
                .zip(self.selected_string.borrow().clone())
            {
                for (pos, item) in string_model.iter::<glib::Object>().enumerate() {
                    let Some(item) = item.ok().and_downcast::<gtk::StringObject>() else {
                        // The iterator is broken.
                        break;
                    };

                    if item.string() == selected_string {
                        selected = pos as u32;
                        break;
                    }
                }
            }

            if self.selected.get() == selected {
                return;
            }

            self.selected.set(selected);
            self.obj().notify_selected();
        }

        /// Whether the row is loading.
        fn is_loading(&self) -> bool {
            self.loading_bin.is_loading()
        }

        /// Set whether the row is loading.
        fn set_is_loading(&self, loading: bool) {
            if self.is_loading() == loading {
                return;
            }

            self.loading_bin.set_is_loading(loading);
            self.obj().notify_is_loading();
        }

        /// Set whether the row is read-only.
        fn set_read_only(&self, read_only: bool) {
            if self.read_only.get() == read_only {
                return;
            }
            let obj = self.obj();

            self.read_only.set(read_only);

            obj.update_property(&[gtk::accessible::Property::ReadOnly(read_only)]);
            obj.notify_read_only();
        }

        /// A row was activated.
        #[template_callback]
        fn row_activated(&self, row: &gtk::ListBoxRow) {
            let Some(string) = row
                .child()
                .and_downcast::<gtk::Box>()
                .and_then(|b| b.first_child())
                .and_downcast::<gtk::Label>()
                .map(|l| l.label())
            else {
                return;
            };

            self.popover.popdown();
            self.set_selected_string(Some(string.into()));
        }

        /// The popover's visibility changed.
        #[template_callback]
        fn popover_visible(&self) {
            let obj = self.obj();
            let is_visible = self.popover.is_visible();

            if is_visible {
                obj.add_css_class("has-open-popup");
            } else {
                obj.remove_css_class("has-open-popup");
            }
        }
    }
}

glib::wrapper! {
    /// An `AdwActionRow` behaving like a combo box, with a loading state.
    pub struct ComboLoadingRow(ObjectSubclass<imp::ComboLoadingRow>)
        @extends gtk::Widget, gtk::ListBoxRow, adw::PreferencesRow, adw::ActionRow,
        @implements gtk::Actionable, gtk::Accessible;
}

impl ComboLoadingRow {
    pub fn new() -> Self {
        glib::Object::new()
    }
}
