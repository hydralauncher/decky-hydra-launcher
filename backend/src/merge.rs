use std::collections::HashMap;

use crate::cloud_save::{SnapshotFileEntry, StateEntry};

/// 3-way merge of local and remote snapshots against a shared base
/// (the launcher anchor or the plugin's own state entries).

pub struct MergeOutcome {
    pub files: Vec<SnapshotFileEntry>,
    pub conflicts: Vec<ConflictEntry>,
}

#[allow(dead_code)]
pub struct ConflictEntry {
    pub identity: String,
    pub local_hash: Option<String>,
    pub remote_hash: Option<String>,
}

fn key(entry: &StateEntry) -> String {
    format!("{}\u{0}{}\u{0}{}", entry.variant_id, entry.raw_path, entry.relative_path)
}

fn to_state_entry(file: &SnapshotFileEntry) -> StateEntry {
    StateEntry {
        variant_id: file.variant_id.clone(),
        raw_path: file.raw_path.clone(),
        relative_path: file.relative_path.clone(),
        hash: file.hash.clone(),
        size_bytes: file.size_bytes,
    }
}

fn same_content(a: &StateEntry, b: &StateEntry) -> bool {
    a.hash == b.hash && a.size_bytes == b.size_bytes
}

pub fn merge_snapshots(
    local: &[SnapshotFileEntry],
    remote: &[SnapshotFileEntry],
    base: Option<&[StateEntry]>,
) -> MergeOutcome {
    let local_map: HashMap<String, &SnapshotFileEntry> =
        local.iter().map(|f| (key(&to_state_entry(f)), f)).collect();
    let remote_map: HashMap<String, &SnapshotFileEntry> =
        remote.iter().map(|f| (key(&to_state_entry(f)), f)).collect();
    let base_map: HashMap<String, &StateEntry> = base
        .unwrap_or(&[])
        .iter()
        .map(|e| (key(e), e))
        .collect();

    let mut identities: Vec<String> = local_map
        .keys()
        .chain(remote_map.keys())
        .cloned()
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    identities.sort();

    let mut files = Vec::new();
    let mut conflicts = Vec::new();

    for identity in identities {
        let local = local_map.get(&identity).copied();
        let remote = remote_map.get(&identity).copied();
        let base_entry = base_map.get(&identity).copied();

        let local_entry = local.map(to_state_entry);
        let remote_entry = remote.map(to_state_entry);

        match (local, remote, base_entry) {
            // Present on both sides.
            (Some(l), Some(r), _) => {
                let le = local_entry.clone().unwrap();
                let re = remote_entry.clone().unwrap();
                if same_content(&le, &re) {
                    files.push(l.clone());
                    continue;
                }
                match base_entry {
                    // Only remote changed → take remote.
                    Some(b) if same_content(&le, b) => files.push(r.clone()),
                    // Only local changed → take local.
                    Some(b) if same_content(&re, b) => files.push(l.clone()),
                    // Both changed differently (or no base) → conflict.
                    _ => conflicts.push(ConflictEntry {
                        identity,
                        local_hash: Some(le.hash),
                        remote_hash: Some(re.hash),
                    }),
                }
            }
            // Deleted locally, remote intact → propagate deletion only if
            // remote matches base; remote changed since base → conflict.
            (None, Some(_r), Some(b)) => {
                let re = remote_entry.clone().unwrap();
                if same_content(&re, b) {
                    continue; // deletion wins
                }
                conflicts.push(ConflictEntry {
                    identity,
                    local_hash: None,
                    remote_hash: Some(re.hash),
                });
            }
            // New on remote → take remote.
            (None, Some(r), None) => files.push(r.clone()),
            // Deleted remotely → propagate if local matches base.
            (Some(_l), None, Some(b)) => {
                let le = local_entry.clone().unwrap();
                if same_content(&le, b) {
                    continue;
                }
                conflicts.push(ConflictEntry {
                    identity,
                    local_hash: Some(le.hash),
                    remote_hash: None,
                });
            }
            // New locally → take local.
            (Some(l), None, None) => files.push(l.clone()),
            (None, None, _) => {}
        }
    }

    MergeOutcome { files, conflicts }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(raw: &str, rel: &str, hash: &str) -> SnapshotFileEntry {
        SnapshotFileEntry {
            variant_id: "v".into(),
            raw_path: raw.into(),
            relative_path: rel.into(),
            hash: hash.into(),
            size_bytes: 1,
            last_modified_at: "2024-01-01T00:00:00.000Z".into(),
        }
    }

    fn entry(raw: &str, rel: &str, hash: &str) -> StateEntry {
        StateEntry {
            variant_id: "v".into(),
            raw_path: raw.into(),
            relative_path: rel.into(),
            hash: hash.into(),
            size_bytes: 1,
        }
    }

    #[test]
    fn one_side_changes_win() {
        let base = vec![entry("<home>/g", "a.sav", "h1")];
        let local = vec![file("<home>/g", "a.sav", "h1")];
        let remote = vec![file("<home>/g", "a.sav", "h2")];

        let merged = merge_snapshots(&local, &remote, Some(&base));
        assert!(merged.conflicts.is_empty());
        assert_eq!(merged.files[0].hash, "h2");
    }

    #[test]
    fn both_change_conflicts() {
        let base = vec![entry("<home>/g", "a.sav", "h1")];
        let local = vec![file("<home>/g", "a.sav", "h2")];
        let remote = vec![file("<home>/g", "a.sav", "h3")];

        let merged = merge_snapshots(&local, &remote, Some(&base));
        assert_eq!(merged.conflicts.len(), 1);
        assert!(merged.files.is_empty());
    }

    #[test]
    fn deletion_propagates_when_other_side_unchanged() {
        let base = vec![entry("<home>/g", "a.sav", "h1")];
        let local: Vec<SnapshotFileEntry> = vec![];
        let remote = vec![file("<home>/g", "a.sav", "h1")];

        let merged = merge_snapshots(&local, &remote, Some(&base));
        assert!(merged.conflicts.is_empty());
        assert!(merged.files.is_empty());
    }

    #[test]
    fn no_base_marks_divergence_as_conflict() {
        let local = vec![file("<home>/g", "a.sav", "h1")];
        let remote = vec![file("<home>/g", "a.sav", "h2")];

        let merged = merge_snapshots(&local, &remote, None);
        assert_eq!(merged.conflicts.len(), 1);
    }
}
