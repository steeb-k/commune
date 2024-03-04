use adw::{prelude::*, subclass::prelude::*};
use geo_uri::GeoUri;
use gtk::{gdk_pixbuf, gio, glib, CompositeTemplate};
use shumate::prelude::*;

use crate::i18n::gettext_f;

mod imp {
    use std::cell::Cell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/components/location_viewer.ui")]
    #[properties(wrapper_type = super::LocationViewer)]
    pub struct LocationViewer {
        #[template_child]
        pub map: TemplateChild<shumate::SimpleMap>,
        #[template_child]
        pub marker_img: TemplateChild<gtk::Image>,
        pub marker: shumate::Marker,
        /// Whether to display this location in a compact format.
        #[property(get, set = Self::set_compact, explicit_notify)]
        pub compact: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for LocationViewer {
        const NAME: &'static str = "ComponentsLocationViewer";
        type Type = super::LocationViewer;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);

            klass.set_css_name("location-viewer");
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for LocationViewer {
        fn constructed(&self) {
            self.marker.set_child(Some(&*self.marker_img));

            let style = gio::resources_lookup_data(
                "/org/gnome/Fractal/mapstyle/osm-liberty/style.json",
                gio::ResourceLookupFlags::NONE,
            )
            .expect("Could not load map style");
            let source =
                shumate::VectorRenderer::new("vector-tiles", &String::from_utf8_lossy(&style))
                    .expect("Could not read map style");
            source.set_license("© OpenMapTiles © OpenStreetMap contributors");
            source.set_license_uri("https://www.openstreetmap.org/copyright");

            let spritepixbuf = gdk_pixbuf::Pixbuf::from_resource(
                "/org/gnome/Fractal/mapstyle/osm-liberty/sprites.png",
            )
            .expect("Could not load map sprites");
            let spritejson = gio::resources_lookup_data(
                "/org/gnome/Fractal/mapstyle/osm-liberty/sprites.json",
                gio::ResourceLookupFlags::NONE,
            )
            .expect("Could not load map sprite sheet");
            source
                .set_sprite_sheet_data(&spritepixbuf, &String::from_utf8_lossy(&spritejson))
                .expect("Could not set map sprite sheet");

            self.map.set_map_source(Some(&source));

            let viewport = self.map.viewport().unwrap();
            viewport.set_zoom_level(12.0);
            let marker_layer = shumate::MarkerLayer::new(&viewport);
            marker_layer.add_marker(&self.marker);
            self.map.add_overlay_layer(&marker_layer);

            // Hide the scale.
            self.map.scale().unwrap().set_visible(false);
            self.parent_constructed();
        }
    }

    impl WidgetImpl for LocationViewer {}
    impl BinImpl for LocationViewer {}

    impl LocationViewer {
        /// Set the compact format of this location.
        fn set_compact(&self, compact: bool) {
            if self.compact.get() == compact {
                return;
            }

            self.map.set_show_zoom_buttons(!compact);
            if let Some(license) = self.map.license() {
                license.set_visible(!compact);
            }

            self.compact.set(compact);
            self.obj().notify_compact();
        }
    }
}

glib::wrapper! {
    /// A widget displaying a location.
    pub struct LocationViewer(ObjectSubclass<imp::LocationViewer>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl LocationViewer {
    /// Create a new location message.
    pub fn new() -> Self {
        glib::Object::new()
    }

    // Move the map viewport to the provided coordinates and draw a marker.
    pub fn set_location(&self, geo_uri: &GeoUri) {
        let imp = self.imp();
        let latitude = geo_uri.latitude();
        let longitude = geo_uri.longitude();

        imp.map
            .viewport()
            .unwrap()
            .set_location(latitude, longitude);
        imp.marker.set_location(latitude, longitude);

        self.update_property(&[gtk::accessible::Property::Description(&gettext_f(
            "Location at latitude {latitude} and longitude {longitude}",
            &[
                ("latitude", &latitude.to_string()),
                ("longitude", &longitude.to_string()),
            ],
        ))]);
    }
}
