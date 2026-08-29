use std::process::Command;
use std::path::PathBuf;

const LUDUSAVI_CONFIG: &str = r#"manifest:
  enable: false
  secondary:
    - url: https://cdn.losbroxas.org/manifest.yaml
      enable: true
customGames: []
"#;

/// Ludusavi working directory (config + manifest cache). Kept separate from
/// the desktop launcher's directory so the pinned manifest config is enforced.
fn get_ludusavi_config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap()
        .join("hydralauncher")
        .join("decky-ludusavi")
}

/// Resolve the ludusavi binary: prefer the copy bundled with the plugin
/// (next to the backend binary), fall back to the desktop launcher's copy.
fn get_ludusavi_binary_path() -> Option<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let bundled = dir.join("ludusavi");
            if bundled.exists() {
                return Some(bundled);
            }
        }
    }

    let desktop_copy = dirs::config_dir()
        .unwrap()
        .join("hydralauncher")
        .join("ludusavi")
        .join("ludusavi");
    if desktop_copy.exists() {
        return Some(desktop_copy);
    }

    None
}

pub fn check_if_ludusavi_binary_exists() -> bool {
    get_ludusavi_binary_path().is_some()
}

fn ensure_ludusavi_config() -> Result<PathBuf, String> {
    let config_path = get_ludusavi_config_path();
    std::fs::create_dir_all(&config_path)
        .map_err(|e| format!("Failed to create ludusavi config dir: {e}"))?;

    // Always (re)write so the pinned manifest source is enforced.
    let config_file = config_path.join("config.yaml");
    std::fs::write(&config_file, LUDUSAVI_CONFIG)
        .map_err(|e| format!("Failed to write ludusavi config: {e}"))?;

    Ok(config_path)
}

pub async fn backup_game(
    object_id: &str,
    backup_path: Option<&str>,
    wine_prefix: Option<&str>,
    preview: bool,
) -> Result<String, String> {
    let ludusavi_binary_path = get_ludusavi_binary_path()
        .ok_or_else(|| "Ludusavi binary not found".to_string())?;
    let ludusavi_path = ensure_ludusavi_config()?;

    let mut args = vec![
        "--config".into(),
        ludusavi_path.to_string_lossy().to_string(),
        "backup".into(),
        object_id.to_string(),
        "--api".into(),
        "--force".into(),
    ];

    if preview {
        args.push("--preview".into());
    }
    if let Some(path) = backup_path {
        args.push("--path".into());
        args.push(path.to_string());
    }
    if let Some(prefix) = wine_prefix {
        args.push("--wine-prefix".into());
        args.push(prefix.to_string());
    }

    let output = Command::new(&ludusavi_binary_path)
        .args(&args)
        .output()
        .map_err(|e| format!("Failed to start Ludusavi: {}", e))?;

    if !output.status.success() {
        return Err(format!(
            "Ludusavi failed with status {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

pub async fn get_backup_preview(
    object_id: &str,
    wine_prefix: Option<&str>,
) -> Result<String, String> {
    backup_game(object_id, None, wine_prefix, true).await
}
