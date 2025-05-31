use std::process::Command;
use std::path::PathBuf;

fn get_ludusavi_path() -> PathBuf {
    dirs::config_dir()
        .unwrap()
        .join("hydralauncher")
        .join("ludusavi")
}

pub async fn backup_game(
    object_id: &str,
    backup_path: Option<&str>,
    wine_prefix: Option<&str>,
    preview: bool,
) -> Result<String, String> {
    let ludusavi_path = get_ludusavi_path();
    let ludusavi_binary_path = ludusavi_path.join("ludusavi");

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
