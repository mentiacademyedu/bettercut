//! The on-disk cache (§19, §67).
//!
//! Proxies, thumbnails, waveforms and analysis results — everything that can be
//! regenerated from the user's media, and nothing that cannot.
//!
//! The single most important property is stated in `store`: **source media is
//! never touched.** Everything here either reads the cache or deletes from
//! inside it, and the one recursive delete refuses any path it cannot prove is
//! within the cache root.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod error;
pub mod layout;
pub mod store;
pub mod thumbnail;

pub use error::CacheError;
pub use layout::CacheLayout;
pub use store::{CACHE_LIMIT_5_GB, CACHE_LIMIT_10_GB, CACHE_LIMIT_20_GB, CacheEntry, CacheStore};
pub use thumbnail::Thumbnail;
