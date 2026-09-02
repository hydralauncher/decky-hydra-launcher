use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::fs as tokio_fs;
use unicode_normalization::UnicodeNormalization;

use crate::wine::get_windows_like_user_profile_path;

/// Mirrors the reference install_directory(): the game root is
/// steamapps/common/<game> when the executable lives there, else the
/// executable's directory.
pub(crate) fn install_dir_from_executable(executable_path: Option<&str>) -> Option<String> {
    let path = executable_path?.replace('\\', "/");
    let marker = "/steamapps/common/";
    if let Some(index) = path.to_ascii_lowercase().find(marker) {
        let start = index + marker.len();
        let end = path[start..]
            .find('/')
            .map(|offset| start + offset)
            .unwrap_or(path.len());
        return Some(path[..end].to_string());
    }
    path.rsplit_once('/').map(|(parent, _)| parent.to_string())
}

/// Steam installation root: derived from the game's executable when it lives
/// in a steamapps tree (covers SD-card/secondary libraries), else the default
/// home locations.
pub(crate) fn steam_root(executable_path: Option<&str>) -> Option<PathBuf> {
    if let Some(path) = executable_path {
        let path = path.replace('\\', "/");
        let marker = "/steamapps/";
        if let Some(index) = path.to_ascii_lowercase().find(marker) {
            return Some(PathBuf::from(&path[..index]));
        }
    }

    let home = dirs::home_dir()?;
    for candidate in [
        home.join(".steam/steam"),
        home.join(".steam/root"),
        home.join(".local/share/Steam"),
    ] {
        if candidate.is_dir() {
            return Some(candidate);
        }
    }
    None
}

pub const API_BASE: &str = "https://hydra-api-us-east-1.losbroxas.org";

const MAX_SNAPSHOT_FILES: usize = 500;
// Matches the launcher's upload-limits.ts exactly.
const MAX_SNAPSHOT_BYTES: u64 = 2_147_483_647;
const MAX_CONCURRENT_TRANSFERS: usize = 8;

// ---------------------------------------------------------------------------
// Auth
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Auth {
    pub access_token: String,
    pub refresh_token: String,
    pub token_expiration_timestamp: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RefreshResponse {
    expires_in: f64,
    access_token: String,
    // The server does not rotate the refresh token; older plugin versions
    // required this field and hard-failed every refresh.
    refresh_token: Option<String>,
}

pub async fn ensure_fresh_token(client: &reqwest::Client, auth: &Auth) -> Result<Auth> {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis() as f64;

    let expired = match auth.token_expiration_timestamp {
        Some(ts) => ts < now_ms + 60_000.0,
        None => false,
    };

    if !expired {
        return Ok(auth.clone());
    }

    let response = client
        .post(format!("{API_BASE}/auth/refresh"))
        .header("User-Agent", "Hydra-Decky-Plugin")
        .json(&serde_json::json!({ "refreshToken": auth.refresh_token }))
        .send()
        .await
        .context("Failed to reach auth refresh endpoint")?
        .error_for_status()
        .context("Auth refresh rejected")?
        .json::<RefreshResponse>()
        .await
        .context("Invalid auth refresh response")?;

    Ok(Auth {
        access_token: response.access_token,
        refresh_token: response.refresh_token.unwrap_or(auth.refresh_token.clone()),
        token_expiration_timestamp: Some(now_ms + response.expires_in * 1000.0),
    })
}

// ---------------------------------------------------------------------------
// Snapshot identity (mirrors hydra native addon identity/mod.rs)
// ---------------------------------------------------------------------------

fn normalize_text(value: &str) -> String {
    value.nfc().collect::<String>()
}

fn normalize_rule_path(value: &str) -> String {
    normalize_text(&value.replace('\\', "/"))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotVariant {
    pub variant_id: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub steam_id64: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub concrete_folder_id: Option<String>,
}

pub fn build_default_variant(shop: &str, object_id: &str) -> SnapshotVariant {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct CanonicalVariant<'a> {
        variant_id_version: u32,
        shop: &'a str,
        object_id: &'a str,
        kind: &'a str,
    }

    let canonical = CanonicalVariant {
        variant_id_version: 1,
        shop,
        object_id,
        kind: "default",
    };
    let serialized = serde_json::to_vec(&canonical).expect("variant serializes");
    let variant_id = format!("{:x}", Sha256::digest(serialized));

    SnapshotVariant {
        variant_id,
        kind: "default".to_string(),
        steam_id64: None,
        concrete_folder_id: None,
    }
}

/// Per-user-folder variant, mirroring the launcher's identity code (verified
/// against its test vectors below).
pub fn build_opaque_variant(shop: &str, object_id: &str, concrete_folder_id: &str) -> SnapshotVariant {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct CanonicalVariant<'a> {
        variant_id_version: u32,
        shop: &'a str,
        object_id: &'a str,
        kind: &'a str,
        concrete_folder_id: &'a str,
    }

    let normalized = concrete_folder_id.nfc().collect::<String>().to_lowercase();
    let canonical = CanonicalVariant {
        variant_id_version: 1,
        shop,
        object_id,
        kind: "opaque-folder",
        concrete_folder_id: &normalized,
    };
    let serialized = serde_json::to_vec(&canonical).expect("variant serializes");
    let variant_id = format!("{:x}", Sha256::digest(serialized));

    SnapshotVariant {
        variant_id,
        kind: "opaque-folder".to_string(),
        steam_id64: None,
        concrete_folder_id: Some(normalized),
    }
}

// ---------------------------------------------------------------------------
// Aggregate hash (mirrors hydra native addon hashing/aggregate.rs)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SnapshotFileEntry {
    pub variant_id: String,
    pub raw_path: String,
    pub relative_path: String,
    pub hash: String,
    pub size_bytes: u64,
    pub last_modified_at: String,
}

pub fn build_aggregate_hash(
    variants: &[SnapshotVariant],
    files: &[SnapshotFileEntry],
) -> Result<String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct CanonicalVariant<'a> {
        variant_id: &'a str,
        kind: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        steam_id64: Option<&'a str>,
        #[serde(skip_serializing_if = "Option::is_none")]
        concrete_folder_id: Option<&'a str>,
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct CanonicalFile<'a> {
        variant_id: &'a str,
        raw_path: &'a str,
        relative_path: &'a str,
        hash: &'a str,
        size_bytes: u64,
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct CanonicalSnapshot<'a> {
        snapshot_hash_version: u32,
        variants: Vec<CanonicalVariant<'a>>,
        files: Vec<CanonicalFile<'a>>,
    }

    let mut sorted_variants: Vec<&SnapshotVariant> = variants.iter().collect();
    sorted_variants.sort_by(|a, b| a.variant_id.cmp(&b.variant_id));

    let mut normalized_files: Vec<SnapshotFileEntry> = files
        .iter()
        .map(|file| SnapshotFileEntry {
            variant_id: file.variant_id.clone(),
            raw_path: normalize_rule_path(&file.raw_path),
            relative_path: normalize_text(&file.relative_path),
            hash: file.hash.clone(),
            size_bytes: file.size_bytes,
            last_modified_at: file.last_modified_at.clone(),
        })
        .collect();
    normalized_files.sort_by(|a, b| {
        a.variant_id
            .cmp(&b.variant_id)
            .then_with(|| a.raw_path.cmp(&b.raw_path))
            .then_with(|| a.relative_path.cmp(&b.relative_path))
            .then_with(|| a.hash.cmp(&b.hash))
            .then_with(|| a.size_bytes.cmp(&b.size_bytes))
    });

    let canonical = CanonicalSnapshot {
        snapshot_hash_version: 1,
        variants: sorted_variants
            .iter()
            .map(|variant| CanonicalVariant {
                variant_id: &variant.variant_id,
                kind: &variant.kind,
                steam_id64: variant.steam_id64.as_deref(),
                concrete_folder_id: variant.concrete_folder_id.as_deref(),
            })
            .collect(),
        files: normalized_files
            .iter()
            .map(|file| CanonicalFile {
                variant_id: &file.variant_id,
                raw_path: &file.raw_path,
                relative_path: &file.relative_path,
                hash: &file.hash,
                size_bytes: file.size_bytes,
            })
            .collect(),
    };

    let serialized = serde_json::to_vec(&canonical).context("aggregate hash serialization")?;
    Ok(format!("{:x}", Sha256::digest(serialized)))
}

// ---------------------------------------------------------------------------
// Local state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudSaveState {
    pub snapshot_id: String,
    pub version: u64,
    pub aggregate_hash: String,
    pub wine_prefix_path: Option<String>,
    pub updated_at: String,
}

fn state_dir() -> Result<PathBuf> {
    Ok(dirs::config_dir()
        .ok_or_else(|| anyhow!("No config dir"))?
        .join("hydralauncher")
        .join("decky-cloud-saves"))
}

fn state_path(shop: &str, object_id: &str) -> Result<PathBuf> {
    if !object_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(anyhow!("Invalid object id"));
    }
    Ok(state_dir()?.join(format!("{shop}-{object_id}.json")))
}

fn read_state(shop: &str, object_id: &str) -> Option<CloudSaveState> {
    let path = state_path(shop, object_id).ok()?;
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

fn write_state(shop: &str, object_id: &str, state: &CloudSaveState) -> Result<()> {
    let dir = state_dir()?;
    std::fs::create_dir_all(&dir)?;
    let path = state_path(shop, object_id)?;
    std::fs::write(path, serde_json::to_string_pretty(state)?)?;
    Ok(())
}

/// State is a best-effort local record; losing it must not fail an operation
/// whose remote side already succeeded.
fn write_state_logged(shop: &str, object_id: &str, state: &CloudSaveState) {
    if let Err(err) = write_state(shop, object_id, state) {
        eprintln!("Failed to persist cloud save state: {err:#}");
    }
}

// ---------------------------------------------------------------------------
// API types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct RemoteSnapshotSummary {
    pub id: String,
    pub version: u64,
    pub created_at: String,
    pub updated_at: String,
    pub file_count: u64,
    pub total_size_bytes: u64,
    pub aggregate_hash: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrepareSnapshotResponse {
    pending_snapshot_id: String,
    snapshot_hash: String,
    files: Vec<PrepareSnapshotFile>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrepareSnapshotFile {
    variant_id: String,
    raw_path: String,
    relative_path: String,
    status: String,
    upload_url: Option<String>,
    required_headers: Option<HashMap<String, String>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CommitSnapshotResponse {
    snapshot_id: String,
    version: u64,
    file_count: u64,
    total_size_bytes: u64,
    aggregate_hash: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RestoreManifestResponse {
    snapshot: RestoreManifestSnapshot,
    #[serde(default)]
    variants: Vec<SnapshotVariant>,
    files: Vec<RestoreManifestFile>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RestoreManifestSnapshot {
    id: String,
    version: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct RestoreManifestFile {
    variant_id: String,
    raw_path: String,
    relative_path: String,
    hash: String,
    size_bytes: u64,
    last_modified_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct DownloadUrlFile {
    variant_id: String,
    raw_path: String,
    relative_path: String,
    hash: String,
    size_bytes: u64,
    download_url: String,
}

fn is_valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn is_safe_relative_path(value: &str) -> bool {
    !value.is_empty()
        && !value.contains('\\')
        && !value.starts_with('/')
        && value
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

fn is_safe_raw_path(value: &str) -> bool {
    !value.is_empty()
        && !value.contains('\\')
        && !value.split('/').any(|segment| segment == "..")
}

fn is_safe_manifest_file(file: &RestoreManifestFile) -> bool {
    is_valid_hash(&file.hash)
        && is_safe_raw_path(&file.raw_path)
        && is_safe_relative_path(&file.relative_path)
}

fn hydra_client(auth: &Auth) -> Result<reqwest::Client> {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::AUTHORIZATION,
        format!("Bearer {}", auth.access_token).parse()?,
    );
    headers.insert(
        reqwest::header::USER_AGENT,
        "Hydra-Decky-Plugin".parse().unwrap(),
    );

    Ok(reqwest::Client::builder()
        .default_headers(headers)
        .connect_timeout(std::time::Duration::from_secs(30))
        .timeout(std::time::Duration::from_secs(1800))
        .build()?)
}

async fn send_checked(builder: reqwest::RequestBuilder) -> Result<reqwest::Response> {
    let response = builder.send().await?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        let body: String = body.chars().take(512).collect();
        return Err(anyhow!("Request failed with status {status}: {body}"));
    }
    Ok(response)
}

pub async fn list_snapshots(
    client: &reqwest::Client,
    shop: &str,
    object_id: &str,
) -> Result<Vec<RemoteSnapshotSummary>> {
    let mut snapshots = send_checked(
        client
            .get(format!("{API_BASE}/profile/cloud-saves/snapshots"))
            .query(&[("shop", shop), ("objectId", object_id)]),
    )
    .await
    .context("Failed to list cloud save snapshots")?
    .json::<Vec<RemoteSnapshotSummary>>()
    .await
    .context("Invalid snapshot list response")?;

    snapshots.sort_by_key(|snapshot| snapshot.version);
    Ok(snapshots)
}

// ---------------------------------------------------------------------------
// Discovery: scan local saves via the launcher's own manifest index.
// ---------------------------------------------------------------------------

struct DiscoveredFile {
    entry: SnapshotFileEntry,
    source_path: PathBuf,
}

pub struct DiscoveryOutput {
    files: Vec<DiscoveredFile>,
    variants: Vec<SnapshotVariant>,
}

async fn discover_files(
    object_id: &str,
    shop: &str,
    wine_prefix: Option<&str>,
) -> Result<DiscoveryOutput> {
    let rules = crate::rules::GameRules::load(object_id)?
        .ok_or_else(|| {
            anyhow!("Save rules unavailable for this game; open Hydra launcher once, then retry")
        })?;

    let ctx = crate::scanner::ScanContext::build(object_id, shop, wine_prefix);
    let candidates = crate::scanner::scan_game_saves(&ctx, &rules);

    let default_variant = build_default_variant(shop, object_id);
    let mut variants: Vec<SnapshotVariant> = vec![default_variant.clone()];

    let mut files = Vec::new();
    for (real_path, tokenized) in candidates {
        // A listed file that can no longer be read must not be silently
        // dropped: committing without it would delete the save on other
        // devices. Fail retryable so discovery re-runs once.
        let metadata = tokio_fs::metadata(&real_path).await.map_err(|_| {
            anyhow!("Save file changed during sync; aborting before commit")
        })?;
        if !metadata.is_file() {
            return Err(anyhow!(
                "Save file changed during sync; aborting before commit"
            ));
        }

        let size_bytes = metadata.len();
        let last_modified_at: chrono::DateTime<chrono::Utc> =
            metadata.modified().unwrap_or(std::time::SystemTime::now()).into();

        // Snapshot identity uses the manifest rule path, matching what the
        // desktop launcher produces for the same file.
            let rule_match = rules
                .match_rule(&tokenized, ctx.windows_compat, shop)
                .ok_or_else(|| anyhow!("No save rule matches discovered file: {tokenized}"))?;

            let (raw_path, relative_path) =
                crate::rules::split_rule_match(
                    rule_match.raw_path,
                    rule_match.kind,
                    &tokenized,
                    rule_match.store_user.as_deref(),
                );
        let variant = match &rule_match.store_user {
            Some(folder) => {
                let variant = build_opaque_variant(shop, object_id, folder);
                if !variants.iter().any(|v| v.variant_id == variant.variant_id) {
                    variants.push(variant.clone());
                }
                variant
            }
            None => default_variant.clone(),
        };

        let hash = sha256_file_hex(&real_path).await.map_err(|_| {
            anyhow!("Save file changed during sync; aborting before commit")
        })?;

        files.push(DiscoveredFile {
            entry: SnapshotFileEntry {
                variant_id: variant.variant_id,
                raw_path,
                relative_path,
                hash,
                size_bytes,
                last_modified_at: last_modified_at
                    .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            },
            source_path: real_path,
        });
    }

    if files.is_empty() {
        return Err(anyhow!("No save files found for this game"));
    }

    let total: u64 = files.iter().map(|f| f.entry.size_bytes).sum();
    eprintln!(
        "discovery: {} files, {} bytes, {} variants",
        files.len(),
        total,
        variants.len()
    );

    Ok(DiscoveryOutput { files, variants })
}


async fn sha256_file_hex(path: &Path) -> Result<String> {
    let bytes = tokio_fs::read(path).await?;
    Ok(format!("{:x}", Sha256::digest(&bytes)))
}

// ---------------------------------------------------------------------------
// Upload (sync)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncResult {
    pub ok: bool,
    pub snapshot_id: String,
    pub version: u64,
    pub file_count: u64,
    pub total_size_bytes: u64,
    pub uploaded_files: usize,
    pub skipped_files: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth: Option<Auth>,
}

pub async fn sync_cloud_save(
    auth_json: &str,
    object_id: &str,
    shop: &str,
    wine_prefix: Option<&str>,
    force: bool,
) -> Result<SyncResult> {
    let auth: Auth = serde_json::from_str(auth_json).context("Invalid auth payload")?;
    let base_client = reqwest::Client::new();
    let mut auth = ensure_fresh_token(&base_client, &auth).await?;
    let mut client = hydra_client(&auth)?;

    let hostname = hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .ok()
        .filter(|h| !h.is_empty());

    // Retry once on conflict / expired pending snapshot / files changing
    // mid-sync. Discovery runs inside the loop so a retry re-scans and
    // re-hashes the save files from scratch.
    let mut last_error: Option<anyhow::Error> = None;
    let mut result: Option<(CommitSnapshotResponse, usize, usize)> = None;

    for attempt in 0..2 {
        let discovered = match discover_files(object_id, shop, wine_prefix).await {
            Ok(discovered) => discovered,
            Err(err) => {
                let retryable = err
                    .chain()
                    .any(|cause| cause.to_string().contains("changed during sync"));
                if attempt == 0 && retryable {
                    last_error = Some(err);
                    continue;
                }
                return Err(err);
            }
        };
        let files: Vec<SnapshotFileEntry> = discovered.files.iter().map(|f| f.entry.clone()).collect();

        let total_size: u64 = files.iter().map(|f| f.size_bytes).sum();
        if files.len() > MAX_SNAPSHOT_FILES {
            return Err(anyhow!(
                "Too many save files ({} > {MAX_SNAPSHOT_FILES})",
                files.len()
            ));
        }
        if total_size > MAX_SNAPSHOT_BYTES {
            return Err(anyhow!("Save files exceed 2 GiB limit"));
        }

        let aggregate_hash = build_aggregate_hash(&discovered.variants, &files)?;

        let snapshots = list_snapshots(&client, shop, object_id).await?;

        // Refuse to silently overwrite a remote snapshot this device has not
        // seen. Auto-sync passes force=false; manual sync confirms with the
        // user first and passes force=true.
        if !force {
            if let Some(latest) = snapshots.last() {
                let state = read_state(shop, object_id);
                let remote_newer = match &state {
                    Some(local) => {
                        latest.version > local.version
                            || (latest.version == local.version
                                && latest.aggregate_hash != local.aggregate_hash)
                    }
                    None => true,
                };
                if remote_newer {
                    return Err(anyhow!("remote-newer"));
                }
            }
        }

        let base_version = snapshots.last().map(|s| s.version).unwrap_or(0);

        match prepare_upload_commit(
            &client,
            shop,
            object_id,
            hostname.as_deref(),
            base_version,
            &discovered.variants,
            &files,
            &discovered.files,
            &aggregate_hash,
        )
        .await
        {
            Ok((committed, uploaded_files, skipped_files)) => {
                if committed.version != base_version + 1
                    || committed.file_count != files.len() as u64
                    || committed.total_size_bytes != total_size
                    || committed.aggregate_hash != aggregate_hash
                {
                    return Err(anyhow!("Committed snapshot is inconsistent"));
                }
                result = Some((committed, uploaded_files, skipped_files));
                break;
            }
            Err(err) => {
                let retryable = err.chain().any(|cause| {
                    let msg = cause.to_string();
                    msg.contains("409")
                        || msg.contains("401")
                        || msg.contains("403")
                        || msg.contains("pending-snapshot")
                        || msg.contains("pending_snapshot")
                        || msg.contains("changed during sync")
                });
                if attempt == 0 && retryable {
                    // The upload phase can outlive the access token; refresh
                    // before the retry when the failure was auth-related.
                    let msg = format!("{err:#}");
                    if msg.contains("401") || msg.contains("403") {
                        auth = ensure_fresh_token(&base_client, &auth).await?;
                        client = hydra_client(&auth)?;
                    }
                    last_error = Some(err);
                    continue;
                }
                return Err(err);
            }
        }
    }

    let (committed, uploaded_files, skipped_files) =
        result.ok_or_else(|| last_error.unwrap_or_else(|| anyhow!("Commit did not complete")))?;

    write_state_logged(
        shop,
        object_id,
        &CloudSaveState {
            snapshot_id: committed.snapshot_id.clone(),
            version: committed.version,
            aggregate_hash: committed.aggregate_hash.clone(),
            wine_prefix_path: wine_prefix.map(|p| p.to_string()),
            updated_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        },
    );

    Ok(SyncResult {
        ok: true,
        snapshot_id: committed.snapshot_id,
        version: committed.version,
        file_count: committed.file_count,
        total_size_bytes: committed.total_size_bytes,
        uploaded_files,
        skipped_files,
        auth: Some(auth),
    })
}

#[allow(clippy::too_many_arguments)]
async fn prepare_upload_commit(
    client: &reqwest::Client,
    shop: &str,
    object_id: &str,
    hostname: Option<&str>,
    base_version: u64,
    variants: &[SnapshotVariant],
    files: &[SnapshotFileEntry],
    discovered: &[DiscoveredFile],
    aggregate_hash: &str,
) -> Result<(CommitSnapshotResponse, usize, usize)> {
    let mut payload = serde_json::json!({
        "shop": shop,
        "objectId": object_id,
        "platform": "linux",
        "snapshotHash": aggregate_hash,
        "baseVersion": base_version,
        "customPathRawPaths": [],
        "variants": variants,
        "files": files.iter().map(|f| serde_json::json!({
            "variantId": f.variant_id,
            "rawPath": f.raw_path,
            "relativePath": f.relative_path,
            "hash": f.hash,
            "sizeBytes": f.size_bytes,
            "lastModifiedAt": f.last_modified_at,
        })).collect::<Vec<_>>(),
    });

    // The reference client omits the key entirely when no hostname is known.
    if let Some(hostname) = hostname.filter(|h| !h.is_empty()) {
        payload["hostname"] = serde_json::Value::String(hostname.to_string());
    }

    let response = send_checked(
        client
            .post(format!("{API_BASE}/profile/cloud-saves/prepare-snapshot"))
            .json(&payload),
    )
    .await
    .context("Failed to prepare snapshot")?
    .json::<PrepareSnapshotResponse>()
    .await
    .context("Invalid prepare snapshot response")?;

    if response.snapshot_hash != aggregate_hash {
        return Err(anyhow!("Prepare snapshot hash does not match the proposal"));
    }
    if response.files.len() != files.len() {
        return Err(anyhow!("Prepare snapshot response does not cover proposal files"));
    }

    let source_by_key: HashMap<String, &DiscoveredFile> = discovered
        .iter()
        .map(|f| {
            (
                format!("{}\u{0}{}\u{0}{}", f.entry.variant_id, f.entry.raw_path, f.entry.relative_path),
                f,
            )
        })
        .collect();
    let source_by_blob: HashMap<String, &DiscoveredFile> = discovered
        .iter()
        .map(|f| (format!("{}\u{0}{}", f.entry.hash, f.entry.size_bytes), f))
        .collect();

    let mut upload_jobs: HashMap<String, (String, String, PathBuf, usize, String)> =
        HashMap::new();
    let mut skipped_files = 0usize;

    for file in &response.files {
        let key = format!("{}\u{0}{}\u{0}{}", file.variant_id, file.raw_path, file.relative_path);
        let proposal = files
            .iter()
            .find(|f| {
                f.variant_id == file.variant_id
                    && f.raw_path == file.raw_path
                    && f.relative_path == file.relative_path
            })
            .ok_or_else(|| anyhow!("Unknown prepare response file"))?;

        if file.status == "skip" {
            skipped_files += 1;
            continue;
        }

        let upload_url = file
            .upload_url
            .clone()
            .ok_or_else(|| anyhow!("Missing upload URL"))?;
        if !upload_url.starts_with("https://") {
            return Err(anyhow!("Refusing non-HTTPS upload URL"));
        }
        let required_headers = file
            .required_headers
            .clone()
            .ok_or_else(|| anyhow!("Missing required headers"))?;
        if required_headers.len() != 2 {
            return Err(anyhow!("Unexpected prepare upload headers"));
        }

        let expected_checksum = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            hex_decode(&proposal.hash)?,
        );
        if required_headers.get("Content-Length").map(String::as_str)
            != Some(proposal.size_bytes.to_string().as_str())
            || required_headers.get("x-amz-checksum-sha256").map(String::as_str)
                != Some(expected_checksum.as_str())
        {
            return Err(anyhow!("Prepare upload headers do not match the proposal"));
        }

        let source = source_by_key
            .get(&key)
            .or_else(|| source_by_blob.get(&format!("{}\u{0}{}", proposal.hash, proposal.size_bytes)))
            .ok_or_else(|| anyhow!("Missing local upload source"))?;

        let blob_key = format!("{}\u{0}{}", proposal.hash, proposal.size_bytes);
        upload_jobs
            .entry(blob_key)
            .or_insert_with(|| {
                (
                    upload_url,
                    expected_checksum,
                    source.source_path.clone(),
                    proposal.size_bytes as usize,
                    proposal.hash.clone(),
                )
            });
    }

    let jobs: Vec<(String, String, PathBuf, usize, String)> =
        upload_jobs.into_values().collect();
    let uploaded_files = response
        .files
        .iter()
        .filter(|f| f.status == "upload")
        .count();
    eprintln!(
        "prepare: {} files to upload ({} unique blobs), {} already remote",
        uploaded_files,
        jobs.len(),
        skipped_files
    );
    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_TRANSFERS));
    let mut join_set = tokio::task::JoinSet::new();

    for (url, checksum, path, size, expected_hash) in jobs {
        let permit = semaphore.clone().acquire_owned().await?;
        let upload_client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(30))
            .timeout(std::time::Duration::from_secs(1800))
            .build()?;
        join_set.spawn(async move {
            let _permit = permit;
            // A file deleted or locked between discovery and upload is a
            // mutation: surface it in the retryable class so the sync
            // re-discovers from scratch.
            let body = tokio_fs::read(&path).await.map_err(|_| {
                anyhow!("Save file changed during sync; aborting before commit")
            })?;

            // Re-hash at upload time: the file may have changed since
            // discovery, and the committed snapshot must match these bytes.
            let actual_hash = format!("{:x}", Sha256::digest(&body));
            if actual_hash != expected_hash {
                return Err(anyhow!(
                    "Save file changed during sync; aborting before commit"
                ));
            }

            let resp = upload_client
                .put(&url)
                .header("Content-Length", size.to_string())
                .header("x-amz-checksum-sha256", checksum)
                .body(body)
                .send()
                .await?;
            let status = resp.status();
            if !status.is_success() {
                // Do not include the URL: it is a signed credential.
                return Err(anyhow!("Blob upload failed with status {status}"));
            }
            Ok::<(), anyhow::Error>(())
        });
    }

    while let Some(result) = join_set.join_next().await {
        result.context("Upload task panicked")??;
    }

    // Re-hash every proposed file before commit — including ones the server
    // marked "skip". A file that changed since discovery would otherwise be
    // committed with a stale identity while the local change stays unpreserved.
    for source in discovered {
        let actual_hash = sha256_file_hex(&source.source_path).await.map_err(|_| {
            anyhow!("Save file changed during sync; aborting before commit")
        })?;
        if actual_hash != source.entry.hash {
            return Err(anyhow!(
                "Save file changed during sync; aborting before commit"
            ));
        }
    }

    // Commit, retrying once on transport failure (the reference client does
    // the same to tolerate a dropped connection after the server committed).
    let mut committed: Option<CommitSnapshotResponse> = None;
    for attempt in 0..2 {
        let result = client
            .post(format!("{API_BASE}/profile/cloud-saves/commit-snapshot"))
            .json(&serde_json::json!({ "pendingSnapshotId": response.pending_snapshot_id }))
            .send()
            .await;

        match result {
            Ok(resp) => {
                let status = resp.status();
                if !status.is_success() {
                    let body = resp.text().await.unwrap_or_default();
                    let body: String = body.chars().take(512).collect();
                    return Err(anyhow!("Commit failed with status {status}: {body}"));
                }
                let committed_response = resp
                    .json::<CommitSnapshotResponse>()
                    .await
                    .context("Invalid commit snapshot response")?;
                committed = Some(committed_response);
                break;
            }
            Err(err) => {
                if attempt == 0 && (err.is_connect() || err.is_timeout() || err.is_request()) {
                    continue;
                }
                return Err(err).context("Failed to commit snapshot");
            }
        }
    }

    Ok((
        committed.ok_or_else(|| anyhow!("Commit did not complete"))?,
        uploaded_files,
        skipped_files,
    ))
}

fn hex_decode(hex: &str) -> Result<Vec<u8>> {
    if hex.len() % 2 != 0 {
        return Err(anyhow!("Invalid hex string"));
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&hex[i..i + 2], 16).map_err(|e| anyhow!("Invalid hex: {e}"))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Restore
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreResult {
    pub ok: bool,
    pub snapshot_id: String,
    pub version: u64,
    pub restored_files: usize,
    pub skipped_files: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth: Option<Auth>,
}

struct RestoreContext {
    wine_prefix: Option<String>,
    wine_user_name: Option<String>,
    home_dir: Option<PathBuf>,
    install_dir: Option<String>,
    steam_root: Option<PathBuf>,
    windows_compat: bool,
    variant_folders: HashMap<String, String>,
}

impl RestoreContext {
    fn resolve(&self, raw_path: &str, relative_path: &str) -> Option<PathBuf> {
        let raw_path = raw_path.replace('\\', "/");
        let profile_root = self.wine_user_profile_root();

        let base = if let Some(rest) = raw_path.strip_prefix("<winAppData>") {
            profile_root.map(|p| format!("{p}/AppData/Roaming{rest}"))
        } else if let Some(rest) = raw_path.strip_prefix("<winLocalAppData>") {
            profile_root.map(|p| format!("{p}/AppData/Local{rest}"))
        } else if let Some(rest) = raw_path.strip_prefix("<winDocuments>") {
            profile_root.map(|p| format!("{p}/Documents{rest}"))
        } else if let Some(rest) = raw_path.strip_prefix("<winPublic>") {
            self.wine_prefix
                .as_ref()
                .map(|p| format!("{p}/drive_c/users/Public{rest}"))
        } else if let Some(rest) = raw_path.strip_prefix("<winProgramData>") {
            self.wine_prefix
                .as_ref()
                .map(|p| format!("{p}/drive_c/ProgramData{rest}"))
        } else if let Some(rest) = raw_path
            .strip_prefix("<winDir>")
            .or_else(|| raw_path.strip_prefix("<windows>"))
        {
            self.wine_prefix
                .as_ref()
                .map(|p| format!("{p}/drive_c/windows{rest}"))
        } else if let Some(rest) = raw_path.strip_prefix("<osUserName>") {
            self.wine_user_name
                .as_ref()
                .map(|n| format!("{n}{rest}"))
        } else if let Some(rest) = raw_path.strip_prefix("<home>") {
            // Under Proton <home> is the wine user profile (mirrors the
            // scanner and the reference launcher); on native games it is the
            // linux home directory.
            if self.windows_compat {
                profile_root.map(|p| format!("{p}{rest}"))
            } else {
                self.home_dir
                    .as_ref()
                    .map(|p| format!("{}{rest}", p.to_string_lossy()))
            }
        } else if let Some(rest) = raw_path.strip_prefix("<xdgData>") {
            self.home_dir
                .as_ref()
                .map(|p| format!("{}/.local/share{rest}", p.to_string_lossy()))
        } else if let Some(rest) = raw_path.strip_prefix("<xdgConfig>") {
            self.home_dir
                .as_ref()
                .map(|p| format!("{}/.config{rest}", p.to_string_lossy()))
        } else if let Some(rest) = raw_path.strip_prefix("<base>") {
            self.install_dir.as_ref().map(|d| format!("{d}{rest}"))
        } else if let Some(rest) = raw_path.strip_prefix("<root>") {
            self.steam_root
                .as_ref()
                .map(|r| format!("{}{rest}", r.to_string_lossy()))
        } else if raw_path.contains("<storeUserId>") {
            // Resolve against the variant's concrete folder id when known.
            None
        } else if raw_path.starts_with("C:/") || raw_path.starts_with("c:/") {
            self.resolve_windows_path(&raw_path)
        } else if raw_path.starts_with('/') {
            // Manifest data is server-supplied: confine absolute paths to the
            // user's home directory so a hostile snapshot cannot write
            // anywhere else on the system.
            let home = self.home_dir.as_ref()?.to_string_lossy().to_string();
            if raw_path == home || raw_path.starts_with(&format!("{home}/")) {
                Some(raw_path.clone())
            } else {
                None
            }
        } else {
            None
        };

        base.map(|base| {
            let mut path = PathBuf::from(base);
            for segment in relative_path.split('/') {
                if !segment.is_empty() && segment != "." && segment != ".." {
                    path.push(segment);
                }
            }
            path
        })
    }

    fn resolve_with_variant(
        &self,
        variant_id: &str,
        raw_path: &str,
        relative_path: &str,
    ) -> Option<PathBuf> {
        if raw_path.contains("<storeUserId>") {
            let concrete = self.variant_folders.get(variant_id)?;
            // Server-supplied value: reject anything that could escape the
            // target directory after substitution.
            let safe = !concrete.is_empty()
                && concrete.len() <= 255
                && !concrete.contains(['/', '\\', '\0'])
                && concrete != "."
                && concrete != "..";
            if !safe {
                return None;
            }
            let replaced = raw_path.replace("<storeUserId>", concrete);
            return self.resolve(&replaced, relative_path);
        }
        self.resolve(raw_path, relative_path)
    }

    fn wine_user_profile_root(&self) -> Option<String> {
        let prefix = self.wine_prefix.as_ref()?;
        let name = self.wine_user_name.as_ref()?;
        Some(format!("{prefix}/drive_c/users/{name}"))
    }

    fn resolve_windows_path(&self, path: &str) -> Option<String> {
        let prefix = self.wine_prefix.as_ref()?;
        let without_drive = path[3..].trim_start_matches('/');

        // Substitute foreign windows user profile with the local one.
        let adjusted = if let Some(local_name) = &self.wine_user_name {
            if let Some(rest) = without_drive
                .strip_prefix("users/")
                .or_else(|| without_drive.strip_prefix("Users/"))
            {
                let mut parts = rest.splitn(2, '/');
                let name = parts.next().unwrap_or("");
                let tail = parts.next().unwrap_or("");
                if name.eq_ignore_ascii_case("Public") {
                    format!("users/Public/{tail}")
                } else {
                    format!("users/{local_name}/{tail}")
                }
            } else {
                without_drive.to_string()
            }
        } else {
            without_drive.to_string()
        };

        Some(format!("{prefix}/drive_c/{adjusted}"))
    }
}

pub async fn restore_cloud_save(
    auth_json: &str,
    object_id: &str,
    shop: &str,
    wine_prefix: Option<&str>,
) -> Result<RestoreResult> {
    let auth: Auth = serde_json::from_str(auth_json).context("Invalid auth payload")?;
    let base_client = reqwest::Client::new();
    let auth = ensure_fresh_token(&base_client, &auth).await?;
    let client = hydra_client(&auth)?;

    let snapshots = list_snapshots(&client, shop, object_id).await?;
    let latest = snapshots
        .last()
        .ok_or_else(|| anyhow!("No cloud save snapshot exists for this game"))?;
    let manifest = send_checked(
        client
            .get(format!(
                "{API_BASE}/profile/cloud-saves/snapshot-restore-manifest"
            ))
            .query(&[("snapshotId", latest.id.as_str())]),
    )
    .await
    .context("Failed to fetch restore manifest")?
    .json::<RestoreManifestResponse>()
    .await
    .context("Invalid restore manifest response")?;

    // Cross-check manifest against the snapshot summary.
    let manifest_total_size: u64 = manifest.files.iter().try_fold(0u64, |acc, f| {
        acc.checked_add(f.size_bytes)
    }).ok_or_else(|| anyhow!("Restore manifest size overflow"))?;

    let mut identities = std::collections::HashSet::new();
    for f in &manifest.files {
        if !identities.insert((&f.variant_id, &f.raw_path, &f.relative_path)) {
            return Err(anyhow!("Restore manifest contains duplicate file identity"));
        }
    }

    if manifest.snapshot.id != latest.id
        || manifest.snapshot.version != latest.version
        || manifest.files.len() as u64 != latest.file_count
        || manifest_total_size != latest.total_size_bytes
    {
        return Err(anyhow!("Restore manifest does not match the snapshot summary"));
    }

    let manifest_entries: Vec<SnapshotFileEntry> = manifest
        .files
        .iter()
        .map(|f| SnapshotFileEntry {
            variant_id: f.variant_id.clone(),
            raw_path: f.raw_path.clone(),
            relative_path: f.relative_path.clone(),
            hash: f.hash.clone(),
            size_bytes: f.size_bytes,
            last_modified_at: f.last_modified_at.clone(),
        })
        .collect();
    let manifest_hash = build_aggregate_hash(&manifest.variants, &manifest_entries)
        .context("Failed to verify restore manifest hash")?;
    if manifest_hash != latest.aggregate_hash {
        return Err(anyhow!("Restore manifest aggregate hash mismatch"));
    }

    let download_files = send_checked(
        client
            .get(format!(
                "{API_BASE}/profile/cloud-saves/snapshot-download-urls"
            ))
            .query(&[("snapshotId", latest.id.as_str())]),
    )
    .await
    .context("Failed to fetch download URLs")?
    .json::<Vec<DownloadUrlFile>>()
    .await
    .context("Invalid download URLs response")?;

    let temp = tempfile::tempdir().context("Failed to create temp dir")?;

    // Download blobs, deduplicated by content hash. Only hashes referenced by
    // the already-validated manifest are fetched; the hash becomes a path
    // component, so anything else is rejected.
    let manifest_hashes: std::collections::HashSet<&str> =
        manifest.files.iter().map(|f| f.hash.as_str()).collect();
    let mut blob_urls: HashMap<String, (String, u64)> = HashMap::new();
    for file in &download_files {
        if !is_valid_hash(&file.hash) || !manifest_hashes.contains(file.hash.as_str()) {
            continue;
        }
        blob_urls
            .entry(file.hash.clone())
            .or_insert_with(|| (file.download_url.clone(), file.size_bytes));
    }

    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_TRANSFERS));
    let mut join_set = tokio::task::JoinSet::new();
    let download_client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .timeout(std::time::Duration::from_secs(1800))
        .build()?;

    for (hash, (url, _size)) in &blob_urls {
        let permit = semaphore.clone().acquire_owned().await?;
        let client = download_client.clone();
        let dest = temp.path().join(hash);
        let url = url.clone();
        let hash = hash.clone();
        join_set.spawn(async move {
            let _permit = permit;
            let bytes = client.get(&url).send().await?.error_for_status()?.bytes().await?;
            let actual = format!("{:x}", Sha256::digest(&bytes));
            if actual != hash {
                return Err(anyhow!("Downloaded blob failed hash verification"));
            }
            tokio_fs::write(&dest, &bytes).await?;
            Ok::<(), anyhow::Error>(())
        });
    }

    while let Some(result) = join_set.join_next().await {
        result.context("Download task panicked")??;
    }

    // Downloads may have outlived the access token; refresh before the
    // authenticated re-check below.
    let auth = ensure_fresh_token(&base_client, &auth).await?;
    let client = hydra_client(&auth)?;

    // Confirm the snapshot is still the latest before overwriting local saves.
    let current_snapshots = list_snapshots(&client, shop, object_id).await?;
    let current = current_snapshots.last();
    if current.map(|s| (s.id.as_str(), s.version)) != Some((latest.id.as_str(), latest.version)) {
        return Err(anyhow!(
            "Cloud save snapshot changed during restore; aborting to avoid stale data"
        ));
    }

    let wine_user_name = wine_prefix.and_then(|prefix| {
        get_windows_like_user_profile_path(prefix)
            .ok()
            .and_then(|profile| {
                profile
                    .replace('\\', "/")
                    .rsplit('/')
                    .next()
                    .map(|s| s.to_string())
            })
    });

    let variant_folders: HashMap<String, String> = manifest
        .variants
        .iter()
        .filter_map(|v| {
            v.concrete_folder_id
                .clone()
                .map(|folder| (v.variant_id.clone(), folder))
        })
        .collect();

    let executable_path = crate::hydra::get_game_executable_path(object_id, shop);
    let windows_compat = wine_prefix.is_some()
        && executable_path
            .as_deref()
            .is_some_and(|p| p.to_ascii_lowercase().ends_with(".exe"));

    let context = RestoreContext {
        wine_prefix: wine_prefix.map(|p| p.to_string()),
        wine_user_name,
        home_dir: dirs::home_dir(),
        install_dir: install_dir_from_executable(executable_path.as_deref()),
        steam_root: steam_root(executable_path.as_deref()),
        windows_compat,
        variant_folders,
    };

    // Only write paths ludusavi designated for this game (same guard as the
    // desktop launcher's blocked-rule-unavailable). Without the rules the
    // allowlist cannot be evaluated — abort rather than fail open.
    let rules = crate::rules::GameRules::load(object_id)?
        .ok_or_else(|| anyhow!("Save rules unavailable for this game; restore aborted"))?;

    let mut restored_files = 0usize;
    let mut skipped_files: Vec<String> = Vec::new();

    eprintln!(
        "restore: {} files in manifest v{}",
        manifest.files.len(),
        manifest.snapshot.version
    );

    for file in &manifest.files {
        let display = || format!("{}/{}", file.raw_path, file.relative_path);

        if !is_safe_manifest_file(file) {
            skipped_files.push(format!("{} (invalid manifest entry)", display()));
            continue;
        }

        if !rules.allows_raw_path(&file.raw_path) {
            skipped_files.push(format!("{} (path not in game rules)", display()));
            continue;
        }

        // File rules store the full path in rawPath; dir/glob rules store the
        // root and need relativePath appended after stripping glob segments.
        let effective_raw = crate::rules::join_restore_path(&file.raw_path, &file.relative_path);

        let Some(target) =
            context.resolve_with_variant(&file.variant_id, &effective_raw, "")
        else {
            skipped_files.push(format!("{} (no local target)", display()));
            continue;
        };

        let blob_path = temp.path().join(&file.hash);
        if !blob_path.exists() {
            skipped_files.push(format!("{} (blob missing)", display()));
            continue;
        }

        // Per-file failures must not abort the whole restore; collect them.
        let result: Result<()> = async {
            if let Some(parent) = target.parent() {
                tokio_fs::create_dir_all(parent).await?;
            }

            // Atomic replace: copy to a unique sibling temp file on the same
            // filesystem, then rename over the target. A blob can be
            // referenced by multiple manifest files, so the staged blob is
            // never moved. File name capped so the temp name stays under
            // NAME_MAX.
            let file_name = target
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "save".to_string());
            let file_name: String = file_name.chars().take(100).collect();
            let temp_target = target.with_file_name(format!(
                ".{file_name}.hydra-restore-{}",
                std::process::id()
            ));
            if let Err(err) = tokio_fs::copy(&blob_path, &temp_target).await {
                let _ = tokio_fs::remove_file(&temp_target).await;
                return Err(err).context("Failed to stage save file");
            }
            if let Err(err) = tokio_fs::rename(&temp_target, &target).await {
                let _ = tokio_fs::remove_file(&temp_target).await;
                return Err(err).context("Failed to replace save file");
            }

            if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(&file.last_modified_at) {
                let mtime = filetime::FileTime::from_unix_time(parsed.timestamp(), 0);
                let _ = filetime::set_file_mtime(&target, mtime);
            }

            Ok(())
        }
        .await;

        match result {
            Ok(()) => restored_files += 1,
            Err(err) => {
                eprintln!("Failed to restore {}: {err:#}", target.display());
                skipped_files.push(format!("{} (write failed)", display()));
            }
        }
    }

    if !skipped_files.is_empty() {
        eprintln!("restore skips: {}", skipped_files.join("; "));
    }

    // Only record the snapshot as current when every file landed. A partial
    // restore must keep the guard engaged: the local state does not match the
    // remote snapshot, and auto-sync would otherwise drop the skipped files
    // from the cloud on the next run.
    if skipped_files.is_empty() {
        write_state_logged(
            shop,
            object_id,
            &CloudSaveState {
                snapshot_id: manifest.snapshot.id.clone(),
                version: manifest.snapshot.version,
                aggregate_hash: latest.aggregate_hash.clone(),
                wine_prefix_path: wine_prefix.map(|p| p.to_string()),
                updated_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            },
        );
    }

    Ok(RestoreResult {
        ok: true,
        snapshot_id: manifest.snapshot.id,
        version: manifest.snapshot.version,
        restored_files,
        skipped_files,
        auth: Some(auth),
    })
}

// ---------------------------------------------------------------------------
// Remote status check (pre-launch guard)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudSaveStatus {
    pub ok: bool,
    pub remote_newer: bool,
    pub remote_version: Option<u64>,
    pub local_version: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth: Option<Auth>,
}

/// Compares the latest remote snapshot with the local sync state. The plugin
/// cannot block a Steam launch, so callers use this to suppress the post-exit
/// sync (protecting newer remote data) and prompt a manual restore instead.
///
/// Version bookkeeping alone produces false positives when the local state
/// file is missing (fresh install, lost state) even if this same device
/// created the remote snapshot — so a version mismatch triggers a content
/// comparison before reporting the remote as newer.
pub async fn check_cloud_save_status(
    auth_json: &str,
    object_id: &str,
    shop: &str,
    wine_prefix: Option<&str>,
) -> Result<CloudSaveStatus> {
    let auth: Auth = serde_json::from_str(auth_json).context("Invalid auth payload")?;
    let base_client = reqwest::Client::new();
    let auth = ensure_fresh_token(&base_client, &auth).await?;
    let client = hydra_client(&auth)?;

    let snapshots = list_snapshots(&client, shop, object_id).await?;
    let latest = snapshots.last();
    let state = read_state(shop, object_id);

    let version_says_newer = match (latest, &state) {
        (Some(remote), Some(local)) => {
            remote.version > local.version
                || (remote.version == local.version
                    && remote.aggregate_hash != local.aggregate_hash)
        }
        (Some(_), None) => true,
        (None, _) => false,
    };

    let mut remote_newer = version_says_newer;

    if version_says_newer {
        if let Some(remote) = latest {
            // Content check: if the local saves hash to the same aggregate as
            // the remote snapshot, they are identical regardless of version
            // bookkeeping.
            if let Ok(discovered) = discover_files(object_id, shop, wine_prefix).await {
                let entries: Vec<SnapshotFileEntry> =
                    discovered.files.iter().map(|f| f.entry.clone()).collect();
                if let Ok(local_hash) = build_aggregate_hash(&discovered.variants, &entries) {
                    if local_hash == remote.aggregate_hash {
                        remote_newer = false;

                        // Self-heal the bookkeeping so later checks skip the
                        // content comparison.
                        write_state_logged(
                            shop,
                            object_id,
                            &CloudSaveState {
                                snapshot_id: remote.id.clone(),
                                version: remote.version,
                                aggregate_hash: remote.aggregate_hash.clone(),
                                wine_prefix_path: wine_prefix.map(|p| p.to_string()),
                                updated_at: chrono::Utc::now()
                                    .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                            },
                        );
                    }
                }
            }
        }
    }

    Ok(CloudSaveStatus {
        ok: true,
        remote_newer,
        remote_version: latest.map(|s| s.version),
        local_version: state.map(|s| s.version),
        auth: Some(auth),
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_variant_id_matches_hydra_vector() {
        // hydra native identity test vector: steam:1817070 default variant.
        let variant = build_default_variant("steam", "1817070");
        assert_eq!(
            variant.variant_id,
            "6bb5b19456b48c65d5b6120154934d146013679fd8673e7d42694fff131774db"
        );
    }

    #[test]
    fn aggregate_hash_matches_hydra_sekiro_vector() {
        // hydra native hashing test vector (Sekiro fixture).
        let variant = SnapshotVariant {
            variant_id: build_opaque_variant_id_for_test("steam", "814380", "12345"),
            kind: "opaque-folder".to_string(),
            steam_id64: None,
            concrete_folder_id: Some("12345".to_string()),
        };

        let hash = build_aggregate_hash(
            std::slice::from_ref(&variant),
            &[SnapshotFileEntry {
                variant_id: variant.variant_id.clone(),
                raw_path: "<winAppData>/Sekiro/<storeUserId>/S0000.sl2".to_string(),
                relative_path: "S0000.sl2".to_string(),
                hash: "a".repeat(64),
                size_bytes: 4,
                last_modified_at: "2024-01-01T00:00:00.000Z".to_string(),
            }],
        )
        .unwrap();

        assert_eq!(
            hash,
            "c940e59b1eaa065e7c748a80aafde1328584a58ff5cca3d0810474ebecf5fa15"
        );
    }

    fn build_opaque_variant_id_for_test(shop: &str, object_id: &str, folder: &str) -> String {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct CanonicalVariant<'a> {
            variant_id_version: u32,
            shop: &'a str,
            object_id: &'a str,
            kind: &'a str,
            concrete_folder_id: &'a str,
        }

        let normalized = folder.nfc().collect::<String>().to_lowercase();
        let canonical = CanonicalVariant {
            variant_id_version: 1,
            shop,
            object_id,
            kind: "opaque-folder",
            concrete_folder_id: &normalized,
        };
        let serialized = serde_json::to_vec(&canonical).unwrap();
        format!("{:x}", Sha256::digest(serialized))
    }

    #[test]
    fn opaque_variant_id_matches_hydra_vector() {
        assert_eq!(
            build_opaque_variant_id_for_test("steam", "1817070", "76561197960271872"),
            "82e6580b982018f47d8ce8e17656a22675f2277d2cdd0a11ae501b10c8a430e1"
        );
    }

}
