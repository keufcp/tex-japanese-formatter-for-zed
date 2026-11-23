use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FormatterConfig {
    pub enabled: bool,
    pub format_on_save: bool,
    pub target_languages: Vec<String>,
    /// Top-level frontmatter keys whose values are left untouched.
    pub frontmatter_exclude_keys: Vec<String>,
}

impl Default for FormatterConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            format_on_save: true,
            target_languages: vec!["latex".to_string()],
            frontmatter_exclude_keys: Vec::new(),
        }
    }
}

const KNOWN_KEYS: [&str; 4] = [
    "enabled",
    "formatOnSave",
    "targetLanguages",
    "frontmatterExcludeKeys",
];

impl FormatterConfig {
    /// Builds a config from either the settings object itself or a value that
    /// wraps it in a `settings` field. Missing fields take their defaults.
    ///
    /// Returns `None` if the value is malformed or contains none of the known
    /// keys (e.g. `{}`), so that such a value does not silently reset the
    /// current config to the defaults.
    pub fn from_value(value: &serde_json::Value) -> Option<Self> {
        let settings = match value.get("settings") {
            Some(inner) if inner.is_object() => inner,
            _ => value,
        };
        let object = settings.as_object()?;
        if !KNOWN_KEYS.iter().any(|key| object.contains_key(*key)) {
            return None;
        }
        serde_json::from_value(settings.clone()).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let config = FormatterConfig::from_value(&json!({ "targetLanguages": ["mdx"] })).unwrap();
        assert!(config.enabled);
        assert!(config.format_on_save);
        assert_eq!(config.target_languages, vec!["mdx"]);
        assert!(config.frontmatter_exclude_keys.is_empty());
    }

    #[test]
    fn reads_wrapped_settings() {
        let config = FormatterConfig::from_value(&json!({
            "settings": { "enabled": false, "frontmatterExcludeKeys": ["slug"] }
        }))
        .unwrap();
        assert!(!config.enabled);
        assert_eq!(config.frontmatter_exclude_keys, vec!["slug"]);
        assert_eq!(config.target_languages, vec!["latex"]);
    }

    #[test]
    fn rejects_malformed_values() {
        assert!(FormatterConfig::from_value(&json!(null)).is_none());
        assert!(FormatterConfig::from_value(&json!({ "enabled": "yes" })).is_none());
    }

    #[test]
    fn rejects_values_without_known_keys() {
        assert!(FormatterConfig::from_value(&json!({})).is_none());
        assert!(FormatterConfig::from_value(&json!({ "settings": {} })).is_none());
        assert!(
            FormatterConfig::from_value(&json!({
                "tex-japanese-formatter": { "targetLanguages": ["mdx"] }
            }))
            .is_none()
        );
    }
}
