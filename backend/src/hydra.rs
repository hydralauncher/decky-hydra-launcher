use rusty_leveldb::{DB, LdbIterator, Options};

use serde::{Deserialize, Serialize};

use tempfile::TempDir;

use std::fs;

use std::fs::File;

use std::path::PathBuf;

use tar::Archive;

use std::io::Write;

use reqwest::Client;

use std::collections::HashMap;

use crate::wine::{add_wine_prefix_to_windows_path, get_windows_like_user_profile_path, transform_ludusavi_backup_path_into_windows_path};

struct Snapshot {

    db: DB,

    _temp_dir: TempDir,

}

#[derive(Debug, Deserialize)]

pub struct BackupManifest {

    pub drives: HashMap<String, String>,

    pub backups: Vec<LudusaviBackup>,

}

#[derive(Debug, Deserialize)]

pub struct LudusaviBackup {

    pub files: HashMap<String, FileMetadata>,

}

#[derive(Debug, Deserialize)]

pub struct FileMetadata {

}

#[derive(Serialize, Deserialize)]

#[serde(rename_all = "camelCase")]

struct Game {

    remote_id: Option<String>,

    object_id: String,

    shop: String,

    title: String,

    last_time_played: Option<String>,

    play_time_in_milliseconds: f64,

    is_deleted: bool,

    icon_url: Option<String>,

    wine_prefix_path: Option<String>,

    automatic_cloud_sync: Option<bool>,

    executable_path: Option<String>,

    #[serde(default, deserialize_with = "deserialize_optional_id")]

    steam_shortcut_app_id: Option<u64>,

}

fn deserialize_optional_id<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>

where

    D: serde::Deserializer<'de>,

{

    let value: Option<serde_json::Value> = Option::deserialize(deserializer)?;

    Ok(value.and_then(|v| match v {

        serde_json::Value::Number(n) => n.as_u64(),

        serde_json::Value::String(s) => s.parse().ok(),

        _ => None,

    }))

}

fn get_leveldb_snapshot() -> Option<Snapshot> {

    let original_path = dirs::config_dir()?

        .join("hydralauncher")

        .join("hydra-db");

    let temp_dir = tempfile::tempdir().ok()?;

    fs_extra::dir::copy(

        &original_path,

        temp_dir.path(),

        &fs_extra::dir::CopyOptions {

            content_only: true,

            ..Default::default()

        },

    )

    .ok()?;

    match DB::open(temp_dir.path(), Options::default()) {

        Ok(db) => Some(Snapshot {

            db,

            _temp_dir: temp_dir,

        }),

        Err(err) => {

            eprintln!("Failed to open launcher database snapshot: {err}");

            None

        }

    }

}

pub fn get_auth() -> String {

    let Some(mut snapshot) = get_leveldb_snapshot() else {

        return String::new();

    };

    let auth = match snapshot.db.get(b"auth") {

        Some(auth_data) => String::from_utf8(auth_data).unwrap_or_default(),

        None => String::from(""),

    };

    let _ = snapshot.db.close();

    if auth.is_empty() {

        return "null".to_string();

    }

    auth

}

pub fn get_game_executable_path(object_id: &str, shop: &str) -> Option<String> {

    let Some(mut snapshot) = get_leveldb_snapshot() else {

        return None;

    };

    let key = format!("!games!{shop}:{object_id}");

    let value = snapshot.db.get(key.as_bytes())?;

    let _ = snapshot.db.close();

    let game: serde_json::Value = serde_json::from_slice(&value).ok()?;

    game.get("executablePath")?.as_str().map(|s| s.to_string())

}

#[derive(Debug, Clone, Serialize)]

#[serde(rename_all = "camelCase")]

pub struct ShortcutResolution {

    pub object_id: String,

    pub shop: String,

    pub source: &'static str,

}

fn shortcut_id_of(value: &serde_json::Value) -> Option<u64> {

    match value.get("steamShortcutAppId") {

        Some(serde_json::Value::Number(n)) => n.as_u64(),

        Some(serde_json::Value::String(s)) => s.parse().ok(),

        _ => None,

    }

}

fn normalize_exe(path: &str) -> String {

    let trimmed = path.trim();

    let unquoted = trimmed

        .strip_prefix('"')

        .and_then(|s| s.strip_suffix('"'))

        .unwrap_or(trimmed);

    unquoted.replace('\\', "/").to_lowercase()

}

#[derive(Debug)]

enum VdfNode {

    Str(String),

    Int(i32),

    Dict(Vec<(String, VdfNode)>),

}

fn read_cstring(data: &[u8], pos: &mut usize) -> Option<String> {

    let start = *pos;

    while *pos < data.len() && data[*pos] != 0 {

        *pos += 1;

    }

    if *pos >= data.len() {

        return None;

    }

    let s = String::from_utf8_lossy(&data[start..*pos]).to_string();

    *pos += 1;

    Some(s)

}

fn parse_vdf_dict(data: &[u8], pos: &mut usize) -> Option<Vec<(String, VdfNode)>> {

    let mut out = Vec::new();

    loop {

        if *pos >= data.len() {

            return None;

        }

        let kind = data[*pos];

        *pos += 1;

        if kind == 0x08 {

            return Some(out);

        }

        let key = read_cstring(data, pos)?;

        let value = match kind {

            0x00 => VdfNode::Dict(parse_vdf_dict(data, pos)?),

            0x01 => VdfNode::Str(read_cstring(data, pos)?),

            0x02 => {

                if *pos + 4 > data.len() {

                    return None;

                }

                let v = i32::from_le_bytes(data[*pos..*pos + 4].try_into().ok()?);

                *pos += 4;

                VdfNode::Int(v)

            }

            0x03 | 0x04 | 0x06 => {

                if *pos + 4 > data.len() {

                    return None;

                }

                *pos += 4;

                continue;

            }

            0x07 => {

                if *pos + 8 > data.len() {

                    return None;

                }

                *pos += 8;

                continue;

            }

            0x05 => {

                while *pos + 1 < data.len() && !(data[*pos] == 0 && data[*pos + 1] == 0) {

                    *pos += 2;

                }

                *pos = (*pos + 2).min(data.len());

                continue;

            }

            _ => return None,

        };

        out.push((key, value));

    }

}

fn vdf_get<'a>(dict: &'a [(String, VdfNode)], key: &str) -> Option<&'a VdfNode> {

    dict.iter().find(|(k, _)| k == key).map(|(_, v)| v)

}

struct ShortcutEntry {

    app_id: i32,

    exe: String,

}

fn parse_shortcuts_vdf(data: &[u8]) -> Vec<ShortcutEntry> {

    let mut pos = 0;

    let mut out = Vec::new();

    let Some(root) = parse_vdf_dict(data, &mut pos) else {

        return out;

    };

    let Some(VdfNode::Dict(shortcuts)) = vdf_get(&root, "shortcuts") else {

        return out;

    };

    for (_, node) in shortcuts {

        let VdfNode::Dict(fields) = node else {

            continue;

        };

        let (Some(VdfNode::Int(app_id)), Some(VdfNode::Str(exe))) =

            (vdf_get(fields, "appid"), vdf_get(fields, "Exe"))

        else {

            continue;

        };

        out.push(ShortcutEntry {

            app_id: *app_id,

            exe: exe.clone(),

        });

    }

    out

}

fn shortcuts_vdf_paths() -> Vec<PathBuf> {

    let mut paths = Vec::new();

    for root in steam_roots() {

        let userdata = root.join("userdata");

        if let Ok(users) = std::fs::read_dir(userdata) {

            for user in users.flatten() {

                let vdf = user.path().join("config/shortcuts.vdf");

                if vdf.is_file() {

                    paths.push(vdf);

                }

            }

        }

    }

    paths

}

fn steam_roots_in(home: &std::path::Path) -> Vec<PathBuf> {

    let mut roots = Vec::new();

    for base in [".local/share/Steam", ".steam/steam", ".steam/root"] {

        let root = home.join(base);

        if !root.is_dir() {

            continue;

        }

        let canonical = std::fs::canonicalize(&root).unwrap_or(root);

        if !roots.contains(&canonical) {

            roots.push(canonical);

        }

    }

    roots

}

fn steam_roots() -> Vec<PathBuf> {

    dirs::home_dir().map(|home| steam_roots_in(&home)).unwrap_or_default()

}

fn compat_prefix_in(roots: &[PathBuf], app_id: u32) -> Option<String> {

    for root in roots {

        let candidate = root
            .join("steamapps")
            .join("compatdata")
            .join(app_id.to_string())
            .join("pfx");

        if candidate.is_dir() {

            return Some(candidate.to_string_lossy().to_string());

        }

    }

    None

}

fn shortcut_live_in(paths: &[PathBuf], app_id: u32) -> bool {

    let wanted = app_id as i32;

    paths.iter().any(|path| {

        std::fs::read(path)

            .map(|data| {

                parse_shortcuts_vdf(&data)
                    .iter()
                    .any(|entry| entry.app_id == wanted)

            })

            .unwrap_or(false)

    })

}

fn select_prefix(
    windows_exe: bool,
    compat_prefix: Option<String>,
    shortcut_live: bool,
    fallback_prefix: Option<String>,
) -> Option<String> {

    if !windows_exe {

        return fallback_prefix;

    }

    if shortcut_live {

        if compat_prefix.is_some() {

            return compat_prefix;

        }

        return None;

    }

    fallback_prefix

}

pub fn operative_prefix(
    object_id: &str,
    shop: &str,
    fallback_prefix: Option<&str>,
) -> Option<String> {

    let fallback = fallback_prefix.map(|p| p.to_string());

    let executable = get_game_executable_path(object_id, shop);

    let windows_exe = executable
        .as_deref()
        .is_some_and(|p| p.to_ascii_lowercase().ends_with(".exe"));

    if !windows_exe {

        eprintln!(
            "prefix: game {shop}:{object_id} native, resolved={}",
            fallback.as_deref().unwrap_or("none")
        );

        return fallback;

    }

    let shortcut_id = game_identities()
        .into_iter()
        .find(|game| game.object_id == object_id && game.shop == shop)
        .and_then(|game| game.shortcut_id)
        .and_then(|id| u32::try_from(id).ok());

    let live = shortcut_id.is_some_and(|id| shortcut_live_in(&shortcuts_vdf_paths(), id));

    let compat = shortcut_id.and_then(|id| compat_prefix_in(&steam_roots(), id));

    let resolved = select_prefix(windows_exe, compat, live, fallback);

    eprintln!(
        "prefix: game {shop}:{object_id} live={live} resolved={}",
        resolved.as_deref().unwrap_or("none")
    );

    resolved

}

struct GameIdentity {

    object_id: String,

    shop: String,

    shortcut_id: Option<u64>,

    executable: Option<String>,

    wine_prefix: Option<String>,

}

fn game_identities() -> Vec<GameIdentity> {

    let mut out = Vec::new();

    let Some(mut snapshot) = get_leveldb_snapshot() else {

        return out;

    };

    if let Ok(mut iter) = snapshot.db.new_iter() {

        while let Some((key_bytes, value_bytes)) = iter.next() {

            let Ok(key) = String::from_utf8(key_bytes) else {

                continue;

            };

            if !key.starts_with("!games") {

                continue;

            }

            let Ok(value) = serde_json::from_slice::<serde_json::Value>(&value_bytes) else {

                continue;

            };

            let Some(object_id) = value.get("objectId").and_then(|v| v.as_str()) else {

                continue;

            };

            out.push(GameIdentity {

                object_id: object_id.to_string(),

                shop: value

                    .get("shop")

                    .and_then(|v| v.as_str())

                    .unwrap_or("steam")

                    .to_string(),

                shortcut_id: shortcut_id_of(&value),

                executable: value

                    .get("executablePath")

                    .and_then(|v| v.as_str())

                    .map(|s| s.to_string()),

                wine_prefix: value

                    .get("winePrefixPath")

                    .and_then(|v| v.as_str())

                    .map(|s| s.to_string()),

            });

        }

    }

    let _ = snapshot.db.close();

    out

}

pub fn resolve_shortcut_app_id(app_id: u32) -> Option<ShortcutResolution> {

    let games = game_identities();

    if let Some(game) = games

        .iter()

        .find(|g| g.shortcut_id == Some(app_id as u64))

    {

        return Some(ShortcutResolution {

            object_id: game.object_id.clone(),

            shop: game.shop.clone(),

            source: "leveldb",

        });

    }

    let wanted = app_id as i32;

    let mut exes: Vec<String> = Vec::new();

    for path in shortcuts_vdf_paths() {

        let Ok(data) = std::fs::read(&path) else {

            continue;

        };

        for entry in parse_shortcuts_vdf(&data) {

            if entry.app_id == wanted {

                exes.push(entry.exe);

            }

        }

    }

    if !exes.is_empty() {

        let normalized: Vec<String> = exes.iter().map(|e| normalize_exe(e)).collect();

        let mut hits: Vec<&GameIdentity> = games

            .iter()

            .filter(|g| {

                g.executable

                    .as_deref()

                    .is_some_and(|e| normalized.contains(&normalize_exe(e)))

            })

            .collect();

        hits.sort_by(|a, b| a.object_id.cmp(&b.object_id));

        hits.dedup_by(|a, b| a.object_id == b.object_id);

        match hits.as_slice() {

            [game] => {

                return Some(ShortcutResolution {

                    object_id: game.object_id.clone(),

                    shop: game.shop.clone(),

                    source: "shortcuts-vdf",

                })

            }

            [_, _, ..] => {

                eprintln!("resolve-shortcut: ambiguous exe for appid {app_id}");

                return None;

            }

            [] => {}

        }

    }

    let needle = format!("/compatdata/{app_id}/");

    let mut hits: Vec<&GameIdentity> = games

        .iter()

        .filter(|g| {

            g.wine_prefix

                .as_deref()

                .is_some_and(|p| p.replace('\\', "/").contains(&needle))

        })

        .collect();

    hits.sort_by(|a, b| a.object_id.cmp(&b.object_id));

    hits.dedup_by(|a, b| a.object_id == b.object_id);

    match hits.as_slice() {

        [game] => Some(ShortcutResolution {

            object_id: game.object_id.clone(),

            shop: game.shop.clone(),

            source: "prefix-path",

        }),

        [_, _, ..] => {

            eprintln!("resolve-shortcut: ambiguous prefix for appid {app_id}");

            None

        }

        [] => None,

    }

}

pub fn get_custom_paths(object_id: &str, shop: &str) -> Vec<(String, String, Option<String>)> {

    let Some(mut snapshot) = get_leveldb_snapshot() else {

        return Vec::new();

    };

    let user_id = snapshot

        .db

        .get(b"user")

        .and_then(|value| {

            let parsed: serde_json::Value = serde_json::from_slice(&value).ok()?;

            parsed.get("id")?.as_str().map(|s| s.to_string())

        });

    let Some(user_id) = user_id else {

        let _ = snapshot.db.close();

        return Vec::new();

    };

    let key = format!(

        "!cloud-save-custom-paths!{}",

        serde_json::json!([user_id, shop, object_id])

    );

    let value = snapshot.db.get(key.as_bytes());

    let _ = snapshot.db.close();

    let Some(value) = value else { return Vec::new() };

    let Ok(entries) = serde_json::from_slice::<Vec<serde_json::Value>>(&value) else {

        return Vec::new();

    };

    let mut bindings: Vec<(String, String, Option<String>)> = entries

        .iter()

        .filter(|entry| entry.get("tracking").and_then(|t| t.as_str()) != Some("ignored"))

        .filter_map(|entry| {

            let raw_path = entry.get("rawPath")?.as_str()?.to_string();

            let local_path = entry.get("localPath").and_then(|p| p.as_str())?.to_string();

            if local_path.contains("..") {

                return None;

            }

            let store_user_id = entry

                .get("storeUserId")

                .and_then(|s| s.as_str())

                .map(|s| s.to_string());

            Some((raw_path, local_path, store_user_id))

        })

        .collect();

    bindings.sort_by(|a, b| b.0.len().cmp(&a.0.len()));

    bindings

}

pub struct SyncAnchor {

    pub base_version: u64,

    pub entries: Vec<crate::cloud_save::StateEntry>,

    pub unresolved_entry_ids: Vec<String>,

}

pub struct AnchorRecord {

    pub anchor: SyncAnchor,

    pub updated_at: String,

}

fn valid_environment_marker(value: &str) -> bool {

    let marker = value.trim().to_lowercase();

    let parts: Vec<&str> = marker.split('-').collect();

    if parts.len() != 5 {
        return false;
    }

    let [a, b, c, d, e] = [parts[0], parts[1], parts[2], parts[3], parts[4]];

    if a.len() != 8 || b.len() != 4 || c.len() != 4 || d.len() != 4 || e.len() != 12 {
        return false;
    }

    if !marker.chars().all(|ch| ch.is_ascii_hexdigit() || ch == '-') {
        return false;
    }

    if !('1'..='8').contains(&c.chars().next().unwrap_or('0')) {
        return false;
    }

    if !"89ab".contains(d.chars().next().unwrap_or('0')) {
        return false;
    }

    true

}

pub fn prefix_environment_id(prefix: Option<&str>) -> Option<String> {

    let prefix = prefix.map(|p| p.trim()).filter(|p| !p.is_empty())?;

    let marker = std::fs::read_to_string(
        std::path::Path::new(prefix).join(".hydra-cloud-save-environment-id"),
    )
    .ok()?;

    let marker = marker.trim().to_lowercase();

    valid_environment_marker(&marker).then_some(marker)

}

fn anchor_key_accepted(
    parts: &[serde_json::Value],
    environment_id: Option<&str>,
) -> bool {

    if parts.len() == 3 {
        return true;
    }

    if parts.len() == 5 && parts[3].as_str() == Some("environment") {

        if let Some(key_env) = parts[4].as_str() {

            return environment_id == Some(key_env);

        }

        return false;

    }

    false

}

fn anchor_user_matches(parts: &[serde_json::Value], user_id: &str) -> bool {

    parts.first().and_then(|v| v.as_str()) == Some(user_id)

}

fn current_user_id(snapshot: &mut Snapshot) -> Option<String> {

    let raw = snapshot.db.get(b"user")?;

    serde_json::from_slice::<serde_json::Value>(&raw)

        .ok()?

        .get("id")?

        .as_str()

        .map(|s| s.to_string())

}

pub fn list_sync_anchors(
    object_id: &str,
    shop: &str,
    environment_id: Option<&str>,
) -> Vec<AnchorRecord> {

    let Some(mut snapshot) = get_leveldb_snapshot() else {

        return Vec::new();

    };

    let Some(user_id) = current_user_id(&mut snapshot) else {

        return Vec::new();

    };

    let Ok(mut iter) = snapshot.db.new_iter() else {

        return Vec::new();

    };

    let mut out: Vec<AnchorRecord> = Vec::new();

    while let Some((key_bytes, value_bytes)) = iter.next() {

        let Ok(key) = String::from_utf8(key_bytes) else { continue };

        let Some(raw) = key.strip_prefix("!cloud-save-sync-anchors!") else {

            continue;

        };

        let Ok(parts) = serde_json::from_str::<Vec<serde_json::Value>>(raw) else {

            continue;

        };

        if !anchor_user_matches(&parts, &user_id) {

            continue;

        }

        if parts[1].as_str() != Some(shop) || parts[2].as_str() != Some(object_id) {

            continue;

        }

        if !anchor_key_accepted(&parts, environment_id) {

            continue;

        }

        let Ok(value) = serde_json::from_slice::<serde_json::Value>(&value_bytes) else {

            continue;

        };

        let (Some(version), Some(updated_at)) = (

            value.get("baseVersion").and_then(|v| v.as_u64()),

            value.get("updatedAt").and_then(|v| v.as_str()),

        ) else {

            continue;

        };

        if version < 1 || updated_at.is_empty() {

            continue;

        }

        if parts.len() == 5 {

            let key_env = parts[4].as_str().unwrap_or_default();

            if value.get("environmentId").and_then(|v| v.as_str()) != Some(key_env) {

                continue;

            }

        }

        let entries = value

            .get("entries")

            .and_then(|e| e.as_array())

            .map(|entries| {

                entries

                    .iter()

                    .filter_map(|e| {

                        Some(crate::cloud_save::StateEntry {

                            variant_id: e.get("variantId")?.as_str()?.to_string(),

                            raw_path: e.get("rawPath")?.as_str()?.to_string(),

                            relative_path: e.get("relativePath")?.as_str()?.to_string(),

                            hash: e.get("hash")?.as_str()?.to_string(),

                            size_bytes: e.get("sizeBytes")?.as_u64()?,

                        })

                    })

                    .collect()

            })

            .unwrap_or_default();

        let unresolved_entry_ids = value

            .get("unresolvedRemoteEntryIds")

            .and_then(|e| e.as_array())

            .map(|ids| {

                ids.iter()

                    .filter_map(|id| {

                        let parts: Vec<String> = serde_json::from_value(id.clone()).ok()?;

                        if parts.len() == 3 {

                            Some(parts.join("\u{0}"))

                        } else {

                            None

                        }

                    })

                    .collect()

            })

            .unwrap_or_default();

        let anchor = SyncAnchor {

            base_version: version,

            entries,

            unresolved_entry_ids,

        };

        out.push(AnchorRecord {

            anchor,

            updated_at: updated_at.to_string(),

        });

    }

    let _ = snapshot.db.close();

    out

}

pub fn get_sync_anchor(
    object_id: &str,
    shop: &str,
    environment_id: Option<&str>,
) -> Option<SyncAnchor> {

    list_sync_anchors(object_id, shop, environment_id)

        .into_iter()

        .max_by(|a, b| a.updated_at.cmp(&b.updated_at))

        .map(|record| record.anchor)

}

pub fn get_library() -> String {

    let Some(mut snapshot) = get_leveldb_snapshot() else {

        return "[]".to_string();

    };

    let mut sync_settings: HashMap<String, bool> = HashMap::new();

    let Ok(mut iter) = snapshot.db.new_iter() else {

        return "[]".to_string();

    };

    while let Some((key_bytes, value_bytes)) = iter.next() {

        let Ok(key) = String::from_utf8(key_bytes) else { continue };

        if let Some(game_key) = key.strip_prefix("!cloud-save-automatic-sync-settings!") {

            let Ok(value) = String::from_utf8(value_bytes) else { continue };

            let enabled = matches!(value.trim(), "true" | "\"true\"");

            sync_settings.insert(game_key.to_string(), enabled);

        }

    }

    let Some(config_dir) = dirs::config_dir() else {

        return "[]".to_string();

    };

    let wine_prefixes_dir = config_dir.join("hydralauncher").join("wine-prefixes");

    let Ok(mut iter) = snapshot.db.new_iter() else {

        return "[]".to_string();

    };

    let mut library = Vec::new();

    while let Some((key_bytes, value_bytes)) = iter.next() {

        let Ok(key) = String::from_utf8(key_bytes) else { continue };

        if key.starts_with("!games") {

            let Ok(value_str) = String::from_utf8(value_bytes) else { continue };

            let Ok(mut game) = serde_json::from_str::<Game>(&value_str) else { continue };

            let game_key = format!("{}:{}", game.shop, game.object_id);

            if let Some(enabled) = sync_settings.get(&game_key) {

                game.automatic_cloud_sync = Some(*enabled);

            } else if game.shop == "steam" {

                game.automatic_cloud_sync = Some(true);

            }

            if game.wine_prefix_path.is_none() {

                let candidate = wine_prefixes_dir.join(&game.object_id);

                if candidate.is_dir() {

                    game.wine_prefix_path = Some(candidate.to_string_lossy().to_string());

                }

            }

            library.push(game);

        }

    }

    let _ = snapshot.db.close();

    serde_json::to_string(&library).unwrap_or_else(|_| "[]".to_string())

}

fn restore_ludusavi_backup(

    backup_path: PathBuf,

    title: &str,

    home_dir: &str,

    wine_prefix_path: Option<&str>,

    artifact_wine_prefix_path: Option<String>,

) -> std::io::Result<()> {

    let game_backup_path = backup_path.join(title);

    let mapping_yaml_path = game_backup_path.join("mapping.yaml");

    let data = fs::read_to_string(&mapping_yaml_path)?;

    let manifest: BackupManifest = serde_yaml::from_str(&data)

        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

    let user_profile_path = get_windows_like_user_profile_path(wine_prefix_path.unwrap_or(""))

        .map_err(|e| std::io::Error::new(std::io::ErrorKind::NotFound, e.to_string()))?;

    for backup in manifest.backups {

        for key in backup.files.keys() {

            let mut source_path_with_drives = key.clone();

            for (drive_key, drive_value) in &manifest.drives {

                source_path_with_drives = source_path_with_drives.replacen(drive_value, drive_key, 1);

            }

            let source_path = game_backup_path.join(&source_path_with_drives);

            let public_path = "C:/users/Public";

            let destination_path = transform_ludusavi_backup_path_into_windows_path(key, artifact_wine_prefix_path.clone())

                .replacen(

                    home_dir,

                    &add_wine_prefix_to_windows_path(&user_profile_path, wine_prefix_path),

                    1,

                )

                .replacen(

                    &public_path,

                    &add_wine_prefix_to_windows_path(&public_path, wine_prefix_path),

                    1,

                );

            let destination_path = PathBuf::from(destination_path);

            println!("Moving {} to {}", source_path.display(), destination_path.display());

            if let Some(parent) = destination_path.parent() {

                fs::create_dir_all(parent)?;

            }

            if destination_path.exists() {

                fs::remove_file(&destination_path)?;

            }

            fs::rename(source_path, destination_path)?;

        }

    }

    Ok(())

}

pub async fn download_game_artifact(

    object_id: &str,

    shop: &str,

    download_url: &str,

    object_key: &str,

    home_dir: &str,

    wine_prefix_path: Option<&str>,

    artifact_wine_prefix_path: Option<String>,

) -> Result<(), Box<dyn std::error::Error>> {

    let backups_path = dirs::config_dir()

        .ok_or("No config dir")?

        .join("hydralauncher")

        .join("Backups");

    fs::create_dir_all(&backups_path)?;

    let zip_location = backups_path.join(object_key);

    let backup_path = backups_path.join(format!("{}-{}", shop, object_id));

    if backup_path.exists() {

        fs::remove_dir_all(&backup_path)?;

    }

    if !download_url.starts_with("https://") {

        return Err("Refusing non-HTTPS download URL".into());

    }

    let client = Client::new();

    let mut response = client.get(download_url).send().await?;

    let mut file = File::create(&zip_location)?;

    while let Some(chunk) = response.chunk().await? {

        file.write_all(&chunk)?;

    }

    fs::create_dir_all(&backup_path)?;

    let archive_file = File::open(&zip_location)?;

    let mut archive = Archive::new(archive_file);

    archive.unpack(&backup_path)?;

    restore_ludusavi_backup(

        backup_path,

        object_id,

        home_dir,

        wine_prefix_path,

        artifact_wine_prefix_path,

    )?;

    Ok(())

}

#[cfg(test)]

mod shortcut_tests {

    use super::*;

    fn fixture_vdf() -> Vec<u8> {

        let mut b = Vec::new();

        b.push(0x00);

        b.extend_from_slice(b"shortcuts\0");

        b.push(0x00);

        b.extend_from_slice(b"0\0");

        b.push(0x02);

        b.extend_from_slice(b"appid\0");

        b.extend_from_slice(&12345i32.to_le_bytes());

        b.push(0x01);

        b.extend_from_slice(b"Exe\0");

        b.extend_from_slice(b"\"C:\\Games\\Foo\\game.exe\"\0");

        b.push(0x01);

        b.extend_from_slice(b"AppName\0");

        b.extend_from_slice(b"Foo\0");

        b.push(0x08);

        b.push(0x00);

        b.extend_from_slice(b"7\0");

        b.push(0x02);

        b.extend_from_slice(b"appid\0");

        b.extend_from_slice(&(2972030656u32 as i32).to_le_bytes());

        b.push(0x01);

        b.extend_from_slice(b"Exe\0");

        b.extend_from_slice(b"/mnt/storage/Games/SB.exe\0");

        b.push(0x08);

        b.push(0x08);

        b.push(0x08);

        b

    }

    #[test]

    fn parses_shortcut_entries() {

        let entries = parse_shortcuts_vdf(&fixture_vdf());

        assert_eq!(entries.len(), 2);

        assert_eq!(entries[0].app_id, 12345);

        assert_eq!(entries[0].exe, "\"C:\\Games\\Foo\\game.exe\"");

        assert_eq!(entries[1].app_id, 2972030656u32 as i32);

    }

    #[test]

    fn wrapped_appid_round_trips() {

        let big: u32 = 2972030656;

        assert_eq!((big as i32) as u32, big);

    }

    #[test]

    fn exe_normalization_agrees_across_formats() {

        assert_eq!(

            normalize_exe("\"C:\\Games\\Foo\\game.exe\""),

            normalize_exe("c:/games/foo/game.exe")

        );

        assert_eq!(

            normalize_exe("  /mnt/storage/Games/SB.exe  "),

            "/mnt/storage/games/sb.exe"

        );

    }

    #[test]

    fn prefix_needle_is_segment_safe() {

        let needle = format!("/compatdata/{}/", 10u32);

        assert!("/x/compatdata/10/pfx".contains(&needle));

        assert!(!"/x/compatdata/110/pfx".contains(&needle));

        assert!(!"/x/compatdata/101/pfx".contains(&needle));

    }

    #[test]

    fn garbage_vdf_fails_closed() {

        assert!(parse_shortcuts_vdf(b"\x00shortcuts").is_empty());

        assert!(parse_shortcuts_vdf(b"not vdf at all..........").is_empty());

        assert!(parse_shortcuts_vdf(&[]).is_empty());

    }

    #[test]

    fn select_prefix_matrix() {

        let compat = Some("/steamapps/compatdata/10/pfx".to_string());

        let fallback = Some("/wine-prefixes/game".to_string());

        assert_eq!(
            select_prefix(false, compat.clone(), true, fallback.clone()),
            fallback
        );

        assert_eq!(
            select_prefix(true, compat.clone(), true, fallback.clone()),
            compat
        );

        assert_eq!(select_prefix(true, None, true, fallback.clone()), None);

        assert_eq!(
            select_prefix(true, compat.clone(), false, fallback.clone()),
            fallback
        );

        assert_eq!(select_prefix(true, None, false, None), None);

    }

    #[test]

    fn steam_roots_dedupe_symlinks_and_missing() {

        let home = tempfile::tempdir().unwrap();

        let real = home.path().join(".local/share/Steam");

        std::fs::create_dir_all(&real).unwrap();

        let link_parent = home.path().join(".steam");

        std::fs::create_dir(&link_parent).unwrap();

        std::os::unix::fs::symlink(real.clone(), link_parent.join("steam")).unwrap();

        let roots = steam_roots_in(home.path());

        assert_eq!(roots.len(), 1);

        assert_eq!(roots[0], std::fs::canonicalize(&real).unwrap());

    }

    #[test]

    fn shortcut_live_matches_wrapped_appid_across_files() {

        let dir = tempfile::tempdir().unwrap();

        let with_match = dir.path().join("a.vdf");

        std::fs::write(&with_match, fixture_vdf()).unwrap();

        let without_match = dir.path().join("b.vdf");

        std::fs::write(&without_match, b"\x00shortcuts\x00\x08\x08").unwrap();

        let paths = vec![without_match, with_match];

        assert!(shortcut_live_in(&paths, 12345));

        assert!(shortcut_live_in(&paths, 2972030656));

        assert!(!shortcut_live_in(&paths, 99999));

        assert!(!shortcut_live_in(&[dir.path().join("missing.vdf")], 12345));

    }

    #[test]

    fn compat_prefix_first_existing_dir_wins() {

        let home = tempfile::tempdir().unwrap();

        let good = home.path().join("steamapps/compatdata/10/pfx");

        std::fs::create_dir_all(&good).unwrap();

        let roots = vec![home.path().join("missing"), home.path().to_path_buf()];

        assert_eq!(
            compat_prefix_in(&roots, 10),
            Some(good.to_string_lossy().to_string())
        );

        assert_eq!(compat_prefix_in(&roots, 11), None);

    }

    #[test]

    fn anchor_user_matches_first_key_part_only() {

        let owned: Vec<serde_json::Value> =
            serde_json::from_str(r#"["n5zKXo0j","steam","1313140"]"#).unwrap();

        assert!(anchor_user_matches(&owned, "n5zKXo0j"));

        assert!(!anchor_user_matches(&owned, "75BWoNvj"));

        assert!(!anchor_user_matches(&owned, ""));

    }

    #[test]

    fn anchor_key_accepts_legacy_and_own_environment_only() {

        let legacy: Vec<serde_json::Value> =
            serde_json::from_str(r#"["user","steam","1313140"]"#).unwrap();

        assert!(anchor_key_accepted(&legacy, None));

        assert!(anchor_key_accepted(&legacy, Some("env-a")));

        let own: Vec<serde_json::Value> = serde_json::from_str(
            r#"["user","steam","1313140","environment","env-a"]"#,
        )
        .unwrap();

        assert!(anchor_key_accepted(&own, Some("env-a")));

        assert!(!anchor_key_accepted(&own, None));

        assert!(!anchor_key_accepted(&own, Some("env-b")));

        let short: Vec<serde_json::Value> =
            serde_json::from_str(r#"["user","steam"]"#).unwrap();

        assert!(!anchor_key_accepted(&short, None));

        let mistyped: Vec<serde_json::Value> = serde_json::from_str(
            r#"["user","steam","1313140","environments","env-a"]"#,
        )
        .unwrap();

        assert!(!anchor_key_accepted(&mistyped, Some("env-a")));

    }

    #[test]

    fn prefix_environment_id_reads_valid_marker_only() {

        let dir = tempfile::tempdir().unwrap();

        let prefix = dir.path().to_str().unwrap().to_string();

        assert_eq!(prefix_environment_id(Some(&prefix)), None);

        std::fs::write(
            dir.path().join(".hydra-cloud-save-environment-id"),
            "not-a-uuid\n",
        )
        .unwrap();

        assert_eq!(prefix_environment_id(Some(&prefix)), None);

        std::fs::write(
            dir.path().join(".hydra-cloud-save-environment-id"),
            "123e4567-e89b-12d3-a456-426614174000\n",
        )
        .unwrap();

        assert_eq!(
            prefix_environment_id(Some(&prefix)),
            Some("123e4567-e89b-12d3-a456-426614174000".to_string())
        );

        assert_eq!(prefix_environment_id(None), None);

    }

}