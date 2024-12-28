use adw::subclass::prelude::*;
use gettextrs::gettext;
use gtk::{glib, prelude::*, CompositeTemplate};

use crate::session::model::VirtualItemKind;

mod imp {
    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(resource = "/org/gnome/Fractal/ui/session/view/content/room_history/divider_row.ui")]
    pub struct DividerRow {
        #[template_child]
        inner_label: TemplateChild<gtk::Label>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DividerRow {
        const NAME: &'static str = "ContentDividerRow";
        type Type = super::DividerRow;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);

            klass.set_css_name("divider-row");
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for DividerRow {}

    impl WidgetImpl for DividerRow {}
    impl BinImpl for DividerRow {}

    impl DividerRow {
        /// Set the kind of this divider.
        ///
        /// Panics if the kind is not `TimelineStart`, `DayDivider` or
        /// `NewMessages`.
        pub(super) fn set_kind(&self, kind: &VirtualItemKind) {
            let label = match kind {
                VirtualItemKind::TimelineStart => {
                    gettext("This is the start of the visible history")
                }
                VirtualItemKind::DayDivider(utc_date) => {
                    let date = utc_date.to_local().unwrap_or(utc_date.clone());

                    let fmt = if date.year()
                        == glib::DateTime::now_local()
                            .expect("we should be able to get the local datetime")
                            .year()
                    {
                        // Translators: This is a date format in the day divider without the
                        // year. For example, "Friday, May 5".
                        // Please use `-` before specifiers that add spaces on single
                        // digits. See `man strftime` or the documentation of g_date_time_format for the available specifiers: <https://docs.gtk.org/glib/method.DateTime.format.html>
                        gettext("%A, %B %-e")
                    } else {
                        // Translators: This is a date format in the day divider with the
                        // year. For ex. "Friday, May 5, 2023".
                        // Please use `-` before specifiers that add spaces on single
                        // digits. See `man strftime` or the documentation of g_date_time_format for the available specifiers: <https://docs.gtk.org/glib/method.DateTime.format.html>
                        gettext("%A, %B %-e, %Y")
                    };

                    date.format(&fmt)
                        .expect("we should be able to format the datetime")
                        .into()
                }
                VirtualItemKind::NewMessages => gettext("New Messages"),
                _ => unimplemented!(),
            };

            let obj = self.obj();
            if matches!(kind, VirtualItemKind::NewMessages) {
                obj.add_css_class("new-messages");
            } else {
                obj.remove_css_class("new-messages");
            }

            self.inner_label.set_label(&label);
        }
    }
}

glib::wrapper! {
    /// A row presenting a divider in the timeline.
    pub struct DividerRow(ObjectSubclass<imp::DividerRow>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl DividerRow {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Set the kind of this divider.
    ///
    /// Panics if the kind is not `TimelineStart`, `DayDivider` or
    /// `NewMessages`.
    pub(crate) fn set_kind(&self, kind: &VirtualItemKind) {
        self.imp().set_kind(kind);
    }
}
