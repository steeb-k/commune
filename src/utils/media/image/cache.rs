//! An in-memory cache of decoded image textures.
//!
//! Decoding an image goes through a glycin sandbox, which means spawning a
//! subprocess for every single image. Without this cache, scrolling back over
//! images that were already shown pays that cost again every time, because the
//! decoded texture only ever lived on the row widget that the list view then
//! recycled.

use std::cell::RefCell;

use gtk::gdk;
use matrix_sdk::media::{MediaRequestParameters, UniqueKey};
use quick_cache::unsync::Cache;

use crate::utils::media::FrameDimensions;

/// The maximum number of textures kept in the cache.
///
/// The textures are bounded by the dimensions that were requested for them, so
/// counting them is a good enough proxy for the memory that they use.
const CACHE_CAPACITY: usize = 500;

thread_local! {
    /// The cache of decoded textures of the current thread.
    ///
    /// The images are decoded on the main thread, so in practice this is a
    /// single shared cache. A different thread getting its own empty cache
    /// costs a decode, it does not return a wrong texture.
    static TEXTURES: RefCell<Cache<TextureCacheKey, gdk::Texture>> =
        RefCell::new(Cache::new(CACHE_CAPACITY));
}

/// The key of a texture in the cache.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct TextureCacheKey {
    /// The unique identifier of the source of the texture.
    source: String,
    /// The dimensions that the texture was decoded for.
    dimensions: FrameDimensions,
}

impl TextureCacheKey {
    /// Construct the key for the given media request.
    pub(crate) fn with_request(
        request: &MediaRequestParameters,
        dimensions: FrameDimensions,
    ) -> Self {
        Self {
            source: request.unique_key(),
            dimensions,
        }
    }

    /// Construct the key for the placeholder of the given Blurhash.
    pub(crate) fn with_blurhash(blurhash: &str, dimensions: FrameDimensions) -> Self {
        Self {
            source: format!("blurhash:{blurhash}"),
            dimensions,
        }
    }
}

/// Get the cached texture for the given key, if there is one.
pub(crate) fn get(key: &TextureCacheKey) -> Option<gdk::Texture> {
    TEXTURES.with_borrow_mut(|textures| textures.get(key).cloned())
}

/// Cache the given texture for the given key.
pub(crate) fn insert(key: TextureCacheKey, texture: gdk::Texture) {
    TEXTURES.with_borrow_mut(|textures| textures.insert(key, texture));
}
