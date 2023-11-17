use gtk::{gio, glib, glib::clone, prelude::*, subclass::prelude::*};
use tracing::error;

use crate::utils::BoundObject;

mod imp {
    use std::cell::RefCell;

    use once_cell::sync::Lazy;

    use super::*;

    #[derive(Debug, Default)]
    pub struct ExpressionListModel {
        pub model: BoundObject<gio::ListModel>,
        pub expressions: RefCell<Vec<gtk::Expression>>,
        pub watches: RefCell<Vec<Vec<gtk::ExpressionWatch>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ExpressionListModel {
        const NAME: &'static str = "ExpressionListModel";
        type Type = super::ExpressionListModel;
        type Interfaces = (gio::ListModel,);
    }

    impl ObjectImpl for ExpressionListModel {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![glib::ParamSpecObject::builder::<gio::ListModel>("model")
                    .read_only()
                    .build()]
            });

            PROPERTIES.as_ref()
        }

        fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            let obj = self.obj();

            match pspec.name() {
                "model" => obj.model().to_value(),
                _ => unimplemented!(),
            }
        }

        fn dispose(&self) {
            for watch in self.watches.take().iter().flatten() {
                watch.unwatch()
            }
        }
    }

    impl ListModelImpl for ExpressionListModel {
        fn item_type(&self) -> glib::Type {
            self.model
                .obj()
                .map(|m| m.item_type())
                .unwrap_or_else(glib::Object::static_type)
        }

        fn n_items(&self) -> u32 {
            self.model.obj().map(|m| m.n_items()).unwrap_or_default()
        }

        fn item(&self, position: u32) -> Option<glib::Object> {
            self.model.obj().and_then(|m| m.item(position))
        }
    }
}

glib::wrapper! {
    /// A list model that signals an item as changed when the expression's value changes.
    pub struct ExpressionListModel(ObjectSubclass<imp::ExpressionListModel>)
        @implements gio::ListModel;
}

impl ExpressionListModel {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// The underlying model.
    pub fn model(&self) -> Option<gio::ListModel> {
        self.imp().model.obj()
    }

    /// Set the underlying model.
    pub fn set_model(&self, model: Option<gio::ListModel>) {
        let imp = self.imp();

        let removed = self.n_items();

        imp.model.disconnect_signals();
        for watch in imp.watches.take().iter().flatten() {
            watch.unwatch();
        }

        let added = if let Some(model) = model {
            let items_changed_handler = model.connect_items_changed(
                clone!(@weak self as obj => move |_, pos, removed, added| {
                    obj.watch_items(pos, removed, added);
                    obj.items_changed(pos, removed, added);
                }),
            );

            let added = model.n_items();
            imp.model.set(model, vec![items_changed_handler]);

            self.watch_items(0, 0, added);
            added
        } else {
            0
        };

        self.items_changed(0, removed, added);
        self.notify("model");
    }

    /// The expressions to watch.
    pub fn expressions(&self) -> Vec<gtk::Expression> {
        self.imp().expressions.borrow().clone()
    }

    /// Set the expressions to watch.
    pub fn set_expressions(&self, expressions: Vec<gtk::Expression>) {
        let imp = self.imp();

        for watch in imp.watches.take().iter().flatten() {
            watch.unwatch();
        }

        imp.expressions.replace(expressions);
        self.watch_items(0, 0, self.n_items());
    }

    /// Watch and unwatch items according to changes in the underlying model.
    fn watch_items(&self, pos: u32, removed: u32, added: u32) {
        let Some(model) = self.model() else {
            return;
        };

        let expressions = self.expressions();
        if expressions.is_empty() {
            return;
        }

        let imp = self.imp();

        let mut new_watches = Vec::with_capacity(added as usize);
        for item_pos in pos..pos + added {
            let Some(item) = model.item(item_pos) else {
                error!("Out of bounds item");
                break;
            };

            let mut item_watches = Vec::with_capacity(expressions.len());
            for expression in &expressions {
                item_watches.push(expression.watch(
                    Some(&item),
                    clone!(@weak self as obj, @weak item => move || {
                        obj.item_expr_changed(&item);
                    }),
                ));
            }

            new_watches.push(item_watches);
        }

        let mut watches = imp.watches.borrow_mut();
        let removed_range = (pos as usize)..((pos + removed) as usize);
        for watch in watches.splice(removed_range, new_watches).flatten() {
            watch.unwatch();
        }
    }

    fn item_expr_changed(&self, item: &glib::Object) {
        let Some(model) = self.model() else {
            return;
        };

        for (pos, obj) in model.snapshot().iter().enumerate() {
            if obj == item {
                self.items_changed(pos as u32, 1, 1);
                break;
            }
        }
    }
}

impl Default for ExpressionListModel {
    fn default() -> Self {
        Self::new()
    }
}
