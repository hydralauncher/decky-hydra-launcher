use sha2::{Digest, Sha256};
use std::path::Path;

const CLOUD_SAVE_ENVIRONMENT_MARKER: &str = ".hydra-cloud-save-environment-id";

const ANCHOR_IDENTITY_VERSION: u32 = 2;

const MILLIS_PER_SECOND: f64 = 1000.0;
const NANOS_PER_MILLISECOND: f64 = 1_000_000.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrefixIdentityMode {
    Marker,
    Filesystem,
}

#[derive(Debug, Clone)]
pub struct ResolvedEnvironment {
    pub id: String,
    pub mode: PrefixIdentityMode,
}

#[derive(Debug, serde::Serialize)]
pub struct EnvironmentIdentity {
    version: u32,
    platform: &'static str,
    #[serde(rename = "homeDir")]
    home_dir: String,
    #[serde(rename = "documentsDir")]
    documents_dir: Option<String>,
    #[serde(rename = "appDataDir")]
    app_data_dir: Option<String>,
    #[serde(rename = "executableDirectory")]
    executable_directory: Option<String>,
    #[serde(rename = "winePrefixPath")]
    wine_prefix_path: Option<String>,
    #[serde(rename = "prefixGeneration")]
    prefix_generation: Option<String>,
    #[serde(rename = "steamPath")]
    steam_path: Option<String>,
}

pub fn environment_id_for_identity(identity: &EnvironmentIdentity) -> String {
    let serialized = serialized_identity(identity);
    format!("{:x}", Sha256::digest(serialized.as_bytes()))
}

pub fn serialized_identity(identity: &EnvironmentIdentity) -> String {
    serde_json::to_string(identity).expect("environment identity serializes")
}

fn lexical_clean_absolute(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    format!("/{}", parts.join("/"))
}

fn absolutize(value: &str) -> String {
    if value.starts_with('/') {
        return lexical_clean_absolute(value);
    }
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "/".to_string());
    lexical_clean_absolute(&format!("{cwd}/{value}"))
}

pub fn canonicalize_path(value: &str) -> Option<String> {
    if let Ok(real) = std::fs::canonicalize(value) {
        return Some(real.to_string_lossy().to_string());
    }
    Some(absolutize(value))
}

pub fn posix_dirname(path: &str) -> String {
    match path.rfind('/') {
        None => ".".to_string(),
        Some(0) => "/".to_string(),
        Some(index) => path[..index].to_string(),
    }
}

fn birthtime_ms(metadata: &std::fs::Metadata) -> Option<f64> {
    use std::os::unix::fs::MetadataExt;
    let created = metadata.created().ok().or_else(|| {
        let secs: u64 = metadata.ctime().try_into().ok()?;
        let nanos: u32 = metadata.ctime_nsec().max(0).try_into().ok()?;
        std::time::UNIX_EPOCH.checked_add(std::time::Duration::new(secs, nanos))
    })?;
    let duration = created.duration_since(std::time::UNIX_EPOCH).ok()?;
    Some(duration.as_secs() as f64 * MILLIS_PER_SECOND + f64::from(duration.subsec_nanos()) / NANOS_PER_MILLISECOND)
}

fn file_identity(path: &Path) -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    let stat = std::fs::metadata(path).ok()?;
    let birth = birthtime_ms(&stat)?;
    Some(format!("{}:{}:{birth}", stat.dev(), stat.ino()))
}

fn prefix_fingerprint(prefix: &Path) -> Option<String> {
    let prefix_identity = file_identity(prefix)?;
    let drive_identity = file_identity(&prefix.join("drive_c")).unwrap_or_else(|| "missing".to_string());
    Some(format!("prefix:{prefix_identity}|drive_c:{drive_identity}"))
}

pub fn filesystem_generation(prefix: &Path) -> String {
    match file_identity(prefix) {
        Some(identity) => format!("fs:{identity}"),
        None => "missing".to_string(),
    }
}

pub fn validate_wine_prefix(prefix: &Path) -> bool {
    for name in ["system.reg", "user.reg", "userdef.reg"] {
        if !prefix.join(name).exists() {
            return false;
        }
    }
    for name in ["dosdevices", "drive_c"] {
        let dir = prefix.join(name);
        if !dir.is_dir() {
            return false;
        }
    }
    true
}

pub fn resolve_prefix_path(requested: &str, home: &str) -> Option<String> {
    let expanded = if requested == "~" {
        home.to_string()
    } else if let Some(rest) = requested.strip_prefix("~/") {
        format!("{home}/{rest}")
    } else {
        requested.to_string()
    };
    let absolute = absolutize(&expanded);
    if let Ok(real) = std::fs::canonicalize(&absolute) {
        return Some(real.to_string_lossy().to_string());
    }
    let mut missing: Vec<String> = Vec::new();
    let mut existing = absolute.clone();
    loop {
        if Path::new(&existing).exists() {
            break;
        }
        let parent = posix_dirname(&existing);
        if parent == existing {
            break;
        }
        if let Some(base) = existing.rsplit('/').next() {
            missing.push(base.to_string());
        }
        existing = parent;
    }
    let canonical_base =
        std::fs::canonicalize(&existing).map(|p| p.to_string_lossy().to_string()).unwrap_or(existing);
    missing.reverse();
    let mut out = canonical_base;
    for segment in missing {
        out.push('/');
        out.push_str(&segment);
    }
    Some(out)
}

fn steam_location(home: &str) -> String {
    let first = format!("{home}/.steam/steam");
    let second = format!("{home}/.local/share/Steam");
    if Path::new(&first).exists() {
        return first;
    }
    if Path::new(&second).exists() {
        return second;
    }
    first
}

fn generation_value(
    resolved_prefix: &str,
    generation_store: &dyn Fn(&str) -> Option<String>,
) -> (String, PrefixIdentityMode) {
    if let Some(marker) = crate::hydra::prefix_environment_id(Some(resolved_prefix)) {
        return (format!("marker:{marker}"), PrefixIdentityMode::Marker);
    }
    let key = format!("{:x}", Sha256::digest(resolved_prefix.as_bytes()));
    if let Some(record) = generation_store(&key) {
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&record) {
            let generation_id = parsed.get("generationId").and_then(|v| v.as_str()).unwrap_or_default();
            let fingerprint = parsed.get("fingerprint").and_then(|v| v.as_str()).unwrap_or_default();
            if !generation_id.is_empty()
                && !fingerprint.is_empty()
                && crate::hydra::valid_environment_marker(generation_id)
            {
                let current = prefix_fingerprint(Path::new(resolved_prefix)).unwrap_or_default();
                if current == fingerprint {
                    return (format!("marker:{generation_id}"), PrefixIdentityMode::Marker);
                }
            }
        } else if crate::hydra::valid_environment_marker(record.trim()) {
            let generation_id = record.trim().to_lowercase();
            if let Ok(marker) =
                std::fs::read_to_string(Path::new(resolved_prefix).join(CLOUD_SAVE_ENVIRONMENT_MARKER))
            {
                if marker.trim().to_lowercase() == generation_id {
                    return (format!("marker:{generation_id}"), PrefixIdentityMode::Marker);
                }
            }
        }
    }
    (
        filesystem_generation(Path::new(resolved_prefix)),
        PrefixIdentityMode::Filesystem,
    )
}

pub struct EnvironmentInputs {
    pub home_dir: String,
    pub documents_dir: Option<String>,
    pub app_data_dir: Option<String>,
    pub executable_path: Option<String>,
    pub recorded_prefix: Option<String>,
    pub default_prefix: Option<String>,
    pub steam_path: Option<String>,
    pub generation_store: Box<dyn Fn(&str) -> Option<String>>,
}

pub fn build_identity(inputs: &EnvironmentInputs) -> EnvironmentIdentity {
    let uses_windows_compat = inputs
        .executable_path
        .as_deref()
        .is_some_and(|exe| exe.to_lowercase().ends_with(".exe"));
    let canonical_home = canonicalize_path(&inputs.home_dir).unwrap_or_else(|| inputs.home_dir.clone());
    let canonical_documents = inputs.documents_dir.as_deref().and_then(|d| canonicalize_path(d).or_else(|| Some(d.to_string())));
    let canonical_app_data = inputs.app_data_dir.as_deref().and_then(|d| canonicalize_path(d).or_else(|| Some(d.to_string())));
    let canonical_executable = inputs.executable_path.as_deref().and_then(canonicalize_path);
    let canonical_steam = inputs.steam_path.as_deref().and_then(|s| canonicalize_path(s).or_else(|| Some(s.to_string())));
    let requested_prefix: Option<String> = if !uses_windows_compat {
        None
    } else {
        inputs.recorded_prefix.clone().or_else(|| inputs.default_prefix.clone())
    };
    let resolved_prefix = requested_prefix.as_deref().and_then(|r| resolve_prefix_path(r, &canonical_home));
    let prefix_valid = resolved_prefix.as_deref().is_some_and(|p| validate_wine_prefix(Path::new(p)));
    let (prefix_generation, _mode) = match resolved_prefix.as_deref() {
        Some(prefix) if prefix_valid => {
            let (generation, mode) = generation_value(prefix, &inputs.generation_store);
            (Some(generation), mode)
        }
        Some(prefix) => (
            Some(filesystem_generation(Path::new(prefix))),
            PrefixIdentityMode::Filesystem,
        ),
        None => (None, PrefixIdentityMode::Filesystem),
    };
    EnvironmentIdentity {
        version: ANCHOR_IDENTITY_VERSION,
        platform: "linux",
        home_dir: canonical_home,
        documents_dir: canonical_documents,
        app_data_dir: canonical_app_data,
        executable_directory: canonical_executable.as_deref().map(posix_dirname),
        wine_prefix_path: resolved_prefix.clone(),
        prefix_generation,
        steam_path: canonical_steam,
    }
}

pub fn resolve_environment(inputs: &EnvironmentInputs) -> Option<ResolvedEnvironment> {
    let identity = build_identity(inputs);
    let mode = match identity.prefix_generation.as_deref() {
        Some(generation) if generation.starts_with("marker:") => PrefixIdentityMode::Marker,
        _ => PrefixIdentityMode::Filesystem,
    };
    Some(ResolvedEnvironment {
        id: environment_id_for_identity(&identity),
        mode,
    })
}

pub fn default_prefix_for_game(user_data: &str, object_id: &str, configured: Option<&str>) -> Option<String> {
    if let Some(custom) = configured.map(str::trim).filter(|c| !c.is_empty()) {
        let custom = custom.trim_end_matches('/');
        return Some(format!("{custom}/{object_id}"));
    }
    Some(format!("{user_data}/wine-prefixes/{object_id}"))
}

pub fn resolve_game_environment(shop: &str, object_id: &str) -> Option<ResolvedEnvironment> {
    let home = dirs::home_dir()?.to_string_lossy().to_string();
    let config = dirs::config_dir().map(|p| p.to_string_lossy().to_string());
    let user_data = user_data_dir(&home, config.as_deref());
    let inputs = EnvironmentInputs {
        documents_dir: Some(format!("{home}/Documents")),
        steam_path: Some(steam_location(&home)),
        home_dir: home,
        app_data_dir: config,
        executable_path: crate::hydra::get_game_executable_path(object_id, shop),
        recorded_prefix: crate::hydra::get_game_wine_prefix_path(object_id, shop),
        default_prefix: default_prefix_for_game(
            &user_data,
            object_id,
            crate::hydra::get_default_wine_prefix_override().as_deref(),
        ),
        generation_store: Box::new(|key: &str| crate::hydra::read_generation_record(key)),
    };
    resolve_environment(&inputs)
}

pub fn user_data_dir(home: &str, config: Option<&str>) -> String {
    match config {
        Some(dir) if !dir.trim().is_empty() => format!("{dir}/hydralauncher"),
        _ => format!("{home}/.config/hydralauncher"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_store() -> Box<dyn Fn(&str) -> Option<String>> {
        Box::new(|_: &str| None)
    }

    #[test]
    fn identity_json_shape_matches_launcher() {
        let identity = EnvironmentIdentity {
            version: 2,
            platform: "linux",
            home_dir: "/home/deck".to_string(),
            documents_dir: Some("/home/deck/Documents".to_string()),
            app_data_dir: Some("/home/deck/.config".to_string()),
            executable_directory: Some("/home/deck/games/cult".to_string()),
            wine_prefix_path: Some("/home/deck/.config/hydralauncher/wine-prefixes/1313140".to_string()),
            prefix_generation: Some("marker:123e4567-e89b-12d3-a456-426614174000".to_string()),
            steam_path: Some("/home/deck/.steam/steam".to_string()),
        };
        assert_eq!(
            serialized_identity(&identity),
            r#"{"version":2,"platform":"linux","homeDir":"/home/deck","documentsDir":"/home/deck/Documents","appDataDir":"/home/deck/.config","executableDirectory":"/home/deck/games/cult","winePrefixPath":"/home/deck/.config/hydralauncher/wine-prefixes/1313140","prefixGeneration":"marker:123e4567-e89b-12d3-a456-426614174000","steamPath":"/home/deck/.steam/steam"}"#
        );
    }

    #[test]
    fn environment_id_matches_node_sha256() {
        let identity = EnvironmentIdentity {
            version: 2,
            platform: "linux",
            home_dir: "/home/deck".to_string(),
            documents_dir: Some("/home/deck/Documents".to_string()),
            app_data_dir: Some("/home/deck/.config".to_string()),
            executable_directory: Some("/home/deck/games/cult".to_string()),
            wine_prefix_path: Some("/home/deck/.config/hydralauncher/wine-prefixes/1313140".to_string()),
            prefix_generation: Some("marker:123e4567-e89b-12d3-a456-426614174000".to_string()),
            steam_path: Some("/home/deck/.steam/steam".to_string()),
        };
        assert_eq!(
            environment_id_for_identity(&identity),
            "03117017066639a28c915d6190a2737e147dfe9b5efe7620b7156e40588633d3"
        );
    }

    #[test]
    fn identity_nulls_match_launcher() {        let identity = EnvironmentIdentity {
            version: 2,
            platform: "linux",
            home_dir: "/home/deck".to_string(),
            documents_dir: None,
            app_data_dir: None,
            executable_directory: None,
            wine_prefix_path: None,
            prefix_generation: None,
            steam_path: None,
        };
        assert_eq!(
            serialized_identity(&identity),
            r#"{"version":2,"platform":"linux","homeDir":"/home/deck","documentsDir":null,"appDataDir":null,"executableDirectory":null,"winePrefixPath":null,"prefixGeneration":null,"steamPath":null}"#
        );
    }

    #[test]
    fn posix_dirname_matches_node() {
        assert_eq!(posix_dirname("/a/b/c.exe"), "/a/b");
        assert_eq!(posix_dirname("game.exe"), ".");
        assert_eq!(posix_dirname("/game.exe"), "/");
        assert_eq!(posix_dirname("C:\\Games\\Foo\\game.exe"), ".");
    }

    #[test]
    fn resolve_prefix_expands_tilde() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().to_string_lossy().to_string();
        std::fs::create_dir_all(format!("{home}/pfx/drive_c")).unwrap();
        let resolved = resolve_prefix_path("~/pfx", &home).unwrap();
        assert_eq!(resolved, format!("{home}/pfx"));
    }

    #[test]
    fn resolve_prefix_keeps_missing_tail() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().to_string_lossy().to_string();
        std::fs::create_dir_all(format!("{home}/base")).unwrap();
        let resolved = resolve_prefix_path("~/base/gone/deeper", &home).unwrap();
        assert_eq!(resolved, format!("{home}/base/gone/deeper"));
    }

    #[test]
    fn validate_prefix_requires_launcher_layout() {
        let dir = tempfile::tempdir().unwrap();
        let prefix = dir.path().join("pfx");
        std::fs::create_dir_all(prefix.join("drive_c")).unwrap();
        assert!(!validate_wine_prefix(&prefix));
        for name in ["system.reg", "user.reg", "userdef.reg"] {
            std::fs::write(prefix.join(name), "x").unwrap();
        }
        assert!(!validate_wine_prefix(&prefix));
        std::fs::create_dir_all(prefix.join("dosdevices")).unwrap();
        assert!(validate_wine_prefix(&prefix));
    }

    #[test]
    fn non_exe_game_has_no_prefix_in_identity() {
        let inputs = EnvironmentInputs {
            home_dir: "/home/deck".to_string(),
            documents_dir: None,
            app_data_dir: None,
            executable_path: Some("/usr/bin/game".to_string()),
            recorded_prefix: Some("/home/deck/pfx".to_string()),
            default_prefix: None,
            steam_path: None,
            generation_store: empty_store(),
        };
        let identity = build_identity(&inputs);
        assert_eq!(identity.wine_prefix_path, None);
        assert_eq!(identity.prefix_generation, None);
    }

    #[test]
    fn generation_store_match_selects_marker() {
        let dir = tempfile::tempdir().unwrap();
        let prefix = dir.path().join("pfx");
        std::fs::create_dir_all(prefix.join("drive_c")).unwrap();
        let resolved = std::fs::canonicalize(&prefix).unwrap().to_string_lossy().to_string();
        let fingerprint = prefix_fingerprint(&prefix).unwrap();
        let key = format!("{:x}", Sha256::digest(resolved.as_bytes()));
        let store: Box<dyn Fn(&str) -> Option<String>> = Box::new(move |k: &str| {
            if k == key {
                return Some(format!(
                    r#"{{"generationId":"123e4567-e89b-12d3-a456-426614174000","fingerprint":{fingerprint:?}}}"#
                ));
            }
            None
        });
        let (value, mode) = generation_value(&resolved, &store);
        assert_eq!(value, "marker:123e4567-e89b-12d3-a456-426614174000");
        assert_eq!(mode, PrefixIdentityMode::Marker);
    }
}
