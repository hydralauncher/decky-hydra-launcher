use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::cloud_save::{install_dir_from_executable, steam_root};
use crate::rules::{glob_base_path, GameRules, RuleKind};
use crate::wine::get_windows_like_user_profile_path;

/// Everything the scanner needs to resolve rule tokens to local paths.
pub struct ScanContext {
    pub wine_prefix: Option<String>,
    pub home_dir: Option<PathBuf>,
    pub install_dir: Option<String>,
    pub steam_root: Option<PathBuf>,
    pub windows_compat: bool,
}

impl ScanContext {
    pub fn build(object_id: &str, shop: &str, wine_prefix: Option<&str>) -> ScanContext {
        let executable_path = crate::hydra::get_game_executable_path(object_id, shop);
        let windows_compat = wine_prefix.is_some()
            && executable_path
                .as_deref()
                .is_some_and(|p| p.to_ascii_lowercase().ends_with(".exe"));

        ScanContext {
            wine_prefix: wine_prefix.map(|p| p.to_string()),
            home_dir: dirs::home_dir(),
            install_dir: install_dir_from_executable(executable_path.as_deref()),
            steam_root: steam_root(),
            windows_compat,
        }
    }

    fn wine_profile_root(&self) -> Option<String> {
        let prefix = self.wine_prefix.as_ref()?;
        let profile = get_windows_like_user_profile_path(prefix).ok()?;
        let name = profile.replace('\\', "/");
        let name = name.rsplit('/').next()?;
        Some(format!("{prefix}/drive_c/users/{name}"))
    }

    /// Local roots for a leading token, as (token, local path) pairs.
    /// <home> resolves to both the linux home and — under Proton — the wine
    /// user profile, matching ludusavi's dual meaning.
    fn token_roots(&self) -> Vec<(&'static str, String)> {
        let mut roots = Vec::new();

        if let Some(profile) = self.wine_profile_root() {
            roots.push(("<winAppData>", format!("{profile}/AppData/Roaming")));
            roots.push(("<winLocalAppData>", format!("{profile}/AppData/Local")));
            roots.push(("<winDocuments>", format!("{profile}/Documents")));
            if self.windows_compat {
                roots.push(("<home>", profile.clone()));
            }
            if let Some(prefix) = &self.wine_prefix {
                roots.push(("<winPublic>", format!("{prefix}/drive_c/users/Public")));
                roots.push(("<winProgramData>", format!("{prefix}/drive_c/ProgramData")));
            }
        }

        if let Some(home) = &self.home_dir {
            let home = home.to_string_lossy().to_string();
            roots.push(("<home>", home.clone()));
            roots.push(("<xdgData>", format!("{home}/.local/share")));
            roots.push(("<xdgConfig>", format!("{home}/.config")));
        }

        if let Some(install_dir) = &self.install_dir {
            roots.push(("<base>", install_dir.clone()));
        }
        if let Some(steam_root) = &self.steam_root {
            roots.push(("<root>", steam_root.to_string_lossy().to_string()));
        }

        roots
    }

    /// Resolves a static rule prefix (no glob segments) to local directories.
    /// `<storeUserId>` segments enumerate the existing subdirectories, and the
    /// returned token prefix carries the concrete folder name.
    fn resolve_prefix(&self, token_prefix: &str) -> Vec<(String, String)> {
        let segments: Vec<&str> = token_prefix.split('/').collect();

        let mut current: Vec<(String, String)> = self
            .token_roots()
            .into_iter()
            .filter(|(token, _)| *token == segments[0])
            .map(|(token, local)| (local, token.to_string()))
            .collect();

        for segment in &segments[1..] {
            let mut next = Vec::new();
            for (local, tokens) in current {
                if *segment == "<storeUserId>" {
                    if let Ok(entries) = std::fs::read_dir(&local) {
                        for entry in entries.flatten() {
                            if entry.path().is_dir() {
                                let name = entry.file_name().to_string_lossy().to_string();
                                next.push((
                                    format!("{local}/{name}"),
                                    format!("{tokens}/{name}"),
                                ));
                            }
                        }
                    }
                } else {
                    next.push((format!("{local}/{segment}"), format!("{tokens}/{segment}")));
                }
            }
            current = next;
        }

        current
    }
}

fn walk_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_files(&path, out);
        } else if path.is_file() {
            out.push(path);
        }
    }
}

/// Scans local save files for a game by expanding its manifest rules.
/// Returns (real path, tokenized path) pairs.
pub fn scan_game_saves(ctx: &ScanContext, rules: &GameRules) -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    for rule in rules.applicable(ctx.windows_compat, "steam") {
        let static_prefix = match rule.kind {
            RuleKind::Glob => glob_base_path(&rule.raw_path),
            _ => rule.raw_path.clone(),
        };

        for (local_prefix, token_prefix) in ctx.resolve_prefix(&static_prefix) {
            let prefix_path = Path::new(&local_prefix);

            if prefix_path.is_file() {
                if rule.matches(&token_prefix).is_some() && seen.insert(token_prefix.clone()) {
                    out.push((prefix_path.to_path_buf(), token_prefix));
                }
                continue;
            }

            let mut files = Vec::new();
            walk_files(prefix_path, &mut files);
            for file in files {
                let Ok(rel) = file.strip_prefix(&local_prefix) else {
                    continue;
                };
                let candidate = format!(
                    "{}/{}",
                    token_prefix.trim_end_matches('/'),
                    rel.to_string_lossy().replace('\\', "/")
                );
                if rule.matches(&candidate).is_some() && seen.insert(candidate.clone()) {
                    out.push((file, candidate));
                }
            }
        }
    }

    out
}

