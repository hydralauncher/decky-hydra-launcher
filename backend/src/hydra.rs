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
}

fn get_leveldb_snapshot() -> Snapshot {
    let original_path = dirs::config_dir()
        .unwrap()
        .join("hydralauncher")
        .join("hydra-db");

    let temp_dir = tempfile::tempdir().unwrap();

    fs_extra::dir::copy(
        &original_path,
        temp_dir.path(),
        &fs_extra::dir::CopyOptions {
            content_only: true,
            ..Default::default()
        },
    )
    .unwrap();

    Snapshot {
        db: DB::open(temp_dir.path(), Options::default()).unwrap(),
        _temp_dir: temp_dir,
    }
}

pub fn get_auth() -> String {
    let mut snapshot = get_leveldb_snapshot();

    let auth = match snapshot.db.get(b"auth") {
        Some(auth_data) => String::from_utf8(auth_data).unwrap().to_string(),
        None => String::from(""),
    };

    snapshot.db.close().unwrap();

    auth
}

pub fn get_game_executable_path(object_id: &str, shop: &str) -> Option<String> {
    let mut snapshot = get_leveldb_snapshot();
    let key = format!("!games!{shop}:{object_id}");
    let value = snapshot.db.get(key.as_bytes())?;
    let _ = snapshot.db.close();

    let game: serde_json::Value = serde_json::from_slice(&value).ok()?;
    game.get("executablePath")?.as_str().map(|s| s.to_string())
}

/// Custom save-path bindings the user configured in the launcher, as
/// (rawPath, localPath) pairs. Read-only: the plugin never writes bindings.
pub fn get_custom_paths(object_id: &str, shop: &str) -> Vec<(String, String)> {
    let mut snapshot = get_leveldb_snapshot();

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

    entries
        .iter()
        .filter_map(|entry| {
            let raw_path = entry.get("rawPath")?.as_str()?.to_string();
            let local_path = entry.get("localPath")?.as_str()?.to_string();
            Some((raw_path, local_path))
        })
        .collect()
}

pub fn get_library() -> String {
    let mut snapshot = get_leveldb_snapshot();

    // The launcher stores the v2 auto-sync toggle in a separate sublevel; the
    // legacy game flag is no longer the source of truth. v2 sync defaults to
    // enabled for Steam games unless the sublevel explicitly says false.
    let mut sync_settings: HashMap<String, bool> = HashMap::new();
    let mut iter = snapshot.db.new_iter().unwrap();
    while let Some((key_bytes, value_bytes)) = iter.next() {
        let key = String::from_utf8(key_bytes).unwrap();
        if let Some(game_key) = key.strip_prefix("!cloud-save-automatic-sync-settings!") {
            let value = String::from_utf8(value_bytes).unwrap();
            let enabled = matches!(value.trim(), "true" | "\"true\"");
            sync_settings.insert(game_key.to_string(), enabled);
        }
    }

    let wine_prefixes_dir = dirs::config_dir()
        .unwrap()
        .join("hydralauncher")
        .join("wine-prefixes");

    let mut iter = snapshot.db.new_iter().unwrap();
    let mut library = Vec::new();

    while let Some((key_bytes, value_bytes)) = iter.next() {
        let key = String::from_utf8(key_bytes).unwrap();
        if key.starts_with("!games") {
            let mut game: Game = serde_json::from_str(&String::from_utf8(value_bytes).unwrap()).unwrap();

            let game_key = format!("{}:{}", game.shop, game.object_id);
            if let Some(enabled) = sync_settings.get(&game_key) {
                game.automatic_cloud_sync = Some(*enabled);
            } else if game.shop == "steam" {
                game.automatic_cloud_sync = Some(true);
            }

            // Newer launchers keep per-game prefixes under wine-prefixes/<id>
            // and no longer write winePrefixPath into the game record.
            if game.wine_prefix_path.is_none() {
                let candidate = wine_prefixes_dir.join(&game.object_id);
                if candidate.is_dir() {
                    game.wine_prefix_path = Some(candidate.to_string_lossy().to_string());
                }
            }

            library.push(game);
        }
    }

    snapshot.db.close().unwrap();

    serde_json::to_string(&library).unwrap()
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
    let manifest: BackupManifest = serde_yaml::from_str(&data).unwrap();

    let user_profile_path = get_windows_like_user_profile_path(wine_prefix_path.unwrap()).unwrap();

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
        .unwrap()
        .join("hydralauncher")
        .join("Backups");

    fs::create_dir_all(&backups_path)?;

    let zip_location = backups_path.join(object_key);
    let backup_path = backups_path.join(format!("{}-{}", shop, object_id));

    if backup_path.exists() {
        fs::remove_dir_all(&backup_path)?;
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