//! Side-effect-free canonical parsing and explicit relative directory loading.
use super::{ConfigError, HiveConfig, Profile, Result};
use serde_json::Value;
use std::{fs, path::Path};

impl HiveConfig {
    /// Parse canonical JSON without resolving credentials or opening files.
    ///
    /// # Errors
    /// Returns `Json` for malformed/unknown fields and `InlineSecret` for literals.
    pub fn from_json(input: &str) -> Result<Self> {
        let value = serde_json::from_str(input).map_err(|_| ConfigError::Json)?;
        Self::from_value(value)
    }
    /// Parse a canonical value without runtime construction.
    ///
    /// # Errors
    /// Returns typed structure or inline credential failures.
    pub fn from_value(value: Value) -> Result<Self> {
        reject_inline(&value, "config")?;
        let original = value.clone();
        let config: Self = serde_json::from_value(value).map_err(|_| ConfigError::Json)?;
        let canonical = serde_json::to_value(&config).map_err(|_| ConfigError::Json)?;
        reject_unknown(&original, &canonical)?;
        Ok(config)
    }
    /// Read `hive.json`, Markdown profiles and contexts beneath one root.
    /// Conventional `README.md` files are directory documentation, matched
    /// case-insensitively, and are not loaded as profiles or context.
    ///
    /// # Errors
    /// Returns typed IO, JSON, frontmatter or duplicate declaration failures.
    pub fn load_dir(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref();
        let mut config = Self::from_json(&read(&root.join("hive.json"))?)?;
        for path in markdown_files(&root.join("profiles"))? {
            let text = read(&path)?;
            let (meta, body) = frontmatter(&text, &path)?;
            let mut profile: Profile = if let Some(meta) = meta {
                let value: Value = serde_yaml_ng::from_str(meta)
                    .map_err(|_| ConfigError::Frontmatter(path.display().to_string()))?;
                reject_inline(&value, "profile")?;
                let profile: Profile = serde_json::from_value(value.clone())
                    .map_err(|_| ConfigError::Frontmatter(path.display().to_string()))?;
                let canonical = serde_json::to_value(&profile).map_err(|_| ConfigError::Json)?;
                reject_unknown(&value, &canonical)
                    .map_err(|_| ConfigError::Frontmatter(path.display().to_string()))?;
                profile
            } else {
                Profile::default()
            };
            if profile.id.is_empty() {
                profile.id = stem(&path)?;
            }
            body.clone_into(&mut profile.system_prompt);
            if config.profiles.iter().any(|p| p.id == profile.id) {
                return Err(ConfigError::DuplicateId {
                    section: "profile".into(),
                    id: profile.id,
                });
            }
            config.profiles.push(profile);
        }
        for path in markdown_files(&root.join("context"))? {
            let text = read(&path)?;
            let (meta, body) = frontmatter(&text, &path)?;
            if let Some(meta) = meta {
                serde_yaml_ng::from_str::<Value>(meta)
                    .map_err(|_| ConfigError::Frontmatter(path.display().to_string()))?;
            }
            let id = stem(&path)?;
            if config
                .contexts
                .insert(id.clone(), body.to_owned())
                .is_some()
            {
                return Err(ConfigError::DuplicateId {
                    section: "context".into(),
                    id,
                });
            }
        }
        Ok(config)
    }
}
/// Load a manifest directory; delegates to `HiveConfig::load_dir`.
///
/// # Errors
/// Returns the same typed parse and filesystem errors as the associated method.
pub fn load_dir(root: impl AsRef<Path>) -> Result<HiveConfig> {
    HiveConfig::load_dir(root)
}
fn read(path: &Path) -> Result<String> {
    fs::read_to_string(path).map_err(|source| ConfigError::Io {
        path: path.display().to_string(),
        source,
    })
}
fn stem(path: &Path) -> Result<String> {
    path.file_stem()
        .and_then(|s| s.to_str())
        .map(str::to_owned)
        .ok_or_else(|| ConfigError::InvalidId("markdown filename".into()))
}
fn markdown_files(dir: &Path) -> Result<Vec<std::path::PathBuf>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(source) => {
            return Err(ConfigError::Io {
                path: dir.display().to_string(),
                source,
            });
        }
    };
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| ConfigError::Io {
            path: dir.display().to_string(),
            source,
        })?;
        let path = entry.path();
        let is_readme = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("README.md"));
        if !is_readme && path.extension().is_some_and(|s| s == "md") {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}
fn frontmatter<'a>(text: &'a str, path: &Path) -> Result<(Option<&'a str>, &'a str)> {
    let Some(first) = text.lines().next() else {
        return Ok((None, text));
    };
    if first != "---" {
        return Ok((None, text));
    }
    let start = text
        .find('\n')
        .map(|p| p + 1)
        .ok_or_else(|| ConfigError::Frontmatter(path.display().to_string()))?;
    let mut offset = start;
    for line in text[start..].split_inclusive('\n') {
        if line.trim_end_matches(['\r', '\n']) == "---" {
            return Ok((Some(&text[start..offset]), &text[offset + line.len()..]));
        }
        offset += line.len();
    }
    Err(ConfigError::Frontmatter(path.display().to_string()))
}
pub(super) fn reject_inline(value: &Value, field: &str) -> Result<()> {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let next = format!("{field}.{key}");
                if key == "endpoint" && child.as_str().is_some_and(endpoint_has_secret) {
                    return Err(ConfigError::InlineSecret { field: next });
                }
                if matches!(
                    key.as_str(),
                    "credential" | "api_key" | "password" | "token" | "secret"
                ) && !child.is_null()
                {
                    let safe = child.as_object().is_some_and(|m| {
                        m.len() == 1
                            && m.iter().all(|(k, v)| {
                                matches!(k.as_str(), "env" | "store")
                                    && v.as_str().is_some_and(|s| !s.trim().is_empty())
                            })
                    });
                    if !safe {
                        return Err(ConfigError::InlineSecret { field: next });
                    }
                }
                if matches!(key.as_str(), "env" | "headers") && child.is_object() {
                    for item in child.as_object().into_iter().flat_map(|m| m.values()) {
                        let safe = item.as_object().is_some_and(|m| {
                            m.len() == 1
                                && m.iter().all(|(k, v)| {
                                    matches!(k.as_str(), "env" | "store")
                                        && v.as_str().is_some_and(|s| !s.trim().is_empty())
                                })
                        });
                        if !safe {
                            return Err(ConfigError::InlineSecret { field: next });
                        }
                    }
                } else {
                    reject_inline(child, &next)?;
                }
            }
        }
        Value::Array(items) => {
            for child in items {
                reject_inline(child, field)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn endpoint_has_secret(endpoint: &str) -> bool {
    let authority = endpoint
        .split_once("://")
        .map(|(_, tail)| tail.split('/').next().unwrap_or(tail));
    authority.is_some_and(|a| a.contains('@'))
        || endpoint.split_once('?').is_some_and(|(_, query)| {
            query.split('&').any(|item| {
                item.split_once('=').is_some_and(|(key, _)| {
                    matches!(
                        key.to_ascii_lowercase().as_str(),
                        "token" | "api_key" | "apikey" | "password" | "secret" | "access_token"
                    )
                })
            })
        })
}

// Existing native policies intentionally accept unknown fields outside manifests.
// Compare the raw structure to its fully typed wire form at this boundary.
fn reject_unknown(raw: &Value, canonical: &Value) -> Result<()> {
    match (raw, canonical) {
        (Value::Object(raw), Value::Object(typed)) => {
            for (key, value) in raw {
                let decoded = typed.get(key).ok_or(ConfigError::Json)?;
                // Native ToolRules already denies unknown fields, but omits
                // optional nulls and empty predicates from its serialized form.
                if key == "tool_rules" {
                    continue;
                }
                reject_unknown(value, decoded)?;
            }
        }
        (Value::Array(raw), Value::Array(typed)) => {
            for (value, decoded) in raw.iter().zip(typed) {
                reject_unknown(value, decoded)?;
            }
        }
        _ => {}
    }
    Ok(())
}
