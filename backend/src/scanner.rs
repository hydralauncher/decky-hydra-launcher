use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::cloud_save::{install_dir_from_executable, steam_root};
use crate::rules::{glob_base_path, GameRules, RuleKind};
use crate::wine::get_windows_like_user_profile_path;

/// Everything the scanner needs to resolve rule tokens to local paths.
pub struct ScanContext {
    pub wine_prefix: Option<String>,
    pub shop: String,
    pub home_dir: Option<PathBuf>,
    pub install_dir: Option<String>,
    pub steam_root: Option<PathBuf>,
    pub windows_compat: bool,
    pub custom_paths: Vec<(String, String)>,
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
            shop: shop.to_string(),
            home_dir: dirs::home_dir(),
            install_dir: install_dir_from_executable(executable_path.as_deref()),
            steam_root: steam_root(executable_path.as_deref()),
            windows_compat,
            custom_paths: crate::hydra::get_custom_paths(object_id, shop),
        }
    }

    fn wine_user_name(&self) -> Option<String> {
        let prefix = self.wine_prefix.as_ref()?;
        let profile = get_windows_like_user_profile_path(prefix).ok()?;
        profile
            .replace('\\', "/")
            .rsplit('/')
            .next()
            .map(|s| s.to_string())
    }

    fn wine_profile_root(&self) -> Option<String> {
        Some(format!(
            "{}/drive_c/users/{}",
            self.wine_prefix.as_ref()?,
            self.wine_user_name()?
        ))
    }

    /// Local roots for a leading token, as (token, local path) pairs.
    /// Under Proton <home> means the wine user profile only (mirrors the
    /// reference: no fallback to the host home while a prefix is active).
    fn token_roots(&self) -> Vec<(&'static str, String)> {
        let mut roots = Vec::new();

        if let Some(prefix) = &self.wine_prefix {
            roots.push(("<winPublic>", format!("{prefix}/drive_c/users/Public")));
            roots.push(("<winProgramData>", format!("{prefix}/drive_c/ProgramData")));
            roots.push(("<winDir>", format!("{prefix}/drive_c/windows")));
            roots.push(("<windows>", format!("{prefix}/drive_c/windows")));
        }

        if let Some(profile) = self.wine_profile_root() {
            roots.push(("<winAppData>", format!("{profile}/AppData/Roaming")));
            roots.push(("<winLocalAppData>", format!("{profile}/AppData/Local")));
            roots.push(("<winDocuments>", format!("{profile}/Documents")));

            // Legacy XP-era layouts some older games still use.
            roots.push(("<winAppData>", format!("{profile}/Application Data")));
            roots.push((
                "<winLocalAppData>",
                format!("{profile}/Local Settings/Application Data"),
            ));
            roots.push(("<winDocuments>", format!("{profile}/My Documents")));

            if let Some(name) = self.wine_user_name() {
                roots.push(("<osUserName>", name));
            }

            if self.windows_compat {
                roots.push(("<home>", profile.clone()));
            }
        }

        if !self.windows_compat {
            if let Some(home) = &self.home_dir {
                let home = home.to_string_lossy().to_string();
                roots.push(("<home>", home.clone()));
                roots.push(("<xdgData>", format!("{home}/.local/share")));
                roots.push(("<xdgConfig>", format!("{home}/.config")));
            }
        }

        if let Some(install_dir) = &self.install_dir {
            roots.push(("<base>", install_dir.clone()));
        }
        if let Some(steam_root) = &self.steam_root {
            roots.push(("<root>", steam_root.to_string_lossy().to_string()));
        }

        roots
    }

    /// Corrects path segments to the on-disk casing (wine paths are
    /// case-insensitive; linux filesystems are not).
    fn resolve_case_insensitive(&self, path: &str) -> String {
        if !self.windows_compat {
            return path.to_string();
        }

        let mut current = String::new();
        for segment in path.trim_start_matches('/').split('/') {
            let candidate = format!("{current}/{segment}");
            if Path::new(&candidate).exists() {
                current = candidate;
                continue;
            }
            // Look up the real casing in the parent directory.
            let corrected = std::fs::read_dir(if current.is_empty() { "/" } else { &current })
                .ok()
                .and_then(|entries| {
                    entries.flatten().find(|entry| {
                        entry.file_name().to_string_lossy().eq_ignore_ascii_case(segment)
                    })
                })
                .map(|entry| entry.file_name().to_string_lossy().to_string());
            match corrected {
                Some(name) => current = format!("{current}/{name}"),
                None => current = candidate,
            }
        }
        current
    }

    /// Resolves a static rule prefix (no glob segments) to local directories.
    /// Standalone `<storeUserId>` segments enumerate the existing
    /// subdirectories, and the returned token prefix carries the concrete
    /// folder name.
    fn resolve_prefix(&self, token_prefix: &str) -> Vec<(String, String)> {
        // Custom save-path bindings resolve verbatim from the stored local path.
        for (raw_path, local_path) in &self.custom_paths {
            if token_prefix == raw_path {
                return vec![(local_path.clone(), raw_path.clone())];
            }
            if let Some(rest) = token_prefix.strip_prefix(&format!("{raw_path}/")) {
                return vec![(format!("{local_path}/{rest}"), token_prefix.to_string())];
            }
        }

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
                    let raw_local = format!("{local}/{segment}");
                    next.push((self.resolve_case_insensitive(&raw_local), format!("{tokens}/{segment}")));
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
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        // Never follow symlinks: a cycle would recurse without bound.
        if file_type.is_symlink() {
            if entry.path().is_file() {
                out.push(entry.path());
            }
            continue;
        }
        if file_type.is_dir() {
            walk_files(&entry.path(), out);
        } else if file_type.is_file() {
            out.push(entry.path());
        }
    }
}

/// Scans local save files for a game by expanding its manifest rules.
/// Returns (real path, tokenized path) pairs.
pub fn scan_game_saves(ctx: &ScanContext, rules: &GameRules) -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    for rule in rules.applicable(ctx.windows_compat, &ctx.shop) {
        // The scannable prefix stops at the first glob segment or at any
        // segment embedding <storeUserId> (e.g. `PlayerProfile<storeUserId>.sav`).
        let raw_prefix = match rule.kind {
            RuleKind::Glob => glob_base_path(&rule.raw_path),
            _ => rule.raw_path.clone(),
        };
        let static_prefix = raw_prefix
            .split('/')
            .take_while(|segment| !segment.contains("<storeUserId>"))
            .collect::<Vec<_>>()
            .join("/");

        if static_prefix.is_empty() {
            continue;
        }

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

