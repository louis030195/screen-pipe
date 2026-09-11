// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

/// Subprocess crash injection is excluded from ordinary application builds.
pub(super) fn checkpoint(_name: &str) {
    #[cfg(feature = "storage-fault-injection")]
    if std::env::var("SCREENPIPE_STORAGE_CRASH_AT").ok().as_deref() == Some(_name) {
        std::process::exit(86);
    }
}
