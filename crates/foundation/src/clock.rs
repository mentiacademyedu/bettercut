//! The clock, in a way that works in a browser too.
//!
//! `std::time::Instant::now()` and `SystemTime::now()` panic in WebAssembly,
//! which has no system clock of its own; `web-time` reads the browser's
//! instead, and on every other platform simply is `std::time`. So elapsed
//! time is measured with [`Instant`] from here, and "now" for comparing with
//! a file's times is [`system_now`].

pub use web_time::Instant;

/// Now, as the `std::time::SystemTime` that file times are given in.
pub fn system_now() -> std::time::SystemTime {
    #[cfg(target_arch = "wasm32")]
    {
        let since = web_time::SystemTime::now()
            .duration_since(web_time::UNIX_EPOCH)
            .unwrap_or_default();
        std::time::UNIX_EPOCH + since
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
    }
}
