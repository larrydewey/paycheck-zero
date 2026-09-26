//! Externalized user-facing strings (spec §13.9). English only in v1; every
//! string shown to users lives in `locales/en.json`.

use std::collections::HashMap;
use std::sync::OnceLock;

const EN: &str = include_str!("../locales/en.json");

fn strings() -> &'static HashMap<String, String> {
    static MAP: OnceLock<HashMap<String, String>> = OnceLock::new();
    MAP.get_or_init(|| serde_json::from_str(EN).unwrap_or_default())
}

/// Looks up a string. Missing keys render as the key itself (and are caught
/// by the `all_keys_exist` test).
#[must_use]
pub fn t(key: &str) -> String {
    strings().get(key).cloned().unwrap_or_else(|| key.to_string())
}

/// Looks up a string and replaces `{name}` placeholders.
#[must_use]
pub fn tf(key: &str, args: &[(&str, &str)]) -> String {
    let mut s = t(key);
    for (k, v) in args {
        s = s.replace(&format!("{{{k}}}"), v);
    }
    s
}

/// Strings the browser needs (client-side validation, offline queue).
#[must_use]
pub fn client_bundle() -> String {
    let map: HashMap<&str, String> = strings()
        .iter()
        .filter(|(k, _)| k.starts_with("client."))
        .map(|(k, v)| (k.as_str(), v.clone()))
        .collect();
    serde_json::to_string(&map).unwrap_or_else(|_| "{}".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `t("…")` / `tf("…"` key used in the source exists in en.json.
    #[test]
    fn all_keys_exist() {
        let sources = [
            include_str!("ui/mod.rs"),
            include_str!("ui/pages.rs"),
            include_str!("ui/plan.rs"),
            include_str!("ui/sheets.rs"),
            include_str!("ui/accounts.rs"),
            include_str!("ui/bank.rs"),
            include_str!("bank.rs"),
            include_str!("ui/actions.rs"),
            include_str!("error.rs"),
            include_str!("export.rs"),
            include_str!("sync.rs"),
            include_str!("service.rs"),
        ];
        let mut missing = Vec::new();
        for src in sources {
            for marker in ["t(\"", "tf(\""] {
                for part in src.split(marker).skip(1) {
                    let key = part.split('"').next().unwrap_or("");
                    let looks_like_key = key.contains('.')
                        && key.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '.');
                    if looks_like_key && !strings().contains_key(key) {
                        missing.push(key.to_string());
                    }
                }
            }
        }
        missing.sort();
        missing.dedup();
        assert!(missing.is_empty(), "missing i18n keys: {missing:?}");
    }
}
