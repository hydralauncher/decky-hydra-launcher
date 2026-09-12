use anyhow::{anyhow, Context, Result};

use serde::{Deserialize, Serialize};

use sha2::{Digest, Sha256};

use std::collections::HashMap;

use std::path::{Path, PathBuf};

use tokio::fs as tokio_fs;

use unicode_normalization::UnicodeNormalization;

use crate::wine::get_windows_like_user_profile_path;

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

const MAX_SNAPSHOT_BYTES: u64 = 2_147_483_647;

const MAX_STORE_USER_FOLDER_LEN: usize = 255;

const MAX_CONCURRENT_TRANSFERS: usize = 8;

const HTTP_CONNECT_TIMEOUT_SECS: u64 = 30;

const HTTP_TOTAL_TIMEOUT_SECS: u64 = 1800;

const TOKEN_REFRESH_GRACE_MS: f64 = 60_000.0;

const ERROR_BODY_PREVIEW_CHARS: usize = 512;

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

    refresh_token: Option<String>,

}

pub async fn ensure_fresh_token(client: &reqwest::Client, auth: &Auth) -> Result<Auth> {

    let now_ms = std::time::SystemTime::now()

        .duration_since(std::time::UNIX_EPOCH)?

        .as_millis() as f64;

    let expired = match auth.token_expiration_timestamp {

        Some(ts) => ts < now_ms + TOKEN_REFRESH_GRACE_MS,

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

#[derive(Debug, Clone, Serialize, Deserialize)]

#[serde(rename_all = "camelCase")]

pub struct StateEntry {

    pub variant_id: String,

    pub raw_path: String,

    pub relative_path: String,

    pub hash: String,

    pub size_bytes: u64,

}

#[derive(Debug, Clone, Serialize, Deserialize)]

#[serde(rename_all = "camelCase")]

pub struct CloudSaveState {

    pub snapshot_id: String,

    pub version: u64,

    pub aggregate_hash: String,

    pub wine_prefix_path: Option<String>,

    pub updated_at: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]

    pub entries: Option<Vec<StateEntry>>,

}

pub(crate) fn state_dir() -> Result<PathBuf> {

    Ok(dirs::config_dir()

        .ok_or_else(|| anyhow!("No config dir"))?

        .join("hydralauncher")

        .join("decky-cloud-saves"))

}

fn lexical_path_segments(path: &str) -> String {

    let mut out: Vec<&str> = Vec::new();

    let absolute = path.starts_with('/');

    for segment in path.split('/') {

        if segment.is_empty() || segment == "." {

            continue;

        }

        if segment == ".." {

            out.pop();

            continue;

        }

        out.push(segment);

    }

    let mut cleaned = out.join("/");

    if absolute {

        cleaned.insert(0, '/');

    }

    cleaned

}

fn prefix_key(wine_prefix: Option<&str>) -> String {

    let path = wine_prefix.map(|p| p.trim()).filter(|p| !p.is_empty());

    let Some(path) = path else {

        return "none".to_string();

    };

    let normalized = path.replace('\\', "/");

    let normalized = lexical_path_segments(&normalized);

    let normalized = normalized.trim_end_matches('/').to_string();

    let canonical = std::fs::canonicalize(&normalized)

        .map(|c| c.to_string_lossy().replace('\\', "/"))

        .unwrap_or(normalized);

    format!("{:x}", Sha256::digest(canonical.as_bytes()))[..16].to_string()

}

fn valid_state_segment(value: &str) -> bool {

    !value.is_empty()

        && value

            .chars()

            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')

}

fn state_path(shop: &str, object_id: &str, key: &str) -> Result<PathBuf> {

    if !valid_state_segment(shop) || !valid_state_segment(object_id) {

        return Err(anyhow!("Invalid game identity"));

    }

    Ok(state_dir()?.join(format!("{shop}-{object_id}@{key}.json")))

}

fn legacy_state_path(shop: &str, object_id: &str) -> Result<PathBuf> {

    if !valid_state_segment(shop) || !valid_state_segment(object_id) {

        return Err(anyhow!("Invalid game identity"));

    }

    Ok(state_dir()?.join(format!("{shop}-{object_id}.json")))

}

fn read_state_file(path: PathBuf) -> Option<CloudSaveState> {

    let content = std::fs::read_to_string(path).ok()?;

    serde_json::from_str(&content).ok()

}

fn has_keyed_state(shop: &str, object_id: &str) -> bool {

    let prefix = format!("{shop}-{object_id}@");

    let Ok(dir) = state_dir().and_then(|d| std::fs::read_dir(d).map_err(|e| anyhow!("{e}"))) else {

        return false;

    };

    dir.flatten().any(|entry| {

        let name = entry.file_name().to_string_lossy().to_string();

        name.starts_with(&prefix) && name.ends_with(".json")

    })

}

fn read_state(shop: &str, object_id: &str, key: &str) -> Option<CloudSaveState> {

    if let Ok(path) = state_path(shop, object_id, key) {

        if let Some(state) = read_state_file(path) {

            return Some(state);

        }

    }

    if has_keyed_state(shop, object_id) {

        return None;

    }

    legacy_state_path(shop, object_id).ok().and_then(read_state_file)

}

fn write_state(shop: &str, object_id: &str, key: &str, state: &CloudSaveState) -> Result<()> {

    let dir = state_dir()?;

    std::fs::create_dir_all(&dir)?;

    let path = state_path(shop, object_id, key)?;

    std::fs::write(path, serde_json::to_string_pretty(state)?)?;

    Ok(())

}

fn write_state_logged(shop: &str, object_id: &str, key: &str, state: &CloudSaveState) {

    match write_state(shop, object_id, key, state) {

        Ok(()) => {

            if let Ok(legacy) = legacy_state_path(shop, object_id) {

                let _ = std::fs::remove_file(legacy);

            }

        }

        Err(err) => {

            eprintln!("Failed to persist cloud save state: {err:#}");

        }

    }

}

fn persist_sync_anchor(
    shop: &str,
    object_id: &str,
    snapshot_id: &str,
    version: u64,
    aggregate_hash: &str,
    files: &[crate::hydra::AnchorWriteEntry],
    unresolved_nul_ids: &[String],
) {
    let Some(environment) = crate::environment::resolve_game_environment(shop, object_id) else {
        eprintln!("sync anchor skipped: environment unavailable for {object_id}");
        return;
    };
    if environment.mode != crate::environment::PrefixIdentityMode::Marker {
        eprintln!("sync anchor degraded: no prefix marker for {object_id}");
    }
    let Some(user_id) = crate::hydra::current_user_id_from_store() else {
        eprintln!("sync anchor skipped: no signed-in user");
        return;
    };
    let Some(record) = crate::hydra::build_sync_anchor_record(
        &environment.id,
        snapshot_id,
        version,
        aggregate_hash,
        files,
        unresolved_nul_ids,
        &crate::hydra::anchor_timestamp_now(),
    ) else {
        eprintln!("sync anchor skipped: invalid anchor payload for {object_id}");
        return;
    };
    if let Err(err) =
        crate::hydra::write_sync_anchor(shop, object_id, &user_id, &environment.id, &record)
    {
        eprintln!("sync anchor skipped for {object_id}: {err:#}");
    }
}

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

    #[serde(default)]

    custom_path_raw_paths: Vec<String>,

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

        && !value.contains(['\\', '\0'])

        && !value.starts_with('/')

        && value

            .split('/')

            .all(|segment| !segment.is_empty() && segment != "." && segment != "..")

}

fn is_safe_raw_path(value: &str) -> bool {

    !value.is_empty()

        && !value.contains(['\\', '\0'])

        && !value.split('/').any(|segment| segment == "..")

}

fn is_safe_manifest_file(file: &RestoreManifestFile) -> bool {

    is_valid_hash(&file.hash)

        && is_safe_raw_path(&file.raw_path)

        && is_safe_relative_path(&file.relative_path)

}

fn mtime_matches_known(actual: std::time::SystemTime, expected_rfc3339: &str) -> bool {

    let actual = chrono::DateTime::<chrono::Utc>::from(actual);

    let expected = chrono::DateTime::parse_from_rfc3339(expected_rfc3339)

        .ok()

        .map(|dt| dt.with_timezone(&chrono::Utc));

    match expected {

        Some(expected) => actual.timestamp_millis() == expected.timestamp_millis(),

        None => false,

    }

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

        .connect_timeout(std::time::Duration::from_secs(HTTP_CONNECT_TIMEOUT_SECS))

        .timeout(std::time::Duration::from_secs(HTTP_TOTAL_TIMEOUT_SECS))

        .build()?)

}

async fn send_checked(builder: reqwest::RequestBuilder) -> Result<reqwest::Response> {

    let response = builder.send().await?;

    let status = response.status();

    if !status.is_success() {

        let body = response.text().await.unwrap_or_default();

        let body: String = body.chars().take(ERROR_BODY_PREVIEW_CHARS).collect();

        return Err(anyhow!("Request failed with status {status}: {body}"));

    }

    Ok(response)

}

async fn fetch_restore_manifest(

    client: &reqwest::Client,

    snapshot_id: &str,

) -> Result<RestoreManifestResponse> {

    send_checked(

        client

            .get(format!(

                "{API_BASE}/profile/cloud-saves/snapshot-restore-manifest"

            ))

            .query(&[("snapshotId", snapshot_id)]),

    )

    .await

    .context("Failed to fetch restore manifest")?

    .json::<RestoreManifestResponse>()

    .await

    .context("Invalid restore manifest response")

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

struct DiscoveredFile {

    entry: SnapshotFileEntry,

    source_path: PathBuf,

}

pub struct DiscoveryOutput {

    files: Vec<DiscoveredFile>,

    variants: Vec<SnapshotVariant>,

    custom_raw_paths: Vec<String>,

    complete: bool,

}

async fn discover_files(

    object_id: &str,

    shop: &str,

    operative: Option<&str>,

) -> Result<DiscoveryOutput> {

    let ctx = crate::scanner::ScanContext::build_resolved(
        object_id,
        shop,
        operative.map(|p| p.to_string()),
    );

    let bindings = ctx.custom_paths.clone();

    let rules = match crate::rules::GameRules::load(object_id)? {

        Some(rules) => rules.with_custom_bindings(&bindings, ctx.windows_compat),

        None if !bindings.is_empty() => {

            crate::rules::GameRules::empty().with_custom_bindings(&bindings, ctx.windows_compat)

        }

        None => {

            return Err(anyhow!(

                "Save rules unavailable for this game; open Hydra launcher once, then retry"

            ))

        }

    };

    let candidates = crate::scanner::scan_game_saves(&ctx, &rules);

    let default_variant = build_default_variant(shop, object_id);

    let mut variants: Vec<SnapshotVariant> = vec![default_variant.clone()];

    let mut files = Vec::new();

    for (real_path, tokenized) in candidates {

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

    let mut custom_raw_paths: Vec<String> =

        bindings.iter().map(|(raw_path, _, _)| raw_path.clone()).collect();

    custom_raw_paths.sort();

    custom_raw_paths.dedup();

    eprintln!(

        "discovery: {} files, {} bytes, {} variants",

        files.len(),

        total,

        variants.len()

    );

    let complete = bindings

        .iter()

        .all(|(_, local_path, _)| Path::new(local_path).is_dir());

    Ok(DiscoveryOutput { files, variants, custom_raw_paths, complete })

}

async fn sha256_file_hex(path: &Path) -> Result<String> {

    let bytes = tokio_fs::read(path).await?;

    Ok(format!("{:x}", Sha256::digest(&bytes)))

}

#[derive(Debug, Serialize)]

#[serde(rename_all = "camelCase")]

pub struct SyncResult {

    pub ok: bool,

    #[serde(skip_serializing_if = "Option::is_none")]

    pub conflict: Option<Vec<String>>,

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

    resolutions: Option<HashMap<String, String>>,

) -> Result<SyncResult> {

    let auth: Auth = serde_json::from_str(auth_json).context("Invalid auth payload")?;

    let base_client = reqwest::Client::new();

    let mut auth = ensure_fresh_token(&base_client, &auth).await?;

    let mut client = hydra_client(&auth)?;

    let operative = crate::hydra::operative_prefix(object_id, shop, wine_prefix);

    let hostname = hostname::get()

        .map(|h| h.to_string_lossy().to_string())

        .ok()

        .filter(|h| !h.is_empty());

    let mut last_error: Option<anyhow::Error> = None;

    let mut result: Option<(CommitSnapshotResponse, usize, usize, Vec<StateEntry>)> = None;

    let mut pre_resolution: Vec<String> = Vec::new();

    for attempt in 0..2 {

        let discovered = match discover_files(object_id, shop, operative.as_deref()).await {

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

        let mut files: Vec<SnapshotFileEntry> = discovered.files.iter().map(|f| f.entry.clone()).collect();

        let mut variants: Vec<SnapshotVariant> = discovered.variants.clone();

        let mut custom_raw_paths = discovered.custom_raw_paths.clone();

        let aggregate_hash;

        let snapshots = list_snapshots(&client, shop, object_id).await?;

        if !force {

            if let Some(latest) = snapshots.last() {

                let mut state = read_state(shop, object_id, &prefix_key(operative.as_deref()));

                if let Some(local) = state.as_ref() {

                    if is_series_reset(latest.version, local.version) {

                        let local_version = local.version;

                        eprintln!(
                            "sync: remote v{} older than state v{}, rebasing state (series reset)",
                            latest.version, local_version
                        );

                        let rebased = CloudSaveState {

                            snapshot_id: latest.id.clone(),

                            version: latest.version,

                            aggregate_hash: latest.aggregate_hash.clone(),

                            wine_prefix_path: wine_prefix.map(|p| p.to_string()),

                            updated_at: chrono::Utc::now()

                                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),

                            entries: None,

                        };

                        write_state_logged(
                            shop,
                            object_id,
                            &prefix_key(operative.as_deref()),
                            &rebased,
                        );

                        state = Some(rebased);

                    }

                }

                let remote_newer = match &state {

                    Some(local) => {

                        latest.version > local.version

                            || (latest.version == local.version

                                && latest.aggregate_hash != local.aggregate_hash)

                    }

                    None => true,

                };

                let environment_id = crate::environment::resolve_game_environment(shop, object_id)
                    .map(|environment| environment.id);

                let anchor = crate::hydra::get_sync_anchor(
                    object_id,
                    shop,
                    environment_id.as_deref(),
                );

                let anchor_matches = anchor

                    .as_ref()

                    .is_some_and(|anchor| anchor.base_version == latest.version);

                if remote_newer && !anchor_matches {

                    if !discovered.complete {

                        return Err(anyhow!("remote-newer"));

                    }

                    let from_state = state

                        .as_ref()

                        .and_then(|s| s.entries.clone().map(|e| (s.version, e)));

                    let from_anchor = anchor

                        .as_ref()

                        .filter(|a| !a.entries.is_empty())

                        .map(|a| (a.base_version, a.entries.clone()));

                    let base_entries = match (from_state, from_anchor) {

                        (Some((sv, se)), Some((av, ae))) => {

                            if sv >= av { Some(se) } else { Some(ae) }

                        }

                        (Some((_, se)), None) => Some(se),

                        (None, Some((_, ae))) => Some(ae),

                        (None, None) => None,

                    };

                    let Some(base_entries) = base_entries.filter(|e| !e.is_empty()) else {

                        return Err(anyhow!("remote-newer"));

                    };

                    let base_exclude: std::collections::HashSet<String> = anchor

                        .map(|a| a.unresolved_entry_ids.into_iter().collect())

                        .unwrap_or_default();

                    let manifest = fetch_restore_manifest(&client, &latest.id).await?;

                    let remote_files: Vec<SnapshotFileEntry> = manifest

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

                    let mut outcome = crate::merge::merge_snapshots(

                        &files,

                        &remote_files,

                        Some(&base_entries),

                        &base_exclude,

                    )

                    .map_err(|e| anyhow!("merge failed: {e}"))?;

                    if let Some(resolutions) = &resolutions {

                        pre_resolution = outcome                            .conflicts
                            .iter()
                            .map(|c| c.identity.clone())
                            .collect();

                        let mut remaining = Vec::new();

                        for conflict in outcome.conflicts.drain(..) {

                            match resolutions.get(&conflict.identity).map(String::as_str) {

                                Some("local") => {

                                    if let Some(f) =

                                        files.iter().find(|f| conflict.id_of(f)).cloned()

                                    {

                                        outcome.files.push(f);

                                    }

                                }

                                Some("remote") => {

                                    if let Some(f) =

                                        remote_files.iter().find(|f| conflict.id_of(f)).cloned()

                                    {

                                        outcome.files.push(f);

                                    }

                                }

                                _ => remaining.push(conflict),

                            }

                        }

                        outcome.conflicts = remaining;

                    }

                    if !outcome.conflicts.is_empty() {

                        return Ok(SyncResult {

                            ok: false,

                            conflict: Some(

                                outcome

                                    .conflicts

                                    .iter()

                                    .map(|c| c.identity.clone())

                                    .collect(),

                            ),

                            snapshot_id: latest.id.clone(),

                            version: latest.version,

                            file_count: 0,

                            total_size_bytes: 0,

                            uploaded_files: 0,

                            skipped_files: 0,

                            auth: Some(auth),

                        });

                    }

                    eprintln!(

                        "sync: merged {} local + {} remote files ({} merged)",

                        files.len(),

                        remote_files.len(),

                        outcome.files.len()

                    );

                    for variant in &manifest.variants {

                        match variants.iter().find(|v| v.variant_id == variant.variant_id) {

                            Some(existing)

                                if existing.kind != variant.kind

                                    || existing.steam_id64 != variant.steam_id64

                                    || existing.concrete_folder_id != variant.concrete_folder_id =>

                            {

                                return Err(anyhow!(

                                    "Merged variant metadata diverges from remote"

                                ));

                            }

                            None => variants.push(variant.clone()),

                            _ => {}

                        }

                    }

                    for raw_path in &manifest.custom_path_raw_paths {

                        if !custom_raw_paths.contains(raw_path) {

                            custom_raw_paths.push(raw_path.clone());

                        }

                    }

                    custom_raw_paths.sort();

                    files = outcome.files;

                }

            }

        }

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

        aggregate_hash = build_aggregate_hash(&variants, &files)?;

        let base_version = snapshots.last().map(|s| s.version).unwrap_or(0);

        if let Some(remote) = snapshots.last() {

            let identity_equal = aggregate_hash == remote.aggregate_hash;

            let mut same = identity_equal;

            if !same {

                match fetch_restore_manifest(&client, &remote.id).await {

                    Ok(manifest) => {

                        let mut local_blobs: Vec<(&str, u64)> = files

                            .iter()

                            .map(|f| (f.hash.as_str(), f.size_bytes))

                            .collect();

                        let mut remote_blobs: Vec<(&str, u64)> = manifest

                            .files

                            .iter()

                            .map(|f| (f.hash.as_str(), f.size_bytes))

                            .collect();

                        local_blobs.sort_unstable();

                        remote_blobs.sort_unstable();

                        same = local_blobs == remote_blobs;

                    }

                    Err(err) => {

                        eprintln!("sync no-op check failed to fetch manifest: {err:#}");

                    }

                }

            }

            if same {

                eprintln!("sync: local content matches remote v{}, skipping", remote.version);

                let entries = if identity_equal {

                    Some(files.iter().map(|f| StateEntry {

                        variant_id: f.variant_id.clone(),

                        raw_path: f.raw_path.clone(),

                        relative_path: f.relative_path.clone(),

                        hash: f.hash.clone(),

                        size_bytes: f.size_bytes,

                    }).collect())

                } else {

                    read_state(shop, object_id, &prefix_key(operative.as_deref())).and_then(|s| s.entries)

                };

                write_state_logged(

                    shop,

                    object_id,

                    &prefix_key(operative.as_deref()),

                    &CloudSaveState {

                        snapshot_id: remote.id.clone(),

                        version: remote.version,

                        aggregate_hash: remote.aggregate_hash.clone(),

                        wine_prefix_path: wine_prefix.map(|p| p.to_string()),

                        updated_at: chrono::Utc::now()

                            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),

                        entries,

                    },

                );

                return Ok(SyncResult {

                    ok: true,

                    conflict: None,

                    snapshot_id: remote.id.clone(),

                    version: remote.version,

                    file_count: files.len() as u64,

                    total_size_bytes: total_size,

                    uploaded_files: 0,

                    skipped_files: files.len(),

                    auth: Some(auth),

                });

            }

        }

        match prepare_upload_commit(

            &client,

            shop,

            object_id,

            hostname.as_deref(),

            base_version,

            &variants,

            &files,

            &discovered,

            &custom_raw_paths,

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

                let entries = files

                    .iter()

                    .map(|f| StateEntry {

                        variant_id: f.variant_id.clone(),

                        raw_path: f.raw_path.clone(),

                        relative_path: f.relative_path.clone(),

                        hash: f.hash.clone(),

                        size_bytes: f.size_bytes,

                    })

                    .collect();

                persist_sync_anchor(
                    shop,
                    object_id,
                    &committed.snapshot_id,
                    committed.version,
                    &committed.aggregate_hash,
                    &files
                        .iter()
                        .map(|f| crate::hydra::AnchorWriteEntry {
                            variant_id: f.variant_id.clone(),
                            raw_path: f.raw_path.clone(),
                            relative_path: f.relative_path.clone(),
                            hash: f.hash.clone(),
                            size_bytes: f.size_bytes,
                        })
                        .collect::<Vec<_>>(),
                    &pre_resolution,
                );

                result = Some((committed, uploaded_files, skipped_files, entries));

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

    let (committed, uploaded_files, skipped_files, state_entries) =

        result.ok_or_else(|| last_error.unwrap_or_else(|| anyhow!("Commit did not complete")))?;

    write_state_logged(

        shop,

        object_id,

        &prefix_key(operative.as_deref()),

        &CloudSaveState {

            snapshot_id: committed.snapshot_id.clone(),

            version: committed.version,

            aggregate_hash: committed.aggregate_hash.clone(),

            wine_prefix_path: wine_prefix.map(|p| p.to_string()),

            updated_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),

            entries: Some(state_entries),

        },

    );

    Ok(SyncResult {

        ok: true,

        conflict: None,

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

    discovered: &DiscoveryOutput,

    custom_raw_paths: &[String],

    aggregate_hash: &str,

) -> Result<(CommitSnapshotResponse, usize, usize)> {

    let mut payload = serde_json::json!({

        "shop": shop,

        "objectId": object_id,

        "platform": "linux",

        "snapshotHash": aggregate_hash,

        "baseVersion": base_version,

        "customPathRawPaths": custom_raw_paths,

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

        .files

        .iter()

        .map(|f| {

            (

                format!("{}\u{0}{}\u{0}{}", f.entry.variant_id, f.entry.raw_path, f.entry.relative_path),

                f,

            )

        })

        .collect();

    let source_by_blob: HashMap<String, &DiscoveredFile> = discovered

        .files

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

        if !required_headers.contains_key("Content-Length")

            || !required_headers.contains_key("x-amz-checksum-sha256")

        {

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

            .ok_or_else(|| anyhow!(

                "Remote file content missing from cloud storage; restore the cloud save first, then sync again"

            ))?;

        let blob_key = format!("{}\u{0}{}", proposal.hash, proposal.size_bytes);

        upload_jobs

            .entry(blob_key)

            .or_insert_with(|| {

                (

                    upload_url,

                    expected_checksum,

                    source.source_path.clone(),

                    proposal.size_bytes as usize,

                    source.entry.last_modified_at.clone(),

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

    let upload_client = reqwest::Client::builder()

        .connect_timeout(std::time::Duration::from_secs(HTTP_CONNECT_TIMEOUT_SECS))

        .timeout(std::time::Duration::from_secs(HTTP_TOTAL_TIMEOUT_SECS))

        .build()?;

    for (url, checksum, path, size, expected_modified) in jobs {

        let metadata = tokio_fs::metadata(&path).await.map_err(|_| {

            anyhow!("Save file changed during sync; aborting before commit")

        })?;

        if metadata.len() != size as u64 {

            return Err(anyhow!(

                "Save file changed during sync; aborting before commit"

            ));

        }

        let modified_known = metadata.modified().ok();

        if let Some(actual) = modified_known {

            if !mtime_matches_known(actual, &expected_modified) {

                return Err(anyhow!(

                    "Save file changed during sync; aborting before commit"

                ));

            }

        }

        let body = tokio_fs::read(&path).await.map_err(|_| {

            anyhow!("Save file changed during sync; aborting before commit")

        })?;

        let resp = upload_client

            .put(&url)

            .header("Content-Length", size.to_string())

            .header("x-amz-checksum-sha256", checksum)

            .body(body)

            .send()

            .await?;

        let status = resp.status();

        if !status.is_success() {

            return Err(anyhow!("Blob upload failed with status {status}"));

        }

    }

    for source in &discovered.files {

        let actual_hash = sha256_file_hex(&source.source_path).await.map_err(|_| {

            anyhow!("Save file changed during sync; aborting before commit")

        })?;

        if actual_hash != source.entry.hash {

            return Err(anyhow!(

                "Save file changed during sync; aborting before commit"

            ));

        }

    }

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

                    let body: String = body.chars().take(ERROR_BODY_PREVIEW_CHARS).collect();

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

    custom_paths: Vec<(String, String, Option<String>)>,

}

impl RestoreContext {

    fn resolve(&self, raw_path: &str, relative_path: &str) -> Option<PathBuf> {

        let raw_path = raw_path.replace('\\', "/");

        let profile_root = self.wine_user_profile_root();

        if raw_path.starts_with("<custom>") {

            for (bound_raw, local_path, store_user_id) in &self.custom_paths {

                let bound_raw = match store_user_id {

                    Some(id) => bound_raw.replace("<storeUserId>", id),

                    None => bound_raw.clone(),

                };

                let local_path = match store_user_id {

                    Some(id) => local_path.replace("<storeUserId>", id),

                    None => local_path.clone(),

                };

                if raw_path == bound_raw {

                    return Some(PathBuf::from(local_path));

                }

                if let Some(rest) = raw_path.strip_prefix(&format!("{bound_raw}/")) {

                    return Some(PathBuf::from(format!("{local_path}/{rest}")));

                }

            }

            return None;

        }

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

            None

        } else if raw_path.starts_with("C:/") || raw_path.starts_with("c:/") {

            self.resolve_windows_path(&raw_path)

        } else if raw_path.starts_with('/') {

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

            let safe = !concrete.is_empty()

                && concrete.len() <= MAX_STORE_USER_FOLDER_LEN

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

    let operative = crate::hydra::operative_prefix(object_id, shop, wine_prefix);

    let snapshots = list_snapshots(&client, shop, object_id).await?;

    let latest = snapshots

        .last()

        .ok_or_else(|| anyhow!("No cloud save snapshot exists for this game"))?;

    let manifest = fetch_restore_manifest(&client, &latest.id).await?;

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

    let wine_user_name = operative.as_deref().and_then(|prefix| {

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

    let windows_compat = operative.is_some()

        && executable_path

            .as_deref()

            .is_some_and(|p| p.to_ascii_lowercase().ends_with(".exe"));

    let context = RestoreContext {

        wine_prefix: operative.clone(),

        wine_user_name,

        home_dir: dirs::home_dir(),

        install_dir: install_dir_from_executable(executable_path.as_deref()),

        steam_root: steam_root(executable_path.as_deref()),

        windows_compat,

        variant_folders,

        custom_paths: crate::hydra::get_custom_paths(object_id, shop),

    };

    let bindings = context.custom_paths.clone();

    let rules = match crate::rules::GameRules::load(object_id)? {

        Some(rules) => rules.with_custom_bindings(&bindings, windows_compat),

        None if !bindings.is_empty() => {

            crate::rules::GameRules::empty().with_custom_bindings(&bindings, windows_compat)

        }

        None => {

            return Err(anyhow!("Save rules unavailable for this game; restore aborted"))

        }

    };

    let mut current_files: std::collections::HashSet<usize> = std::collections::HashSet::new();

    let mut needed_hashes: std::collections::HashSet<&str> = std::collections::HashSet::new();

    for (index, file) in manifest.files.iter().enumerate() {

        if !is_safe_manifest_file(file) || !rules.allows_raw_path(&file.raw_path) {

            continue;

        }

        let effective_raw = crate::rules::join_restore_path(&file.raw_path, &file.relative_path);

        let target = context.resolve_with_variant(&file.variant_id, &effective_raw, "");

        let already_current = match target {

            Some(target) if target.is_file() => {

                matches!(sha256_file_hex(&target).await, Ok(hash) if hash == file.hash)

            }

            _ => false,

        };

        if already_current {

            current_files.insert(index);

        } else {

            needed_hashes.insert(&file.hash);

        }

    }

    eprintln!(

        "restore: {} of {} files already current locally",

        current_files.len(),

        manifest.files.len()

    );

    let temp = match state_dir() {

        Ok(dir) if std::fs::create_dir_all(&dir).is_ok() => {

            tempfile::tempdir_in(dir)

        }

        _ => tempfile::tempdir(),

    }
    .context("Failed to create temp dir")?;

    let manifest_hashes: std::collections::HashSet<&str> =

        manifest.files.iter().map(|f| f.hash.as_str()).collect();

    let mut blob_urls: HashMap<String, (String, u64)> = HashMap::new();

    for file in &download_files {

        if !is_valid_hash(&file.hash)

            || !manifest_hashes.contains(file.hash.as_str())

            || !needed_hashes.contains(file.hash.as_str())

        {

            continue;

        }

        if !file.download_url.starts_with("https://") {

            return Err(anyhow!("Refusing non-HTTPS download URL"));

        }

        blob_urls

            .entry(file.hash.clone())

            .or_insert_with(|| (file.download_url.clone(), file.size_bytes));

    }

    let declared_bytes: u64 = blob_urls.values().map(|(_, size)| *size).sum();

    if declared_bytes > MAX_SNAPSHOT_BYTES {

        return Err(anyhow!("Cloud snapshot exceeds 2 GiB limit"));

    }

    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_TRANSFERS));

    let mut join_set = tokio::task::JoinSet::new();

    let download_client = reqwest::Client::builder()

        .connect_timeout(std::time::Duration::from_secs(HTTP_CONNECT_TIMEOUT_SECS))

        .timeout(std::time::Duration::from_secs(HTTP_TOTAL_TIMEOUT_SECS))

        .build()?;

    for (hash, (url, size)) in &blob_urls {

        let permit = semaphore.clone().acquire_owned().await?;

        let client = download_client.clone();

        let dest = temp.path().join(hash);

        let url = url.clone();

        let hash = hash.clone();

        let declared_size = *size;

        join_set.spawn(async move {

            let _permit = permit;

            let mut response = client.get(&url).send().await?.error_for_status()?;

            let dest_file = tokio_fs::File::create(&dest).await?;

            let mut writer = tokio::io::BufWriter::new(dest_file);

            let mut hasher = Sha256::new();

            let mut written: u64 = 0;

            loop {

                match response.chunk().await? {

                    Some(bytes) => {

                        written = written.saturating_add(bytes.len() as u64);

                        if written > declared_size {

                            drop(writer);

                            let _ = tokio_fs::remove_file(&dest).await;

                            return Err(anyhow!("Downloaded blob exceeds declared size"));

                        }

                        hasher.update(&bytes);

                        tokio::io::AsyncWriteExt::write_all(&mut writer, &bytes).await?;

                    }

                    None => break,

                }

            }

            tokio::io::AsyncWriteExt::shutdown(&mut writer).await?;

            let actual = format!("{:x}", hasher.finalize());

            if actual != hash {

                let _ = tokio_fs::remove_file(&dest).await;

                return Err(anyhow!("Downloaded blob failed hash verification"));

            }

            Ok::<(), anyhow::Error>(())

        });

    }

    while let Some(result) = join_set.join_next().await {

        result.context("Download task panicked")??;

    }

    let auth = ensure_fresh_token(&base_client, &auth).await?;

    let client = hydra_client(&auth)?;

    let current_snapshots = list_snapshots(&client, shop, object_id).await?;

    let current = current_snapshots.last();

    if current.map(|s| (s.id.as_str(), s.version)) != Some((latest.id.as_str(), latest.version)) {

        return Err(anyhow!(

            "Cloud save snapshot changed during restore; aborting to avoid stale data"

        ));

    }

    let mut restored_files = 0usize;

    let mut written_keys: std::collections::HashSet<String> = std::collections::HashSet::new();

    let mut skipped_files: Vec<String> = Vec::new();

    eprintln!(

        "restore: {} files in manifest v{}",

        manifest.files.len(),

        manifest.snapshot.version

    );

    for (index, file) in manifest.files.iter().enumerate() {

        if current_files.contains(&index) {

            restored_files += 1;

            written_keys.insert(crate::merge::identity_key(

                &file.variant_id,

                &file.raw_path,

                &file.relative_path,

            ));

            continue;

        }

        let display = || format!("{}/{}", file.raw_path, file.relative_path);

        if !is_safe_manifest_file(file) {

            skipped_files.push(format!("{} (invalid manifest entry)", display()));

            continue;

        }

        if !rules.allows_raw_path(&file.raw_path) {

            skipped_files.push(format!("{} (path not in game rules)", display()));

            continue;

        }

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

        let result: Result<()> = async {

            if let Some(parent) = target.parent() {

                tokio_fs::create_dir_all(parent).await?;

            }

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

            Ok(()) => {

                restored_files += 1;

                written_keys.insert(crate::merge::identity_key(

                    &file.variant_id,

                    &file.raw_path,

                    &file.relative_path,

                ));

            }

            Err(err) => {

                eprintln!("Failed to restore {}: {err:#}", target.display());

                skipped_files.push(format!("{} (write failed)", display()));

            }

        }

    }

    if !skipped_files.is_empty() {

        eprintln!("restore skips: {}", skipped_files.join("; "));

    }

    if skipped_files.is_empty() {

        write_state_logged(

            shop,

            object_id,

            &prefix_key(operative.as_deref()),

            &CloudSaveState {

                snapshot_id: manifest.snapshot.id.clone(),

                version: manifest.snapshot.version,

                aggregate_hash: latest.aggregate_hash.clone(),

                wine_prefix_path: wine_prefix.map(|p| p.to_string()),

                updated_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),

                entries: Some(

                    manifest

                        .files

                        .iter()

                        .map(|f| StateEntry {

                            variant_id: f.variant_id.clone(),

                            raw_path: f.raw_path.clone(),

                            relative_path: f.relative_path.clone(),

                            hash: f.hash.clone(),

                            size_bytes: f.size_bytes,

                        })

                        .collect(),

                ),

            },

        );

    }

    persist_sync_anchor(
        shop,
        object_id,
        &manifest.snapshot.id,
        manifest.snapshot.version,
        &latest.aggregate_hash,
        &manifest
            .files
            .iter()
            .map(|f| crate::hydra::AnchorWriteEntry {
                variant_id: f.variant_id.clone(),
                raw_path: f.raw_path.clone(),
                relative_path: f.relative_path.clone(),
                hash: f.hash.clone(),
                size_bytes: f.size_bytes,
            })
            .collect::<Vec<_>>(),
        &manifest
            .files
            .iter()
            .filter(|f| {
                !written_keys.contains(
                    crate::merge::identity_key(&f.variant_id, &f.raw_path, &f.relative_path).as_str(),
                )
            })
            .map(|f| {
                crate::merge::identity_key(&f.variant_id, &f.raw_path, &f.relative_path)
            })
            .collect::<Vec<_>>(),
    );

    Ok(RestoreResult {

        ok: true,

        snapshot_id: manifest.snapshot.id,

        version: manifest.snapshot.version,

        restored_files,

        skipped_files,

        auth: Some(auth),

    })

}

#[derive(Debug, Serialize)]

#[serde(rename_all = "camelCase")]

pub struct CloudSaveStatus {

    pub ok: bool,

    pub remote_newer: bool,

    pub local_dirty: bool,

    pub remote_version: Option<u64>,

    pub local_version: Option<u64>,

    #[serde(skip_serializing_if = "Option::is_none")]

    pub remote_file_count: Option<u64>,

    #[serde(skip_serializing_if = "Option::is_none")]

    pub remote_total_bytes: Option<u64>,

    #[serde(skip_serializing_if = "Option::is_none")]

    pub remote_updated_at: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]

    pub local_file_count: Option<u64>,

    #[serde(skip_serializing_if = "Option::is_none")]

    pub local_total_bytes: Option<u64>,

    #[serde(skip_serializing_if = "Option::is_none")]

    pub local_updated_at: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]

    pub auth: Option<Auth>,

}

const ANCHOR_OVERLAP_MIN: f64 = 0.5;

fn blob_multiset_eq(a: &[(String, u64)], b: &[(String, u64)]) -> bool {

    let mut x = a.to_vec();

    let mut y = b.to_vec();

    x.sort_unstable();

    y.sort_unstable();

    x == y

}

fn overlap_score(
    local: &[(String, String, String, String, u64)],
    base: &[StateEntry],
    excluded: &std::collections::HashSet<String>,
) -> f64 {

    use std::collections::{HashMap, HashSet};

    let base_map: HashMap<(String, String, String), (String, u64)> = base
        .iter()
        .filter(|e| {
            !excluded.contains(&format!(
                "{}\u{0}{}\u{0}{}",
                e.variant_id, e.raw_path, e.relative_path
            ))
        })
        .map(|e| {
            (
                (
                    e.variant_id.clone(),
                    e.raw_path.clone(),
                    e.relative_path.clone(),
                ),
                (e.hash.clone(), e.size_bytes),
            )
        })
        .collect();

    let mut union: HashSet<(String, String, String)> =
        base_map.keys().cloned().collect();

    let mut shared = 0usize;

    for (variant_id, raw_path, relative_path, hash, size_bytes) in local {

        let id = (variant_id.clone(), raw_path.clone(), relative_path.clone());

        union.insert(id.clone());

        if base_map
            .get(&id)
            .is_some_and(|(base_hash, base_size)| {
                base_hash == hash && *base_size == *size_bytes
            })
        {

            shared += 1;

        }

    }

    if union.is_empty() {

        return 0.0;

    }

    shared as f64 / union.len() as f64

}

fn anchor_clears(lineage: usize, base_matches_remote: bool, overlap: f64) -> bool {

    if !base_matches_remote {

        return false;

    }

    if lineage <= 1 {

        return true;

    }

    overlap >= ANCHOR_OVERLAP_MIN

}

fn restore_safe(
    local: &[(String, String, String, String, u64)],
    remote: &[(String, String, String, String, u64)],
) -> bool {

    use std::collections::{HashMap, HashSet};

    let remote_map: HashMap<(String, String, String), (String, u64)> = remote
        .iter()
        .map(
            |(variant_id, raw_path, relative_path, hash, size_bytes)| {
                (
                    (
                        variant_id.clone(),
                        raw_path.clone(),
                        relative_path.clone(),
                    ),
                    (hash.clone(), *size_bytes),
                )
            },
        )
        .collect();

    let mut seen: HashSet<(String, String, String)> = HashSet::new();

    for (variant_id, raw_path, relative_path, hash, size_bytes) in local {

        let id = (variant_id.clone(), raw_path.clone(), relative_path.clone());

        if !seen.insert(id.clone()) {

            continue;

        }

        match remote_map.get(&id) {

            Some((remote_hash, remote_size)) => {

                if remote_hash != hash || *remote_size != *size_bytes {

                    return false;

                }

            }

            None => return false,

        }

    }

    true

}

fn is_series_reset(remote_version: u64, local_version: u64) -> bool {

    remote_version < local_version

}

fn status_local_dirty(remote_newer: bool, fast_forward: bool, dirty_assessed: Option<bool>) -> bool {

    if !remote_newer {

        return false;

    }

    if fast_forward {

        return false;

    }

    dirty_assessed.unwrap_or(true)

}

fn local_snapshot_summary(files: &[(u64, &str)]) -> (u64, u64, Option<String>) {

    let updated_at = files
        .iter()
        .filter_map(|(_, modified_at)| {
            chrono::DateTime::parse_from_rfc3339(modified_at)
                .map(|dt| dt.with_timezone(&chrono::Utc))
                .ok()
        })
        .max()
        .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true));

    (
        files.len() as u64,
        files.iter().map(|(size, _)| size).sum(),
        updated_at,
    )

}

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

    let operative = crate::hydra::operative_prefix(object_id, shop, wine_prefix);

    let snapshots = list_snapshots(&client, shop, object_id).await?;

    let latest = snapshots.last();

    let mut state = read_state(shop, object_id, &prefix_key(operative.as_deref()));

    if let (Some(remote), Some(local)) = (latest, state.as_ref()) {

        if is_series_reset(remote.version, local.version) {

            let local_version = local.version;

            eprintln!(
                "status: remote v{} older than state v{}, rebasing state (series reset)",
                remote.version, local_version
            );

            let rebased = CloudSaveState {

                snapshot_id: remote.id.clone(),

                version: remote.version,

                aggregate_hash: remote.aggregate_hash.clone(),

                wine_prefix_path: wine_prefix.map(|p| p.to_string()),

                updated_at: chrono::Utc::now()

                    .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),

                entries: None,

            };

            write_state_logged(shop, object_id, &prefix_key(operative.as_deref()), &rebased);

            state = Some(rebased);

        }

    }

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

    let mut discovery: Option<DiscoveryOutput> = None;

    let mut dirty_assessed: Option<bool> = None;

    let mut fast_forward = false;

    if version_says_newer {

        discovery = discover_files(object_id, shop, operative.as_deref()).await.ok();

        let untouched = match (
            &discovery,
            state.as_ref().and_then(|s| s.entries.clone()),
        ) {

            (Some(discovered), Some(base)) if !base.is_empty() => {

                let local: Vec<(String, u64)> = discovered
                    .files
                    .iter()
                    .map(|f| (f.entry.hash.clone(), f.entry.size_bytes))
                    .collect();

                let base_blobs: Vec<(String, u64)> = base
                    .iter()
                    .map(|e| (e.hash.clone(), e.size_bytes))
                    .collect();

                blob_multiset_eq(&local, &base_blobs)

            }

            _ => false,

        };

        if untouched {

            fast_forward = true;

            eprintln!(
                "status: local untouched since state v{}, remote newer, fast-forward safe",
                state.as_ref().map(|s| s.version).unwrap_or(0)
            );

        } else if let Some(remote) = latest {

            let environment_id = crate::environment::resolve_game_environment(shop, object_id)
                .map(|environment| environment.id);

            let anchors = crate::hydra::list_sync_anchors(
                object_id,
                shop,
                environment_id.as_deref(),
            );

            let picked = anchors
                .iter()
                .max_by(|a, b| a.updated_at.cmp(&b.updated_at));

            let attribute = match picked {

                Some(record) if record.anchor.base_version == remote.version => {

                    let overlap = match &discovery {

                        Some(discovered) => {

                            let excluded: std::collections::HashSet<String> = record
                                .anchor
                                .unresolved_entry_ids
                                .iter()
                                .cloned()
                                .collect();

                            let local: Vec<(String, String, String, String, u64)> =
                                discovered
                                    .files
                                    .iter()
                                    .map(|f| {
                                        (
                                            f.entry.variant_id.clone(),
                                            f.entry.raw_path.clone(),
                                            f.entry.relative_path.clone(),
                                            f.entry.hash.clone(),
                                            f.entry.size_bytes,
                                        )
                                    })
                                    .collect();

                            overlap_score(&local, &record.anchor.entries, &excluded)

                        }

                        None => 0.0,

                    };

                    anchor_clears(anchors.len(), true, overlap)

                }

                _ => false,

            };

            if attribute {

                remote_newer = false;

                eprintln!(
                    "status: anchor attributes remote v{} (lineage {})",
                    remote.version,
                    anchors.len()
                );

                write_state_logged(

                    shop,

                    object_id,

                    &prefix_key(operative.as_deref()),

                    &CloudSaveState {

                        snapshot_id: remote.id.clone(),

                        version: remote.version,

                        aggregate_hash: remote.aggregate_hash.clone(),

                        wine_prefix_path: wine_prefix.map(|p| p.to_string()),

                        updated_at: chrono::Utc::now()

                            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),

                        entries: state.as_ref().and_then(|s| s.entries.clone()),

                    },

                );

            } else if picked
                .is_some_and(|record| record.anchor.base_version == remote.version)
            {

                eprintln!(
                    "status: anchor overruled for remote v{} (lineage {})",
                    remote.version,
                    anchors.len()
                );

            }

        }

    }

    if remote_newer {

        if let Some(remote) = latest {

            if discovery.is_none() {

                discovery = discover_files(object_id, shop, operative.as_deref()).await.ok();

            }

            if let Some(discovered) = &discovery {

                let entries: Vec<SnapshotFileEntry> =

                    discovered.files.iter().map(|f| f.entry.clone()).collect();

                let mut same = false;

                if let Ok(local_hash) = build_aggregate_hash(&discovered.variants, &entries) {

                    same = local_hash == remote.aggregate_hash;

                }

                if !same {

                    match fetch_restore_manifest(&client, &remote.id).await {

                        Ok(manifest) => {

                            let mut local_blobs: Vec<(&str, u64)> = entries

                                .iter()

                                .map(|f| (f.hash.as_str(), f.size_bytes))

                                .collect();

                            let mut remote_blobs: Vec<(&str, u64)> = manifest

                                .files

                                .iter()

                                .map(|f| (f.hash.as_str(), f.size_bytes))

                                .collect();

                            local_blobs.sort_unstable();

                            remote_blobs.sort_unstable();

                            if local_blobs == remote_blobs {

                                if let Ok(local_hash) =

                                    build_aggregate_hash(&discovered.variants, &entries)

                                {

                                    if local_hash != remote.aggregate_hash {

                                        eprintln!(

                                            "status: content equal but identities differ for {object_id}"

                                        );

                                    }

                                }

                                same = true;

                            } else {

                                let remote_ids: std::collections::HashSet<String> = manifest

                                    .files

                                    .iter()

                                    .map(|f| {

                                        format!("{}/{}:{}", f.raw_path, f.relative_path, f.hash)

                                    })

                                    .collect();

                                let local_ids: std::collections::HashSet<String> = entries

                                    .iter()

                                    .map(|f| {

                                        format!("{}/{}:{}", f.raw_path, f.relative_path, f.hash)

                                    })

                                    .collect();

                                let local_only: Vec<&String> = local_ids
                                    .difference(&remote_ids)
                                    .collect();

                                let remote_only: Vec<&String> = remote_ids
                                    .difference(&local_ids)
                                    .collect();

                                eprintln!(
                                    "status mismatch: {} local-only, {} remote-only",
                                    local_only.len(),
                                    remote_only.len()
                                );

                                for id in local_only.iter().take(2) {

                                    eprintln!("status mismatch, local only: {id}");

                                }

                                for id in remote_only.iter().take(2) {

                                    eprintln!("status mismatch, remote only: {id}");

                                }

                                let remote_entries: Vec<(String, String, String, String, u64)> = manifest
                                    .files
                                    .iter()
                                    .map(|f| {
                                        (
                                            f.variant_id.clone(),
                                            f.raw_path.clone(),
                                            f.relative_path.clone(),
                                            f.hash.clone(),
                                            f.size_bytes,
                                        )
                                    })
                                    .collect();

                                let local_entries: Vec<(String, String, String, String, u64)> = entries
                                    .iter()
                                    .map(|f| {
                                        (
                                            f.variant_id.clone(),
                                            f.raw_path.clone(),
                                            f.relative_path.clone(),
                                            f.hash.clone(),
                                            f.size_bytes,
                                        )
                                    })
                                    .collect();

                                let safe = restore_safe(&local_entries, &remote_entries);

                                dirty_assessed = Some(!safe);

                                if !safe && fast_forward {

                                    eprintln!("status: local differs from remote, fast-forward safe");

                                }

                                if !safe && !fast_forward {

                                    eprintln!("status: restore unsafe, local dirty");

                                }

                            }

                        }

                        Err(err) => {

                            eprintln!("status content check failed to fetch manifest: {err:#}");

                            dirty_assessed = Some(true);

                        }

                    }

                }

                if same {

                    remote_newer = false;

                    let healed_entries = if let Ok(local_hash) =

                        build_aggregate_hash(&discovered.variants, &entries)

                    {

                        if local_hash == remote.aggregate_hash {

                            Some(

                                entries

                                    .iter()

                                    .map(|f| StateEntry {

                                        variant_id: f.variant_id.clone(),

                                        raw_path: f.raw_path.clone(),

                                        relative_path: f.relative_path.clone(),

                                        hash: f.hash.clone(),

                                        size_bytes: f.size_bytes,

                                    })

                                    .collect(),

                            )

                        } else {

                            None

                        }

                    } else {

                        None

                    };

                write_state_logged(

                    shop,

                    object_id,

                    &prefix_key(operative.as_deref()),

                    &CloudSaveState {

                        snapshot_id: remote.id.clone(),

                        version: remote.version,

                        aggregate_hash: remote.aggregate_hash.clone(),

                        wine_prefix_path: wine_prefix.map(|p| p.to_string()),

                        updated_at: chrono::Utc::now()

                            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),

                        entries: healed_entries,

                        },

                    );

                }

            }

        }

    }

    let local_dirty = status_local_dirty(remote_newer, fast_forward, dirty_assessed);

    let (local_file_count, local_total_bytes, local_updated_at) = match &discovery {

        Some(discovered) => {

            let entries: Vec<(u64, &str)> = discovered
                .files
                .iter()
                .map(|f| (f.entry.size_bytes, f.entry.last_modified_at.as_str()))
                .collect();

            let (count, bytes, updated_at) = local_snapshot_summary(&entries);

            (Some(count), Some(bytes), updated_at)

        }

        None => (None, None, None),

    };

    Ok(CloudSaveStatus {

        ok: true,

        remote_newer,

        local_dirty,

        remote_version: latest.map(|s| s.version),

        local_version: state.map(|s| s.version),

        remote_file_count: latest.map(|s| s.file_count),

        remote_total_bytes: latest.map(|s| s.total_size_bytes),

        remote_updated_at: latest.map(|s| s.updated_at.clone()),

        local_file_count,

        local_total_bytes,

        local_updated_at,

        auth: Some(auth),

    })

}

#[cfg(test)]

mod tests {

    use super::*;

    fn test_state_entry(
        variant_id: &str,
        raw_path: &str,
        relative_path: &str,
        hash: &str,
        size_bytes: u64,
    ) -> StateEntry {

        StateEntry {

            variant_id: variant_id.to_string(),

            raw_path: raw_path.to_string(),

            relative_path: relative_path.to_string(),

            hash: hash.to_string(),

            size_bytes,

        }

    }

    #[test]

    fn mtime_matches_known_tolerates_submillis_precision() {

        let actual = std::time::SystemTime::UNIX_EPOCH
            + std::time::Duration::new(1_700_000_000, 123_456_789);

        let expected = chrono::DateTime::<chrono::Utc>::from(actual)
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

        assert!(mtime_matches_known(actual, &expected));

        let different = std::time::SystemTime::UNIX_EPOCH
            + std::time::Duration::new(1_700_000_005, 0);

        assert!(!mtime_matches_known(different, &expected));

        assert!(!mtime_matches_known(actual, "not-a-timestamp"));

    }

    #[test]

    fn local_snapshot_summary_picks_max_mtime() {

        let (count, bytes, updated) = local_snapshot_summary(&[
            (10u64, "2026-08-19T22:49:54.000Z"),
            (20u64, "2026-09-10T17:42:46.670Z"),
            (30u64, "not-a-timestamp"),
        ]);

        assert_eq!(count, 3);

        assert_eq!(bytes, 60);

        assert_eq!(updated.as_deref(), Some("2026-09-10T17:42:46.670Z"));

    }

    #[test]

    fn local_snapshot_summary_empty_has_no_timestamp() {

        let (count, bytes, updated) = local_snapshot_summary(&[]);

        assert_eq!(count, 0);

        assert_eq!(bytes, 0);

        assert_eq!(updated, None);

    }

    #[test]

    fn status_verdict_matrix() {

        assert!(!status_local_dirty(false, false, None));

        assert!(!status_local_dirty(false, false, Some(true)));

        assert!(!status_local_dirty(true, true, None));

        assert!(!status_local_dirty(true, true, Some(true)));

        assert!(status_local_dirty(true, false, None));

        assert!(status_local_dirty(true, false, Some(true)));

        assert!(!status_local_dirty(true, false, Some(false)));

    }

    #[test]

    fn series_reset_flags_remote_older_than_state() {

        assert!(is_series_reset(1, 5));

    }

    #[test]

    fn series_reset_ignores_equal_and_newer_remote() {

        assert!(!is_series_reset(5, 5));

        assert!(!is_series_reset(7, 5));

    }

    #[test]

    fn untouched_multiset_matches_regardless_of_order() {

        let a = vec![("h1".to_string(), 10u64), ("h2".to_string(), 20u64)];

        let b = vec![("h2".to_string(), 20u64), ("h1".to_string(), 10u64)];

        assert!(blob_multiset_eq(&a, &b));

    }

    #[test]

    fn untouched_multiset_rejects_drift() {

        let a = vec![("h1".to_string(), 10u64)];

        let b = vec![("h1".to_string(), 10u64), ("h2".to_string(), 20u64)];

        assert!(!blob_multiset_eq(&a, &b));

        let c = vec![("h1".to_string(), 11u64)];

        assert!(!blob_multiset_eq(&a, &c));

    }

    #[test]

    fn overlap_identical_is_one() {

        let local = vec![("v".to_string(), "r".to_string(), "f".to_string(), "h".to_string(), 4u64)];

        let base = vec![test_state_entry("v", "r", "f", "h", 4)];

        assert_eq!(
            overlap_score(&local, &base, &std::collections::HashSet::new()),
            1.0
        );

    }

    #[test]

    fn overlap_disjoint_is_zero() {

        let local = vec![("v".to_string(), "r".to_string(), "f".to_string(), "h1".to_string(), 4u64)];

        let base = vec![test_state_entry("v", "r", "f", "h2", 4)];

        assert_eq!(
            overlap_score(&local, &base, &std::collections::HashSet::new()),
            0.0
        );

    }

    #[test]

    fn overlap_half_counts_shared_identities() {

        let local = vec![
            ("v".to_string(), "r".to_string(), "a".to_string(), "h".to_string(), 1u64),
            ("v".to_string(), "r".to_string(), "b".to_string(), "hx".to_string(), 1u64),
        ];

        let base = vec![
            test_state_entry("v", "r", "a", "h", 1),
            test_state_entry("v", "r", "b", "h", 1),
        ];

        assert_eq!(
            overlap_score(&local, &base, &std::collections::HashSet::new()),
            0.5
        );

    }

    #[test]

    fn overlap_ignores_excluded_identities() {

        let local = vec![("v".to_string(), "r".to_string(), "f".to_string(), "h".to_string(), 4u64)];

        let base = vec![
            test_state_entry("v", "r", "f", "h", 4),
            test_state_entry("v", "r", "other".to_string().as_str(), "x", 1),
        ];

        let mut excluded = std::collections::HashSet::new();

        excluded.insert("v\u{0}r\u{0}other".to_string());

        assert_eq!(overlap_score(&local, &base, &excluded), 1.0);

    }

    #[test]

    fn overlap_empty_is_zero() {

        let empty: Vec<(String, String, String, String, u64)> = Vec::new();

        assert_eq!(
            overlap_score(&empty, &[], &std::collections::HashSet::new()),
            0.0
        );

    }

    fn file_tuple(
        variant_id: &str,
        raw_path: &str,
        relative_path: &str,
        hash: &str,
        size_bytes: u64,
    ) -> (String, String, String, String, u64) {

        (
            variant_id.to_string(),
            raw_path.to_string(),
            relative_path.to_string(),
            hash.to_string(),
            size_bytes,
        )

    }

    #[test]

    fn restore_safe_empty_local() {

        let remote = vec![file_tuple("v", "r", "f", "h", 1)];

        assert!(restore_safe(&[], &remote));

        assert!(restore_safe(&[], &[]));

    }

    #[test]

    fn restore_safe_identical_and_superset_remote() {

        let local = vec![file_tuple("v", "r", "f", "h", 1)];

        let remote = vec![
            file_tuple("v", "r", "f", "h", 1),
            file_tuple("v", "r", "g", "h2", 2),
        ];

        assert!(restore_safe(&local, &remote));

    }

    #[test]

    fn restore_safe_rejects_divergent_and_absent() {

        let divergent = vec![file_tuple("v", "r", "f", "h2", 1)];

        let base = vec![file_tuple("v", "r", "f", "h1", 1)];

        assert!(!restore_safe(&divergent, &base));

        let missing = vec![file_tuple("v", "r", "gone", "h", 1)];

        assert!(!restore_safe(&missing, &base));

        let resized = vec![file_tuple("v", "r", "f", "h1", 99)];

        assert!(!restore_safe(&resized, &base));

    }

    #[test]

    fn restore_safe_dedupes_local_repeats() {

        let file = file_tuple("v", "r", "f", "h", 1);

        assert!(restore_safe(&[file.clone(), file.clone()], &[file]));

    }

    #[test]

    fn attribution_matrix() {

        assert!(anchor_clears(1, true, 0.0));

        assert!(anchor_clears(0, true, 0.0));

        assert!(!anchor_clears(1, false, 1.0));

        assert!(!anchor_clears(3, true, 0.49));

        assert!(anchor_clears(3, true, 0.5));

        assert!(anchor_clears(2, true, 1.0));

    }

    #[test]

    fn prefix_key_rules() {

        assert_eq!(prefix_key(None), "none");

        assert_eq!(prefix_key(Some("")), "none");

        assert_eq!(prefix_key(Some("   ")), "none");

        assert_eq!(
            prefix_key(Some("C:\\Games\\X\\")),
            prefix_key(Some("C:/Games/X"))
        );

        assert_ne!(
            prefix_key(Some("/prefix/a")),
            prefix_key(Some("/prefix/b"))
        );

        assert_eq!(
            prefix_key(Some("/prefix//a/./b/../b/")),
            prefix_key(Some("/prefix/a/b"))
        );

    }

    #[test]

    fn state_path_rejects_traversal_segments() {

        assert!(state_path("../evil", "1313140", "abc").is_err());

        assert!(state_path("steam", "../../etc", "abc").is_err());

        assert!(state_path("steam", "1313140", "abc").is_ok());

    }

    #[test]

    fn prefix_key_symlink_matches_target() {

        let dir = tempfile::tempdir().unwrap();

        let target = dir.path().join("pfx");

        std::fs::create_dir(&target).unwrap();

        std::os::unix::fs::symlink(&target, dir.path().join("link")).unwrap();

        assert_eq!(
            prefix_key(Some(target.to_str().unwrap())),
            prefix_key(Some(dir.path().join("link").to_str().unwrap()))
        );

    }

    #[test]

    fn default_variant_id_matches_hydra_vector() {

        let variant = build_default_variant("steam", "1817070");

        assert_eq!(

            variant.variant_id,

            "6bb5b19456b48c65d5b6120154934d146013679fd8673e7d42694fff131774db"

        );

    }

    #[test]

    fn aggregate_hash_matches_hydra_sekiro_vector() {

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

