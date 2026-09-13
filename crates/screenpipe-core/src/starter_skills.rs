// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

//! Public, portable starter workflows. Both desktop and CLI use this installer.
//! Existing files are never adopted or overwritten. Only unchanged managed
//! copies can be refreshed or removed. No private device skills are exported.
use std::{
    fs, io,
    path::{Path, PathBuf},
};

pub const STARTER_SKILLS: &[(&str, &str)] = &[
    (
        "screenpipe-recall",
        include_str!("../assets/skills/screenpipe-recall/SKILL.md"),
    ),
    (
        "screenpipe-meeting-prep",
        include_str!("../assets/skills/screenpipe-meeting-prep/SKILL.md"),
    ),
    (
        "screenpipe-meeting-follow-up",
        include_str!("../assets/skills/screenpipe-meeting-follow-up/SKILL.md"),
    ),
    (
        "screenpipe-worklog",
        include_str!("../assets/skills/screenpipe-worklog/SKILL.md"),
    ),
    (
        "screenpipe-research-synthesis",
        include_str!("../assets/skills/screenpipe-research-synthesis/SKILL.md"),
    ),
    (
        "screenpipe-focus-review",
        include_str!("../assets/skills/screenpipe-focus-review/SKILL.md"),
    ),
    (
        "screenpipe-durable-learning",
        include_str!("../assets/skills/screenpipe-durable-learning/SKILL.md"),
    ),
    (
        "screenpipe-shareable-recap",
        include_str!("../assets/skills/screenpipe-shareable-recap/SKILL.md"),
    ),
];
const MARKER: &str = ".screenpipe-starter-v1";

fn regular(path: &Path, directory: bool) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(m)
            if !m.file_type().is_symlink()
                && (if directory { m.is_dir() } else { m.is_file() }) =>
        {
            Ok(true)
        }
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "starter skill path must not be a symlink or special file",
        )),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

fn ensure_root(root: &Path) -> io::Result<()> {
    // Inspect the root and its two immediate ancestors too: an agent's entire config directory can
    // be a link. Refuse it rather than writing through a surprising target.
    // Canonicalize the OS temp prefix at the caller in tests (macOS /tmp).
    for ancestor in root
        .ancestors()
        .filter(|p| !p.as_os_str().is_empty())
        .take(3)
    {
        regular(ancestor, true)?;
    }
    fs::create_dir_all(root)
}

pub fn install_one(root: &Path, name: &str, content: &str) -> io::Result<PathBuf> {
    if !STARTER_SKILLS.iter().any(|(key, _)| *key == name) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "unknown starter skill",
        ));
    }
    ensure_root(root)?;
    let dir = root.join(name);
    regular(&dir, true)?;
    fs::create_dir_all(&dir)?;
    let path = dir.join("SKILL.md");
    let marker = dir.join(MARKER);
    let exists = regular(&path, false)?;
    let owned = regular(&marker, false)?;
    if exists {
        // An unmarked collision belongs to the user, even if bytes happen to
        // match. A manual edit also revokes management.
        if !owned || fs::read(&path)? != fs::read(&marker)? {
            return Ok(path);
        }
        if fs::read_to_string(&path)? == content {
            return Ok(path);
        }
        // Leave the previous version intact on a failed write. The marker is
        // updated last; interruption revokes management instead of clobbering.
        let staged = dir.join(".screenpipe-starter-next");
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staged)?;
        use io::Write;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
        drop(file);
        fs::rename(&staged, &path)?;
        fs::write(&marker, content)?;
    } else {
        use io::Write;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
        fs::write(&marker, content)?;
    }
    Ok(path)
}

pub fn install(root: &Path) -> io::Result<Vec<PathBuf>> {
    STARTER_SKILLS
        .iter()
        .map(|(name, md)| install_one(root, name, md))
        .collect()
}

/// Seed the local store once per name, remembering user deletions across app
/// launches. Explicit external-agent setup continues to use install_one.
pub fn install_store(root: &Path) -> io::Result<Vec<PathBuf>> {
    ensure_root(root)?;
    let index = root.join(".screenpipe-starters-installed-v1");
    let mut seen = if regular(&index, false)? {
        fs::read_to_string(&index)?
    } else {
        String::new()
    };
    let mut installed = Vec::new();
    for (name, md) in STARTER_SKILLS {
        if seen.lines().any(|line| line == *name) && !root.join(name).exists() {
            continue;
        }
        installed.push(install_one(root, name, md)?);
        if !seen.lines().any(|line| line == *name) {
            seen.push_str(name);
            seen.push('\n');
        }
    }
    let staged = root.join(".screenpipe-starters-index-next");
    use io::Write;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staged)?;
    file.write_all(seen.as_bytes())?;
    file.sync_all()?;
    drop(file);
    fs::rename(staged, index)?;
    Ok(installed)
}

pub fn is_current_or_custom(root: &Path, name: &str, content: &str) -> bool {
    let dir = root.join(name);
    let Ok(current) = fs::read_to_string(dir.join("SKILL.md")) else {
        return false;
    };
    if current == content {
        return true;
    }
    match fs::read_to_string(dir.join(MARKER)) {
        Ok(previous) => previous != current,
        Err(error) => error.kind() == io::ErrorKind::NotFound,
    }
}

pub fn remove_one(root: &Path, name: &str) -> io::Result<bool> {
    if !STARTER_SKILLS.iter().any(|(key, _)| *key == name) {
        return Ok(false);
    }
    ensure_root(root)?;
    let dir = root.join(name);
    if !regular(&dir, true)? {
        return Ok(false);
    }
    let path = dir.join("SKILL.md");
    let marker = dir.join(MARKER);
    if !regular(&path, false)? || !regular(&marker, false)? {
        return Ok(false);
    }
    if fs::read(&path)? != fs::read(&marker)? {
        return Ok(false);
    }
    fs::remove_file(path)?;
    fs::remove_file(marker)?;
    // Never recursively remove a user's supplementary files.
    let _ = fs::remove_dir(dir);
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
                "screenpipe-starters-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            fs::create_dir(&root).unwrap();
            Self(root)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn bundle_installs_and_is_idempotent() {
        let t = Temp::new();
        let paths = install(&t.0).unwrap();
        assert_eq!(paths.len(), 8);
        assert_eq!(install(&t.0).unwrap(), paths);
        for ((name, body), path) in STARTER_SKILLS.iter().zip(paths) {
            assert!(body.contains(&format!("name: {name}")));
            assert_eq!(fs::read_to_string(path).unwrap(), *body);
        }
    }
    #[test]
    fn preserve_collisions_manual_edits_and_supplementary_files() {
        let t = Temp::new();
        let (name, body) = STARTER_SKILLS[0];
        let path = install_one(&t.0, name, body).unwrap();
        fs::write(&path, "personal edit").unwrap();
        install_one(&t.0, name, "new version").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "personal edit");
        assert!(!remove_one(&t.0, name).unwrap());
        let (other, body) = STARTER_SKILLS[1];
        let p = install_one(&t.0, other, body).unwrap();
        fs::write(p.parent().unwrap().join("notes.txt"), "keep").unwrap();
        assert!(remove_one(&t.0, other).unwrap());
        assert!(p.parent().unwrap().join("notes.txt").exists());
    }
    #[test]
    fn unmarked_collision_is_never_adopted() {
        let t = Temp::new();
        let (name, body) = STARTER_SKILLS[0];
        fs::create_dir(t.0.join(name)).unwrap();
        fs::write(t.0.join(name).join("SKILL.md"), body).unwrap();
        install_one(&t.0, name, "replacement").unwrap();
        assert!(!remove_one(&t.0, name).unwrap());
        assert_eq!(
            fs::read_to_string(t.0.join(name).join("SKILL.md")).unwrap(),
            body
        );
    }
    #[test]
    fn update_managed_copy_and_remove_idempotently() {
        let t = Temp::new();
        let (name, body) = STARTER_SKILLS[0];
        let p = install_one(&t.0, name, body).unwrap();
        install_one(&t.0, name, "new").unwrap();
        assert_eq!(fs::read_to_string(p).unwrap(), "new");
        assert!(remove_one(&t.0, name).unwrap());
        assert!(!remove_one(&t.0, name).unwrap());
    }
    #[test]
    fn store_respects_deletion_and_refreshes_only_managed_copies() {
        let t = Temp::new();
        install_store(&t.0).unwrap();
        let (name, body) = STARTER_SKILLS[0];
        assert!(is_current_or_custom(&t.0, name, body));
        assert!(!is_current_or_custom(&t.0, name, "new version"));
        fs::remove_dir_all(t.0.join(name)).unwrap();
        install_store(&t.0).unwrap();
        assert!(!t.0.join(name).exists());
    }
    #[test]
    fn reject_unknown_names() {
        let t = Temp::new();
        assert!(install_one(&t.0, "../escape", "x").is_err());
    }
    #[cfg(unix)]
    #[test]
    fn reject_links_at_root_directory_file_and_marker() {
        use std::os::unix::fs::symlink;
        for part in ["root", "directory", "file", "marker"] {
            let t = Temp::new();
            let outside = Temp::new();
            let (name, body) = STARTER_SKILLS[0];
            let root = t.0.join("skills");
            if part == "root" {
                symlink(&outside.0, &root).unwrap();
            } else {
                fs::create_dir(&root).unwrap();
                let dir = root.join(name);
                if part == "directory" {
                    symlink(&outside.0, &dir).unwrap();
                } else {
                    fs::create_dir(&dir).unwrap();
                    fs::write(outside.0.join("target"), "unchanged").unwrap();
                    symlink(
                        outside.0.join("target"),
                        dir.join(if part == "file" { "SKILL.md" } else { MARKER }),
                    )
                    .unwrap();
                }
            }
            assert!(install_one(&root, name, body).is_err());
            assert!(!outside.0.join("SKILL.md").exists());
        }
    }
}
