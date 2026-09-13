use anyhow::{anyhow, Result};

use serde::{Deserialize, Serialize};

use std::collections::HashMap;

use std::path::{Path, PathBuf};

#[derive(Debug)]

pub struct GameRules {

    rules: Vec<CompiledRule>,

}

#[derive(Debug)]

pub struct CompiledRule {

    pub raw_path: String,

    pub regex: regex::Regex,

    pub kind: RuleKind,

    pub has_store_user: bool,

    pub when: Vec<RuleCondition>,

}

#[derive(Debug, Clone, Serialize, Deserialize)]

pub struct RuleCondition {

    pub os: Option<String>,

    pub store: Option<String>,

}

#[derive(Debug, Deserialize)]

#[serde(rename_all = "camelCase")]

struct ManifestIndex {

    version: u32,

    games: HashMap<String, IndexedGame>,

}

#[derive(Debug, Clone, Deserialize)]

#[serde(rename_all = "camelCase")]

struct IndexedGame {

    #[serde(default)]

    files: Vec<IndexedRule>,

}

#[derive(Debug, Clone, Deserialize, Serialize)]

#[serde(rename_all = "camelCase")]

struct IndexedRule {

    raw_path: String,

    #[serde(default)]

    when: Vec<RuleCondition>,

}

fn index_path() -> Result<PathBuf> {

    Ok(dirs::config_dir()

        .ok_or_else(|| anyhow!("No config dir"))?

        .join("hydralauncher")

        .join("cloud-save-manifest-index.json"))

}

#[derive(Debug, PartialEq)]

pub enum RuleKind {

    File,

    Dir,

    Glob,

}

fn infer_rule_kind(raw_path: &str) -> RuleKind {

    if raw_path

        .chars()

        .any(|c| matches!(c, '*' | '?' | '[' | '{' | ']'))

    {

        return RuleKind::Glob;

    }

    if raw_path.ends_with('/') {

        return RuleKind::Dir;

    }

    let base_name = raw_path.rsplit('/').next().unwrap_or(raw_path);

    if base_name.contains('.') {

        RuleKind::File

    } else {

        RuleKind::Dir

    }

}

fn compile_rule(raw_path: &str, when: Vec<RuleCondition>) -> Option<CompiledRule> {

    let kind = infer_rule_kind(raw_path);

    let mut pattern = String::from("^");

    let mut has_store_user = false;

    let mut chars = raw_path.chars().peekable();

    while let Some(ch) = chars.next() {

        match ch {

            '*' => {

                if chars.peek() == Some(&'*') {

                    chars.next();

                    pattern.push_str(".*");

                } else {

                    pattern.push_str("[^/]*");

                }

            }

            '?' => pattern.push_str("[^/]"),

            '[' => {

                let mut class = String::from("[");

                let mut closed = false;

                if chars.peek() == Some(&'!') {

                    chars.next();

                    class.push('^');

                }

                for next in chars.by_ref() {

                    if next == ']' {

                        closed = true;

                        break;

                    }

                    class.push(next);

                }

                if closed {

                    class.push(']');

                    pattern.push_str(&class);

                } else {

                    pattern.push_str(r"\[");

                    pattern.push_str(&regex::escape(&class[1..]));

                }

            }

            '{' => {

                let mut group = String::from("(?:");

                let mut closed = false;

                for next in chars.by_ref() {

                    match next {

                        '}' => {

                            closed = true;

                            break;

                        }

                        ',' => group.push('|'),

                        other => group.push_str(&regex::escape(&other.to_string())),

                    }

                }

                if closed {

                    group.push(')');

                    pattern.push_str(&group);

                } else {

                    pattern.push_str(r"\{");

                    pattern.push_str(&group[3..].replace('|', ","));

                }

            }

            '<' => {

                let mut token = String::new();

                for next in chars.by_ref() {

                    if next == '>' {

                        break;

                    }

                    token.push(next);

                }

                if token == "storeUserId" {

                    has_store_user = true;

                    pattern.push_str("(?P<store_user>[^/]+)");

                } else {

                    pattern.push_str(&format!("<{token}>"));

                }

            }

            _ => pattern.push_str(&regex::escape(&ch.to_string())),

        }

    }

    if kind == RuleKind::Dir {

        pattern.push_str("(?:/.*)?$");

    } else {

        pattern.push('$');

    }

    regex::Regex::new(&pattern).ok().map(|regex| CompiledRule {

        raw_path: raw_path.trim_end_matches('/').to_string(),

        regex,

        kind,

        has_store_user,

        when,

    })

}

const WINDOWS_ONLY_TOKENS: [&str; 9] = [

    "<winAppData>",

    "%APPDATA%",

    "<winLocalAppData>",

    "%LOCALAPPDATA%",

    "<winDocuments>",

    "<winPublic>",

    "<winProgramData>",

    "<winDir>",

    "<windows>",

];

const UNIX_ONLY_TOKENS: [&str; 2] = ["<xdgData>", "<xdgConfig>"];

fn normalized_os(value: &str) -> &str {

    match value.to_ascii_lowercase().as_str() {

        "mac" | "macos" | "osx" => "mac",

        "windows" | "win" => "windows",

        "linux" => "linux",

        _ => "unknown",

    }

}

impl CompiledRule {

    pub fn is_applicable(&self, windows_compat: bool, shop: &str) -> bool {

        let effective_os = if windows_compat { "windows" } else { "linux" };

        let conditions_match = self.when.is_empty()

            || self.when.iter().any(|condition| {

                let os_matches = condition

                    .os

                    .as_deref()

                    .map_or(true, |os| normalized_os(os) == effective_os);

                let store_matches = condition

                    .store

                    .as_deref()

                    .map_or(true, |store| store.eq_ignore_ascii_case(shop));

                os_matches && store_matches

            });

        if !conditions_match {

            return false;

        }

        let foreign = if effective_os == "windows" {

            UNIX_ONLY_TOKENS

                .iter()

                .any(|token| self.raw_path.contains(token))

        } else {

            WINDOWS_ONLY_TOKENS

                .iter()

                .any(|token| self.raw_path.contains(token))

        };

        !foreign

    }

    pub fn matches(&self, candidate: &str) -> Option<Option<String>> {

        self.regex.captures(candidate).map(|captures| {

            if self.has_store_user {

                captures.name("store_user").map(|m| m.as_str().to_string())

            } else {

                None

            }

        })

    }

}

#[derive(Debug, Clone, Serialize, Deserialize)]

struct SidecarRules {

    index_mtime_secs: u64,

    index_mtime_nanos: u32,

    index_len: u64,

    index_version: u32,

    files: Vec<IndexedRule>,

}

fn plugin_data_dir() -> Result<PathBuf> {

    Ok(dirs::home_dir()

        .ok_or_else(|| anyhow!("No home dir"))?

        .join("homebrew")
        .join("data")
        .join("Hydra"))

}

fn sidecar_path(dir: &Path, object_id: &str) -> Option<PathBuf> {
    if object_id.is_empty()
        || !object_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {

        return None;

    }

    Some(dir.join(format!("rules-{object_id}.json")))

}

fn index_fingerprint(path: &Path) -> Option<(u64, u32, u64)> {

    let metadata = std::fs::metadata(path).ok()?;

    let modified = metadata.modified().ok()?;

    let duration = modified.duration_since(std::time::UNIX_EPOCH).ok()?;

    Some((duration.as_secs(), duration.subsec_nanos(), metadata.len()))

}

fn read_sidecar(
    dir: &Path,
    object_id: &str,
    mtime_secs: u64,
    mtime_nanos: u32,
    len: u64,
) -> Option<SidecarRules> {

    let path = sidecar_path(dir, object_id)?;

    let content = std::fs::read_to_string(path).ok()?;

    let cached: SidecarRules = serde_json::from_str(&content).ok()?;

    if cached.index_mtime_secs != mtime_secs
        || cached.index_mtime_nanos != mtime_nanos
        || cached.index_len != len
    {

        return None;

    }

    Some(cached)

}

fn write_sidecar(dir: &Path, object_id: &str, cached: &SidecarRules) {

    let Some(path) = sidecar_path(dir, object_id) else {

        return;

    };

    if std::fs::create_dir_all(dir).is_err() {

        return;

    }

    let Ok(content) = serde_json::to_string(cached) else {

        return;

    };

    if let Ok(entries) = std::fs::read_dir(dir) {

        let prefix = format!(".rules-{object_id}-");

        for entry in entries.flatten() {

            let name = entry.file_name().to_string_lossy().to_string();

            if !name.starts_with(&prefix) || !name.ends_with(".tmp") {

                continue;

            }

            let stale = entry
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age.as_secs() > 3600);

            if stale {

                let _ = std::fs::remove_file(entry.path());

            }

        }

    }

    let temp = dir.join(format!(".rules-{object_id}-{}.tmp", std::process::id()));

    if std::fs::write(&temp, content).is_err() {

        return;

    }

    if std::fs::rename(&temp, path).is_err() {

        let _ = std::fs::remove_file(temp);

        return;

    }

    if let Ok(legacy_dir) = crate::cloud_save::state_dir() {

        let _ = std::fs::remove_file(legacy_dir.join(format!("rules-{object_id}.json")));

    }

}

fn cached_rules(cached: &SidecarRules) -> Result<Option<GameRules>> {

    if cached.index_version != 1 {

        return Err(anyhow!(

            "Unsupported cloud save manifest index version {}",

            cached.index_version

        ));

    }

    Ok(compile_files(&cached.files))

}

fn compile_files(files: &[IndexedRule]) -> Option<GameRules> {

    let rules: Vec<CompiledRule> = files

        .iter()

        .filter_map(|rule| compile_rule(&rule.raw_path, rule.when.clone()))

        .collect();

    if rules.is_empty() {

        return None;

    }

    Some(GameRules { rules })

}

impl GameRules {

    pub fn load(object_id: &str) -> Result<Option<GameRules>> {

        let path = index_path()?;

        if !path.exists() {

            return Ok(None);

        }

        if let (Some(dir), Some((mtime_secs, mtime_nanos, len))) = (
            plugin_data_dir().ok(),
            index_fingerprint(&path),
        ) {

            if let Some(cached) = read_sidecar(&dir, object_id, mtime_secs, mtime_nanos, len) {

                return cached_rules(&cached);

            }

        }

        let content = std::fs::read_to_string(&path)

            .map_err(|e| anyhow!("Failed to read cloud save manifest index: {e}"))?;

        let fingerprint = index_fingerprint(&path);

        let index: ManifestIndex = serde_json::from_str(&content)

            .map_err(|e| anyhow!("Invalid cloud save manifest index: {e}"))?;

        if index.version != 1 {

            return Err(anyhow!(

                "Unsupported cloud save manifest index version {}",

                index.version

            ));

        }

        let files: Vec<IndexedRule> = index
            .games
            .get(object_id)
            .map(|game| game.files.clone())
            .unwrap_or_default();

        if let (Some(dir), Some((mtime_secs, mtime_nanos, len))) = (
            plugin_data_dir().ok(),
            fingerprint,
        ) {

            write_sidecar(
                &dir,
                object_id,
                &SidecarRules {
                    index_mtime_secs: mtime_secs,
                    index_mtime_nanos: mtime_nanos,
                    index_len: len,
                    index_version: index.version,
                    files: files.clone(),
                },
            );

        }

        Ok(compile_files(&files))

    }

    pub fn with_custom_bindings(

        mut self,

        bindings: &[(String, String, Option<String>)],

        windows_compat: bool,

    ) -> GameRules {

        let effective_os = if windows_compat { "<windows>" } else { "<linux>" };

        for (raw_path, _, _) in bindings {

            let marker_ok = raw_path

                .strip_prefix("<custom>")

                .is_some_and(|rest| rest.starts_with(effective_os));

            if !marker_ok {

                continue;

            }

            if let Some(mut rule) = compile_rule(raw_path, vec![]) {

                rule.kind = RuleKind::Dir;

                self.rules.push(rule);

            }

        }

        self

    }

    pub fn empty() -> GameRules {

        GameRules { rules: Vec::new() }

    }

    pub fn applicable(&self, windows_compat: bool, shop: &str) -> Vec<&CompiledRule> {

        self.rules

            .iter()

            .filter(|rule| rule.is_applicable(windows_compat, shop))

            .collect()

    }

    pub fn allows_raw_path(&self, raw_path: &str) -> bool {

        self.rules

            .iter()

            .any(|rule| rule.raw_path == raw_path || rule.regex.is_match(raw_path))

    }

}

pub struct RuleMatch<'a> {

    pub raw_path: &'a str,

    pub kind: &'a RuleKind,

    pub store_user: Option<String>,

}

impl GameRules {

    pub fn match_rule<'a>(

        &'a self,

        tokenized_path: &str,

        windows_compat: bool,

        shop: &str,

    ) -> Option<RuleMatch<'a>> {

        let mut best: Option<(&CompiledRule, Option<String>)> = None;

        for (rule, store_user) in self
            .rules
            .iter()
            .filter(|rule| rule.is_applicable(windows_compat, shop))
            .filter_map(|rule| {
                rule.matches(tokenized_path)
                    .map(|store_user| (rule, store_user))
            })
        {

            let dir_rank = (rule.kind == RuleKind::Dir) as usize;

            let replace = match &best {

                None => true,

                Some((current, _)) => {

                    let current_dir_rank = (current.kind == RuleKind::Dir) as usize;

                    (dir_rank, rule.raw_path.len()) > (current_dir_rank, current.raw_path.len())

                }

            };

            if replace {

                best = Some((rule, store_user));

            }

        }

        best.map(|(rule, store_user)| RuleMatch {

            raw_path: &rule.raw_path,

            kind: &rule.kind,

            store_user,

        })

    }

}

pub fn split_rule_match(

    rule_raw_path: &str,

    kind: &RuleKind,

    tokenized_path: &str,

    store_user: Option<&str>,

) -> (String, String) {

    let file_name = || {

        tokenized_path

            .rsplit('/')

            .next()

            .unwrap_or(tokenized_path)

            .to_string()

    };

    let concretize = |path: &str| -> String {

        match store_user {

            Some(folder) => path.replace("<storeUserId>", folder),

            None => path.to_string(),

        }

    };

    match kind {

        RuleKind::File if concretize(rule_raw_path) == tokenized_path => {

            (rule_raw_path.to_string(), file_name())

        }

        RuleKind::Dir | RuleKind::Glob => {

            let base_owned;

            let base = match kind {

                RuleKind::Dir => &concretize(rule_raw_path),

                _ => {

                    base_owned = concretize(&glob_base_path(rule_raw_path));

                    &base_owned

                }

            };

            let base = base.trim_end_matches('/');

            match tokenized_path.strip_prefix(&format!("{base}/")) {

                Some(rest) => (rule_raw_path.to_string(), rest.to_string()),

                None => (rule_raw_path.to_string(), file_name()),

            }

        }

        _ => (rule_raw_path.to_string(), file_name()),

    }

}

pub fn glob_base_path(rule_raw_path: &str) -> String {

    let segments: Vec<&str> = rule_raw_path

        .split('/')

        .take_while(|segment| {

            !segment

                .chars()

                .any(|c| matches!(c, '*' | '?' | '[' | '{'))

        })

        .collect();

    segments.join("/")

}

pub fn join_restore_path(raw_path: &str, relative_path: &str) -> String {

    let base = glob_base_path(raw_path);

    let base_name = base.rsplit('/').next().unwrap_or("");

    if base_name == relative_path {

        return base;

    }

    format!("{}/{}", base.trim_end_matches('/'), relative_path)

}

#[cfg(test)]

mod tests {

    use super::*;

    fn rules(raw: &[&str]) -> GameRules {

        GameRules {

            rules: raw

                .iter()

                .filter_map(|r| compile_rule(r, vec![]))

                .collect(),

        }

    }

    #[test]

    fn matches_file_rule_with_store_user() {

        let rules = rules(&["<winAppData>/Sekiro/<storeUserId>/S0000.sl2"]);

        let m = rules

            .match_rule("<winAppData>/Sekiro/12345/S0000.sl2", true, "steam")

            .unwrap();

        assert_eq!(m.raw_path, "<winAppData>/Sekiro/<storeUserId>/S0000.sl2");

        assert_eq!(m.store_user.as_deref(), Some("12345"));

        let (raw, rel) =

            split_rule_match(m.raw_path, m.kind, "<winAppData>/Sekiro/12345/S0000.sl2", m.store_user.as_deref());

        assert_eq!(raw, "<winAppData>/Sekiro/<storeUserId>/S0000.sl2");

        assert_eq!(rel, "S0000.sl2");

        assert_eq!(join_restore_path(&raw, &rel), raw);

    }

    #[test]

    fn dir_rule_wins_over_file_rule_on_overlap() {

        let rules = rules(&[
            "<home>/AppData/LocalLow/Massive Monster/Cult Of The Lamb/saves",
            "<home>/AppData/LocalLow/Massive Monster/Cult Of The Lamb/saves/settings.json",
        ]);

        let path =
            "<home>/AppData/LocalLow/Massive Monster/Cult Of The Lamb/saves/settings.json";

        let m = rules.match_rule(path, false, "steam").unwrap();

        assert_eq!(
            m.raw_path,
            "<home>/AppData/LocalLow/Massive Monster/Cult Of The Lamb/saves"
        );

        let (raw, rel) =
            split_rule_match(m.raw_path, m.kind, path, m.store_user.as_deref());

        assert_eq!(
            raw,
            "<home>/AppData/LocalLow/Massive Monster/Cult Of The Lamb/saves"
        );

        assert_eq!(rel, "settings.json");

    }

    #[test]

    fn dir_priority_ignores_rule_order() {

        let path =
            "<home>/AppData/LocalLow/Massive Monster/Cult Of The Lamb/saves/settings.json";

        let forward = rules(&[
            "<home>/AppData/LocalLow/Massive Monster/Cult Of The Lamb/saves",
            "<home>/AppData/LocalLow/Massive Monster/Cult Of The Lamb/saves/settings.json",
        ]);

        let reversed = rules(&[
            "<home>/AppData/LocalLow/Massive Monster/Cult Of The Lamb/saves/settings.json",
            "<home>/AppData/LocalLow/Massive Monster/Cult Of The Lamb/saves",
        ]);

        assert_eq!(
            forward.match_rule(path, false, "steam").unwrap().raw_path,
            reversed.match_rule(path, false, "steam").unwrap().raw_path,
        );

    }

    #[test]

    fn dir_rule_wins_over_glob_on_overlap() {

        let rules = rules(&["<home>/Game", "<home>/Game/*.sav"]);

        let m = rules
            .match_rule("<home>/Game/slot1.sav", false, "steam")
            .unwrap();

        assert_eq!(m.raw_path, "<home>/Game");

    }

    #[test]

    fn custom_binding_still_matches() {

        let rules = rules(&["<winAppData>/Game"]).with_custom_bindings(
            &[(
                "<custom><linux>/saves".to_string(),
                "/data/saves".to_string(),
                None,
            )],
            false,
        );

        let m = rules
            .match_rule("<custom><linux>/saves/slot.sav", false, "steam")
            .unwrap();

        assert_eq!(m.raw_path, "<custom><linux>/saves");

    }

    #[test]

    fn matches_dir_rule() {

        let rules = rules(&["<winAppData>/Game"]);

        let m = rules.match_rule("<winAppData>/Game/saves/slot1.sav", true, "steam").unwrap();

        let (raw, rel) =

            split_rule_match(m.raw_path, m.kind, "<winAppData>/Game/saves/slot1.sav", m.store_user.as_deref());

        assert_eq!(raw, "<winAppData>/Game");

        assert_eq!(rel, "saves/slot1.sav");

        assert_eq!(join_restore_path(&raw, &rel), "<winAppData>/Game/saves/slot1.sav");

    }

    #[test]

    fn matches_glob_rule() {

        let rules = rules(&["<home>/Game/*.sav"]);

        let m = rules.match_rule("<home>/Game/slot1.sav", false, "steam").unwrap();

        let (raw, rel) = split_rule_match(m.raw_path, m.kind, "<home>/Game/slot1.sav", m.store_user.as_deref());

        assert_eq!(raw, "<home>/Game/*.sav");

        assert_eq!(rel, "slot1.sav");

        assert_eq!(join_restore_path(&raw, &rel), "<home>/Game/slot1.sav");

    }

    #[test]

    fn dir_rule_does_not_match_sibling_prefix() {

        let rules = rules(&["<winAppData>/Game"]);

        assert!(rules.match_rule("<winAppData>/GameX/file.sav", true, "steam").is_none());

    }

    #[test]

    fn matches_range_and_brace_globs() {

        let rules = rules(&["<home>/Game/TEC2Slot[0-3].sol", "<base>/save{0,1}.dat"]);

        assert!(rules.match_rule("<home>/Game/TEC2Slot2.sol", false, "steam").is_some());

        assert!(rules.match_rule("<home>/Game/TEC2Slot9.sol", false, "steam").is_none());

        assert!(rules.match_rule("<base>/save1.dat", false, "steam").is_some());

        assert!(rules.match_rule("<base>/save2.dat", false, "steam").is_none());

    }

    #[test]

    fn rejects_unknown_paths() {

        let rules = rules(&["<winAppData>/Game"]);

        assert!(rules.allows_raw_path("<winAppData>/Game"));

        assert!(!rules.allows_raw_path("<home>/.ssh/authorized_keys"));

        assert!(rules.match_rule("<home>/Other/file.sav", false, "steam").is_none());

    }

    #[test]

    fn applicability_matches_reference() {

        let mut rules = rules(&["<winAppData>/Game", "<xdgData>/game"]);

        rules.rules[0].when = vec![RuleCondition {

            os: Some("windows".into()),

            store: None,

        }];

        rules.rules[1].when = vec![RuleCondition {

            os: Some("linux".into()),

            store: None,

        }];

        let proton: Vec<&str> = rules

            .applicable(true, "steam")

            .iter()

            .map(|r| r.raw_path.as_str())

            .collect();

        assert_eq!(proton, vec!["<winAppData>/Game"]);

        let native: Vec<&str> = rules

            .applicable(false, "steam")

            .iter()

            .map(|r| r.raw_path.as_str())

            .collect();

        assert_eq!(native, vec!["<xdgData>/game"]);

    }

    #[test]

    fn plugin_data_dir_lives_under_homebrew_data() {

        let dir = plugin_data_dir().unwrap();

        assert!(dir.ends_with("homebrew/data/Hydra"));

    }

    #[test]

    fn sidecar_round_trip_hits_on_matching_fingerprint() {

        let dir = tempfile::tempdir().unwrap();

        let cached = SidecarRules {
            index_mtime_secs: 100,
            index_mtime_nanos: 200,
            index_len: 300,
            index_version: 1,
            files: vec![IndexedRule {
                raw_path: "<winAppData>/Game".to_string(),
                when: vec![],
            }],
        };

        write_sidecar(dir.path(), "10", &cached);

        let hit = read_sidecar(dir.path(), "10", 100, 200, 300).unwrap();

        assert_eq!(hit.index_version, 1);

        assert_eq!(hit.files.len(), 1);

        assert_eq!(hit.files[0].raw_path, "<winAppData>/Game");

        assert!(compile_files(&hit.files).is_some());

    }

    #[test]

    fn sidecar_misses_on_stale_corrupt_or_foreign_keys() {
        let dir = tempfile::tempdir().unwrap();

        let cached = SidecarRules {
            index_mtime_secs: 100,
            index_mtime_nanos: 200,
            index_len: 300,
            index_version: 1,
            files: vec![],
        };

        write_sidecar(dir.path(), "10", &cached);

        assert!(read_sidecar(dir.path(), "10", 101, 200, 300).is_none());

        assert!(read_sidecar(dir.path(), "10", 100, 200, 301).is_none());

        assert!(read_sidecar(dir.path(), "11", 100, 200, 300).is_none());

        assert!(read_sidecar(dir.path(), "../evil", 100, 200, 300).is_none());

        std::fs::write(
            dir.path().join("rules-10.json"),
            "{not json",
        )
        .unwrap();

        assert!(read_sidecar(dir.path(), "10", 100, 200, 300).is_none());

    }


    #[test]

    fn cached_rules_rejects_unsupported_index_version() {

        let cached = SidecarRules {
            index_mtime_secs: 1,
            index_mtime_nanos: 2,
            index_len: 3,
            index_version: 2,
            files: vec![],
        };

        assert!(cached_rules(&cached).is_err());

    }

    #[test]

    fn cached_rules_negative_result_compiles_to_none() {

        let cached = SidecarRules {
            index_mtime_secs: 1,
            index_mtime_nanos: 2,
            index_len: 3,
            index_version: 1,
            files: vec![],
        };

        assert!(cached_rules(&cached).unwrap().is_none());

    }
}
