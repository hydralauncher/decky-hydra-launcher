use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use anyhow::{anyhow, Result};
use regex::Regex;

type RegValue = RegValueEnum;

#[derive(Debug)]
enum RegValueEnum {
    String(String),
    Number(u32),
}

#[derive(Debug)]
struct RegEntry {
    path: String,
    timestamp: Option<String>,
    values: HashMap<String, RegValue>,
}

fn parse_reg_file(content: &str) -> Vec<RegEntry> {
    let content = content.strip_prefix('\u{feff}').unwrap_or(content);
    let mut entries = Vec::new();

    let mut current_entry: Option<RegEntry> = None;

    let time_regex = Regex::new(r"^#time=(\w+)").unwrap();
    let section_regex = Regex::new(r#"^\[(.+?)\](?:\s+\d+)?"#).unwrap();
    let kv_regex = Regex::new(r#"^"?(.*?)"?=(.*)$"#).unwrap();

    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }

        if line.starts_with('#') {
            if let Some(caps) = time_regex.captures(line) {
                if let Some(entry) = current_entry.as_mut() {
                    entry.timestamp = Some(caps[1].to_string());
                }
            }
            continue;
        }

        if let Some(caps) = section_regex.captures(line) {
            if let Some(entry) = current_entry.take() {
                entries.push(entry);
            }
            let path = caps[1].to_string();
            current_entry = Some(RegEntry {
                path,
                timestamp: None,
                values: HashMap::new(),
            });
        } else if let Some(caps) = kv_regex.captures(line) {
            if let Some(entry) = current_entry.as_mut() {
                let mut key = if caps[1].is_empty() {
                    "@".to_string()
                } else {
                    caps[1].to_string()
                };

                key = key.trim_matches('"').to_string();
                let raw_value = caps[2].trim();

                let value = if raw_value == r#""""# {
                    RegValue::String(String::new())
                } else if raw_value.starts_with("dword:") {
                    let num = u32::from_str_radix(&raw_value[6..], 16).unwrap_or(0);
                    RegValue::Number(num)
                } else if raw_value.starts_with('"') && raw_value.ends_with('"') {
                    RegValue::String(raw_value[1..raw_value.len() - 1].to_string())
                } else {
                    RegValue::String(raw_value.to_string())
                };

                if let RegValue::Number(n) = &value {
                    let _ = n;
                }

                entry.values.insert(key, value);
            }
        }
    }

    if let Some(entry) = current_entry {
        entries.push(entry);
    }

    entries
}

pub fn get_windows_like_user_profile_path(wine_prefix_path: &str) -> Result<String> {
    let user_reg_path = PathBuf::from(&wine_prefix_path).join("user.reg");
    let user_reg_content = fs::read_to_string(&user_reg_path)
        .map_err(|e| anyhow!("Failed to read user.reg: {}", e))?;

    let entries = parse_reg_file(&user_reg_content);

    let volatile_environment = entries
        .into_iter()
        .find(|entry| entry.path == "Volatile Environment")
        .ok_or_else(|| anyhow!("Volatile environment not found in user.reg"))?;

    let profile_key = volatile_environment
        .values
        .keys()
        .find(|k| k.trim() == "USERPROFILE")
        .cloned();

    let profile_value = profile_key
        .and_then(|key| volatile_environment.values.get(&key));

    match profile_value {
        Some(RegValue::String(profile)) => Ok(normalize_path(&profile)),
        _ => Err(anyhow!("User profile not found in user.reg")),
    }
}


pub fn normalize_path(path: &str) -> String {
    let replaced = path.replace('\\', "/");

    let mut components = vec![];
    let mut parts = replaced.split('/');

    let mut prefix = String::new();
    if let Some(first) = parts.next() {
        if first.ends_with(':') {
            prefix = format!("{}/", first);
        } else if !first.is_empty() {
            components.push(first);
        }
    }

    for part in parts {
        match part {
            "" | "." => continue,
            ".." => {
                components.pop();
            }
            _ => components.push(part),
        }
    }

    format!("{}{}", prefix, components.join("/"))
}
