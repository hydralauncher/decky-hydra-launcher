use anyhow::{anyhow, Result};
use std::path::PathBuf;

use crate::ludusavi::backup_game;

/// Save rules from the ludusavi manifest, so snapshot rawPaths match the
/// desktop launcher byte-for-byte (rule paths, not fabricated dirnames).

#[derive(Debug)]
pub struct GameRules {
    rules: Vec<CompiledRule>,
}

#[derive(Debug, PartialEq)]
pub enum RuleKind {
    File,
    Dir,
    Glob,
}

#[derive(Debug)]
struct CompiledRule {
    raw_path: String,
    regex: regex::Regex,
    kind: RuleKind,
    has_store_user: bool,
}

fn manifest_path() -> Result<PathBuf> {
    Ok(dirs::config_dir()
        .ok_or_else(|| anyhow!("No config dir"))?
        .join("hydralauncher")
        .join("decky-ludusavi")
        .join("manifest-https___cdn.losbroxas.org_manifest.yaml"))
}

// Mirrors hydra native manifest/rules.rs infer_rule_kind.
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

fn compile_rule(raw_path: &str) -> Option<CompiledRule> {
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
            '?' => pattern.push('.'),
            '[' => {
                // Character class, ludusavi globset semantics: [!...] negates.
                let mut class = String::from("[");
                if chars.peek() == Some(&'!') {
                    chars.next();
                    class.push('^');
                }
                for next in chars.by_ref() {
                    if next == ']' {
                        break;
                    }
                    class.push(next);
                }
                class.push(']');
                pattern.push_str(&class);
            }
            '{' => {
                // Brace alternation: {a,b} matches a or b.
                let mut group = String::from("(?:");
                for next in chars.by_ref() {
                    match next {
                        '}' => break,
                        ',' => group.push('|'),
                        other => group.push_str(&regex::escape(&other.to_string())),
                    }
                }
                group.push(')');
                pattern.push_str(&group);
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
    })
}

impl GameRules {
    fn from_manifest_yaml(content: &str, object_id: &str) -> Option<GameRules> {
        let parsed: serde_yaml::Value = serde_yaml::from_str(content).ok()?;
        let files = parsed.get(object_id)?.get("files")?.as_mapping()?;

        let rules: Vec<CompiledRule> = files
            .keys()
            .filter_map(|key| key.as_str())
            .filter_map(compile_rule)
            .collect();

        if rules.is_empty() {
            return None;
        }
        Some(GameRules { rules })
    }

    /// Loads rules for the game from ludusavi's cached manifest, running a
    /// ludusavi preview first when the cache is missing (which downloads it).
    pub async fn load(object_id: &str, wine_prefix: Option<&str>) -> Result<Option<GameRules>> {
        let path = manifest_path()?;
        if !path.exists() {
            let _ = backup_game(object_id, None, wine_prefix, true).await;
        }

        let content = match std::fs::read_to_string(&path) {
            Ok(content) => content,
            Err(_) => return Ok(None),
        };

        Ok(GameRules::from_manifest_yaml(&content, object_id))
    }

    /// Longest matching rule for a tokenized full file path.
    pub fn match_rule<'a>(&'a self, tokenized_path: &str) -> Option<RuleMatch<'a>> {
        self.rules
            .iter()
            .filter_map(|rule| {
                rule.regex
                    .captures(tokenized_path)
                    .map(|captures| (rule, captures))
            })
            .max_by_key(|(rule, _)| rule.raw_path.len())
            .map(|(rule, captures)| {
                let store_user = if rule.has_store_user {
                    captures
                        .name("store_user")
                        .map(|m| m.as_str().to_string())
                } else {
                    None
                };
                RuleMatch {
                    raw_path: &rule.raw_path,
                    kind: &rule.kind,
                    store_user,
                }
            })
    }

    /// Whether a manifest-provided rawPath is one of the game's known rules.
    /// Guards restores against writing to paths ludusavi never designated.
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

/// Splits a rule match into the snapshot (rawPath, relativePath) pair,
/// mirroring the desktop launcher: file rules keep the full rule path with
/// the file name as relativePath; dir rules use the rule path as root; glob
/// rules keep the pattern and use the path relative to the glob base.
pub fn split_rule_match(
    rule_raw_path: &str,
    kind: &RuleKind,
    tokenized_path: &str,
) -> (String, String) {
    let file_name = || {
        tokenized_path
            .rsplit('/')
            .next()
            .unwrap_or(tokenized_path)
            .to_string()
    };

    match kind {
        RuleKind::File if rule_raw_path == tokenized_path => {
            (rule_raw_path.to_string(), file_name())
        }
        RuleKind::Dir | RuleKind::Glob => {
            let base_owned;
            let base = match kind {
                RuleKind::Dir => rule_raw_path,
                _ => {
                    base_owned = glob_base_path(rule_raw_path);
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

/// Rule path up to the first segment containing a glob character.
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

/// Restores use this to decide whether rawPath already ends at the file
/// (file rules) or needs relativePath appended (dir/glob rules).
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
            rules: raw.iter().filter_map(|r| compile_rule(r)).collect(),
        }
    }

    #[test]
    fn matches_file_rule_with_store_user() {
        let rules = rules(&["<winAppData>/Sekiro/<storeUserId>/S0000.sl2"]);
        let m = rules
            .match_rule("<winAppData>/Sekiro/12345/S0000.sl2")
            .unwrap();
        assert_eq!(m.raw_path, "<winAppData>/Sekiro/<storeUserId>/S0000.sl2");
        assert_eq!(m.store_user.as_deref(), Some("12345"));

        let (raw, rel) =
            split_rule_match(m.raw_path, m.kind, "<winAppData>/Sekiro/12345/S0000.sl2");
        assert_eq!(raw, "<winAppData>/Sekiro/<storeUserId>/S0000.sl2");
        assert_eq!(rel, "S0000.sl2");
        assert_eq!(join_restore_path(&raw, &rel), raw);
    }

    #[test]
    fn matches_dir_rule() {
        let rules = rules(&["<winAppData>/Game"]);
        let m = rules.match_rule("<winAppData>/Game/saves/slot1.sav").unwrap();
        let (raw, rel) =
            split_rule_match(m.raw_path, m.kind, "<winAppData>/Game/saves/slot1.sav");
        assert_eq!(raw, "<winAppData>/Game");
        assert_eq!(rel, "saves/slot1.sav");
        assert_eq!(join_restore_path(&raw, &rel), "<winAppData>/Game/saves/slot1.sav");
    }

    #[test]
    fn matches_glob_rule() {
        let rules = rules(&["<home>/Game/*.sav"]);
        let m = rules.match_rule("<home>/Game/slot1.sav").unwrap();
        let (raw, rel) = split_rule_match(m.raw_path, m.kind, "<home>/Game/slot1.sav");
        assert_eq!(raw, "<home>/Game/*.sav");
        assert_eq!(rel, "slot1.sav");
        assert_eq!(join_restore_path(&raw, &rel), "<home>/Game/slot1.sav");
    }

    #[test]
    fn dir_rule_does_not_match_sibling_prefix() {
        let rules = rules(&["<winAppData>/Game"]);
        assert!(rules.match_rule("<winAppData>/GameX/file.sav").is_none());
    }

    #[test]
    fn matches_range_and_brace_globs() {
        let rules = rules(&["<home>/Game/TEC2Slot[0-3].sol", "<base>/save{0,1}.dat"]);
        assert!(rules.match_rule("<home>/Game/TEC2Slot2.sol").is_some());
        assert!(rules.match_rule("<home>/Game/TEC2Slot9.sol").is_none());
        assert!(rules.match_rule("<base>/save1.dat").is_some());
        assert!(rules.match_rule("<base>/save2.dat").is_none());
    }

    #[test]
    fn rejects_unknown_paths() {
        let rules = rules(&["<winAppData>/Game"]);
        assert!(rules.allows_raw_path("<winAppData>/Game"));
        assert!(!rules.allows_raw_path("<home>/.ssh/authorized_keys"));
        assert!(rules.match_rule("<home>/Other/file.sav").is_none());
    }
}
