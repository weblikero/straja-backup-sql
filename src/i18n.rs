//! Explicit application messages only. User names, paths and SQL identifiers are never translated.
use std::{
    collections::HashMap,
    path::Path,
    sync::{
        OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(usize)]
pub enum Language {
    #[default]
    English,
    German,
    Spanish,
    Portuguese,
    French,
    Italian,
}
impl Language {
    pub const ALL: [Self; 6] = [
        Self::English,
        Self::German,
        Self::Spanish,
        Self::Portuguese,
        Self::French,
        Self::Italian,
    ];
    pub fn code(self) -> &'static str {
        ["en", "de", "es", "pt", "fr", "it"][self as usize]
    }
    pub fn parse(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|l| l.code() == code.trim())
    }
}
impl std::fmt::Display for Language {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(
            [
                "English",
                "Deutsch",
                "Español",
                "Português",
                "Français",
                "Italiano",
            ][*self as usize],
        )
    }
}
static LANGUAGE: AtomicUsize = AtomicUsize::new(0);
pub fn current() -> Language {
    Language::ALL[LANGUAGE.load(Ordering::Relaxed)]
}
pub fn set(language: Language) {
    LANGUAGE.store(language as usize, Ordering::Relaxed);
}

pub fn load(root: &Path) -> Language {
    let saved = crate::store::Store::open(root)
        .ok()
        .and_then(|s| s.get::<String>("settings", "language").ok());
    saved
        .as_deref()
        .and_then(Language::parse)
        .or_else(|| {
            crate::platform::resources_root()
                .ok()
                .and_then(|p| {
                    std::fs::read_to_string(p.join("default-language.txt")).ok()
                })
                .as_deref()
                .and_then(Language::parse)
        })
        .unwrap_or_default()
}
pub fn reload(root: &Path) {
    set(load(root));
}
/// Poll a saved preference using an already open connection. Initial defaults are loaded once.
pub fn reload_from(store: &crate::store::Store) {
    if let Ok(code) = store.get::<String>("settings", "language")
        && let Some(language) = Language::parse(&code)
    {
        set(language);
    }
}
pub fn save(root: &Path, language: Language) -> anyhow::Result<()> {
    crate::store::Store::open(root)?.put("settings", "language", &language.code())?;
    set(language);
    Ok(())
}
type Catalog = HashMap<String, [String; 6]>;
fn catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        serde_json::from_str(include_str!("../locales/catalog.json"))
            .expect("validated translation catalog")
    })
}
pub fn translate(language: Language, key: &str) -> &str {
    catalog()
        .get(key)
        .map(|v| v[language as usize].as_str())
        .unwrap_or(key)
}
pub fn tr(key: &'static str) -> &'static str {
    translate(current(), key)
}

/// Stable persisted statuses, phase names and object types from earlier releases.
/// Never use this function on user data or arbitrary log text.
pub fn label(value: &str) -> &str {
    static LEGACY: OnceLock<HashMap<String, String>> = OnceLock::new();
    let keys = LEGACY.get_or_init(|| {
        serde_json::from_str(include_str!("../locales/legacy-labels.json"))
            .expect("validated legacy labels")
    });
    let key = keys.get(value).map(String::as_str).unwrap_or(value);
    translate(current(), key)
}
pub fn format(key: &'static str, values: &[String]) -> String {
    format_in(current(), key, values)
}
pub fn format_in(language: Language, key: &str, values: &[String]) -> String {
    // A single pass prevents braces or message keys inside user values from being reinterpreted.
    let mut rest = translate(language, key);
    let mut result = String::new();
    while let Some((before, after)) = rest.split_once('{') {
        result.push_str(before);
        if let Some((index, tail)) = after.split_once('}')
            && let Ok(index) = index.parse::<usize>()
            && let Some(value) = values.get(index)
        {
            result.push_str(value);
            rest = tail;
        } else {
            result.push('{');
            rest = after;
        }
    }
    result.push_str(rest);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn slots(text: &str) -> Vec<usize> {
        let mut found = Vec::new();
        let mut rest = text;
        while let Some((_, after)) = rest.split_once('{') {
            let (id, tail) = after.split_once('}').expect("closed placeholder");
            found.push(id.parse().expect("numeric placeholder"));
            rest = tail;
        }
        found.sort();
        found
    }
    #[test]
    fn complete_catalog_and_safe_interpolation() {
        assert!(catalog().len() > 280);
        for (key, translations) in catalog() {
            assert_eq!(key, &translations[0]);
            for translation in translations {
                assert!(!translation.is_empty());
                assert_eq!(slots(key), slots(translation), "{key}: {translation}");
            }
        }
        let name = "Eroare {1} C:\\Données\\東京".to_owned();
        for language in Language::ALL {
            assert!(
                format_in(
                    language,
                    "StrajaBackupSQL · backup failed: {0}",
                    std::slice::from_ref(&name)
                )
                .ends_with(&name)
            );
        }
    }
    #[test]
    fn default_and_persisted_language_do_not_change_plans() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(dir.path()).unwrap();
        // Preference has its own settings key; no locale-dependent schedule or status migration.
        store.put("plan", "fixture", &"unchanged").unwrap();
        for language in Language::ALL {
            store.put("settings", "language", &language.code()).unwrap();
            assert_eq!(load(dir.path()), language);
            assert_eq!(store.get::<String>("plan", "fixture").unwrap(), "unchanged");
        }
        assert_eq!(Language::default(), Language::English);
        assert_eq!(Language::parse("unknown"), None);
    }
}
