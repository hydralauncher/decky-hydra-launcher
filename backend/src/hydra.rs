use rusty_leveldb::{DB, LdbIterator, Options};
use serde::{Deserialize, Serialize};
use tempfile::TempDir;
use anyhow::{Context, Result};
use std::fs;
use std::fs::File;
use std::path::PathBuf;
use uuid::Uuid;
use tar::{Builder, Archive};
use tokio::fs as tokio_fs;
use std::io::Write;
use reqwest::Client;
use serde_json::json;
use std::collections::HashMap;

use crate::ludusavi::backup_game;
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UploadResponse {
    upload_url: String,
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

pub fn get_library() -> String {
    let mut snapshot = get_leveldb_snapshot();

    let mut iter = snapshot.db.new_iter().unwrap();
    let mut library = Vec::new();

    while let Some((key_bytes, value_bytes)) = iter.next() {
        let key = String::from_utf8(key_bytes).unwrap();
        if key.starts_with("!games") {
            let game: Game = serde_json::from_str(&String::from_utf8(value_bytes).unwrap()).unwrap();
            library.push(game);
        }
    }

    snapshot.db.close().unwrap();

    serde_json::to_string(&library).unwrap()
}

async fn bundle_backup(
    shop: &str,
    object_id: &str,
    wine_prefix: Option<&str>,
) -> Result<PathBuf> {
    let backups_path = dirs::config_dir()
        .unwrap()
        .join("hydralauncher")
        .join("Backups");

    let backup_path = backups_path.join(format!("{shop}-{object_id}"));

    // Remove existing backup
    if backup_path.exists() {
        tokio_fs::remove_dir_all(&backup_path)
            .await
            .context("Failed to remove backup path")?;
    }

    let _ = backup_game(
        object_id,
        Some(backup_path.to_str().unwrap()),
        wine_prefix,
        false,
    )
    .await;

    let tar_location = backups_path.join(format!("{}.tar", Uuid::new_v4()));
    let tar_file = File::create(&tar_location).context("Failed to create tar file")?;
    let mut tar_builder = Builder::new(tar_file);

    tar_builder
        .append_dir_all(".", &backup_path)
        .context("Failed to write tar contents")?;

    tar_builder.finish().context("Failed to finish tar archive")?;

    Ok(tar_location)
}

pub async fn upload_save_game(
    object_id: &str,
    shop: &str,
    wine_prefix_path: Option<&str>,
    access_token: &str,
    label: &str,
) -> Result<()> {
    let bundle_location = bundle_backup(shop, object_id, wine_prefix_path).await?;

    let stat = tokio_fs::metadata(&bundle_location).await?;
    let size = stat.len();

    let wine_prefix_real = match wine_prefix_path.clone() {
        Some(path) => Some(fs::canonicalize(path)?),
        None => None,
    };

    let home_dir = get_windows_like_user_profile_path(wine_prefix_path.unwrap_or("")).unwrap();

    let client = Client::new();
    let response = client
        .post("https://hydra-api-us-east-1.losbroxas.org/profile/games/artifacts")
        .bearer_auth(&access_token)
        .json(&json!({
            "artifactLengthInBytes": size,
            "shop": shop,
            "objectId": object_id,
            "hostname": hostname::get()?.to_string_lossy(),
            "winePrefixPath": wine_prefix_real,
            "homeDir": home_dir,
            "downloadOptionTitle": serde_json::Value::Null,
            "platform": std::env::consts::OS,
            "label": label,
        }))
        .send()
        .await?
        .error_for_status()?
        .json::<UploadResponse>()
        .await?;

    let file_bytes = tokio_fs::read(&bundle_location).await?;

    client
        .put(&response.upload_url)
        .header("Content-Type", "application/tar")
        .body(file_bytes)
        .send()
        .await?
        .error_for_status()?;

    if let Err(err) = tokio_fs::remove_file(&bundle_location).await {
        eprintln!("Failed to remove tar file: {:?}", err);
    }

    Ok(())
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