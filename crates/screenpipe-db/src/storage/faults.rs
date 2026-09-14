// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

/// Subprocess crash injection is excluded from ordinary application builds.
pub(super) fn checkpoint(_name: &str) {
    #[cfg(feature = "storage-fault-injection")]
    if std::env::var("SCREENPIPE_STORAGE_CRASH_AT").ok().as_deref() == Some(_name) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static HITS: AtomicUsize = AtomicUsize::new(0);
        let target = std::env::var("SCREENPIPE_STORAGE_CRASH_HIT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1);
        if HITS.fetch_add(1, Ordering::SeqCst) + 1 == target {
            std::process::exit(86);
        }
    }
}
