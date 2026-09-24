use std::collections::HashSet;
use std::path::Path;

pub struct PruneReport {
    pub pruned: usize,
    pub skipped: bool,
}

fn state_file_kept(file_name: &str, library: &HashSet<(String, String)>) -> Option<bool> {
    let stem = file_name.strip_suffix(".json")?;
    if stem.is_empty() {
        return None;
    }
    if !stem.contains('@') && stem.starts_with("rules-") {
        return None;
    }
    if let Some((left, _)) = stem.rsplit_once('@') {
        if left.is_empty() {
            return None;
        }
        let kept = library
            .iter()
            .any(|(shop, object_id)| format!("{shop}-{object_id}") == left);
        return Some(kept);
    }
    let kept = library
        .iter()
        .any(|(shop, object_id)| format!("{shop}-{object_id}") == stem);
    Some(kept)
}

fn sidecar_file_kept(file_name: &str, object_ids: &HashSet<String>) -> Option<bool> {
    let stem = file_name.strip_suffix(".json")?;
    let object_id = stem.strip_prefix("rules-")?;
    if object_id.is_empty()
        || !object_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return None;
    }
    Some(object_ids.contains(object_id))
}

fn prune_dir(dir: &Path, should_keep: impl Fn(&str) -> Option<bool>) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut pruned = 0;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let keep = match should_keep(&name) {
            Some(keep) => keep,
            None => continue,
        };
        if keep {
            continue;
        }
        if entry.path().is_dir() {
            continue;
        }
        match std::fs::remove_file(entry.path()) {
            Ok(()) => {
                eprintln!("prune: removed orphan {name}");
                pruned += 1;
            }
            Err(err) => {
                eprintln!("prune: failed to remove {name}: {err:#}");
            }
        }
    }
    pruned
}

pub fn prune_cloud_save_cache() -> PruneReport {
    let Some(library) = crate::hydra::library_game_keys() else {
        return PruneReport {
            pruned: 0,
            skipped: true,
        };
    };
    let object_ids: HashSet<String> = library
        .iter()
        .map(|(_, object_id)| object_id.clone())
        .collect();
    let mut pruned = 0;
    if let Ok(dir) = crate::cloud_save::state_dir() {
        pruned += prune_dir(&dir, |name| state_file_kept(name, &library));
    }
    if let Ok(dir) = crate::rules::plugin_data_dir() {
        pruned += prune_dir(&dir, |name| sidecar_file_kept(name, &object_ids));
    }
    PruneReport {
        pruned,
        skipped: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn library(pairs: &[(&str, &str)]) -> HashSet<(String, String)> {
        pairs
            .iter()
            .map(|(shop, oid)| (shop.to_string(), oid.to_string()))
            .collect()
    }

    fn ids(values: &[&str]) -> HashSet<String> {
        values.iter().map(|v| v.to_string()).collect()
    }

    #[test]
    fn state_keyed_keep_matches_exact_identity() {
        let library = library(&[("steam", "1313140")]);
        assert_eq!(
            state_file_kept("steam-1313140@abcdef0123456789.json", &library),
            Some(true)
        );
        assert_eq!(
            state_file_kept("steam-1313140@none.json", &library),
            Some(true)
        );
        assert_eq!(
            state_file_kept("steam-9999999@abcdef0123456789.json", &library),
            Some(false)
        );
    }

    #[test]
    fn state_keyed_handles_dashes_in_shop() {
        let library = library(&[("my-shop", "1")]);
        assert_eq!(
            state_file_kept("my-shop-1@abcdef0123456789.json", &library),
            Some(true)
        );
        assert_eq!(
            state_file_kept("my-1@abcdef0123456789.json", &library),
            Some(false)
        );
    }

    #[test]
    fn state_legacy_keep_matches_exact_identity() {
        let library = library(&[("steam", "1313140")]);
        assert_eq!(
            state_file_kept("steam-1313140.json", &library),
            Some(true)
        );
        assert_eq!(state_file_kept("steam-9999999.json", &library), Some(false));
    }

    #[test]
    fn state_ignores_foreign_names() {
        let library = library(&[("steam", "1313140")]);
        assert_eq!(state_file_kept("rules-1313140.json", &library), None);
        assert_eq!(state_file_kept("random.txt", &library), None);
        assert_eq!(state_file_kept("steam-1313140", &library), None);
        assert_eq!(state_file_kept(".json", &library), None);
        assert_eq!(state_file_kept("@abcdef0123456789.json", &library), None);
    }

    #[test]
    fn sidecar_keep_matches_object_id_any_shop() {
        let object_ids = ids(&["1313140"]);
        assert_eq!(
            sidecar_file_kept("rules-1313140.json", &object_ids),
            Some(true)
        );
        assert_eq!(
            sidecar_file_kept("rules-9999999.json", &object_ids),
            Some(false)
        );
        assert_eq!(sidecar_file_kept("rules-.json", &object_ids), None);
        assert_eq!(
            sidecar_file_kept("rules-a/b.json", &object_ids),
            None
        );
        assert_eq!(sidecar_file_kept("other.json", &object_ids), None);
    }

    #[test]
    fn prune_dir_removes_only_orphans() {
        let dir = tempfile::tempdir().unwrap();
        for name in [
            "steam-1@aaaaaaaaaaaaaaaa.json",
            "steam-1.json",
            "steam-2@bbbbbbbbbbbbbbbb.json",
            "rules-1.json",
            "notes.txt",
        ] {
            std::fs::write(dir.path().join(name), "{}").unwrap();
        }
        let library = library(&[("steam", "1")]);
        let pruned = prune_dir(dir.path(), |name| state_file_kept(name, &library));
        assert_eq!(pruned, 1);
        assert!(dir.path().join("steam-1@aaaaaaaaaaaaaaaa.json").exists());
        assert!(dir.path().join("steam-1.json").exists());
        assert!(!dir.path().join("steam-2@bbbbbbbbbbbbbbbb.json").exists());
        assert!(dir.path().join("rules-1.json").exists());
        assert!(dir.path().join("notes.txt").exists());
    }

    #[test]
    fn prune_dir_missing_is_noop() {
        let library = library(&[("steam", "1")]);
        let missing = Path::new("/definitely/not/here/decky-prune-test");
        assert_eq!(
            prune_dir(missing, |name| state_file_kept(name, &library)),
            0
        );
    }
}
