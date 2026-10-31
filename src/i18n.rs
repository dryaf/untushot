//! Embedded catalogs: no installation paths or extra runtime packages required.
//! UI labels are translated at their call sites, never by rewriting user content.
use std::{collections::BTreeMap, sync::OnceLock};
#[allow(unused_imports)]
pub(crate) use untushot_i18n_macros::{anyhow, bail, ensure, eprintln, format, println};

fn supported(locale: &str) -> Option<&'static str> {
    let code = locale
        .split(['_', '-', '.', '@'])
        .next()?
        .to_ascii_lowercase();
    match code.as_str() {
        "de" => Some("de"),
        "en" | "c" | "posix" => Some("en"),
        "fr" => Some("fr"),
        "es" => Some("es"),
        _ => None,
    }
}
fn resolve(values: &[Option<String>]) -> &'static str {
    for value in values.iter().flatten() {
        for locale in value.split(':').filter(|s| !s.is_empty()) {
            if let Some(code) = supported(locale) {
                return code;
            }
        }
    }
    "en"
}
pub fn language() -> &'static str {
    static LANGUAGE: OnceLock<&'static str> = OnceLock::new();
    LANGUAGE.get_or_init(|| {
        resolve(&["LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"].map(|name| std::env::var(name).ok()))
    })
}
type Catalog = BTreeMap<String, BTreeMap<String, String>>;
fn catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        serde_json::from_str(include_str!("../locales/messages.json"))
            .expect("validated translation catalog")
    })
}
pub fn tr(source: &'static str) -> &'static str {
    if language() == "de" {
        return source;
    }
    catalog()
        .get(source)
        .and_then(|entry| entry.get(language()))
        .map_or(source, String::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn locale_variants_preferences_and_fallback() {
        for (locale, expected) in [
            ("de_AT.UTF-8", "de"),
            ("fr_CA", "fr"),
            ("es-MX", "es"),
            ("en_GB.UTF-8", "en"),
            ("C.UTF-8", "en"),
            ("POSIX", "en"),
        ] {
            assert_eq!(resolve(&[Some(locale.into())]), expected);
        }
        assert_eq!(
            resolve(&[Some("it:fr:en".into()), Some("de_DE".into())]),
            "fr"
        );
        assert_eq!(
            resolve(&[None, Some("es_ES".into()), Some("de_DE".into())]),
            "es"
        );
        assert_eq!(resolve(&[Some("".into()), Some("ja_JP".into())]), "en");
        assert_eq!(resolve(&[]), "en");
    }
    #[test]
    fn all_catalog_entries_have_all_translations() {
        for (source, translations) in catalog() {
            for language in ["en", "fr", "es"] {
                assert!(!translations
                    .get(language)
                    .unwrap_or_else(|| panic!("{source}: {language}"))
                    .is_empty());
            }
        }
    }
    #[test]
    fn translated_formatting_preserves_captures_precision_and_user_text() {
        let layer_count = 17;
        let result = crate::i18n::format!("Ebenen · {layer_count}");
        assert!(result.ends_with(" · 17"));
        let filename = "Projekt gespeichert: {layer_count} · Grüße.png";
        assert!(crate::i18n::format!("Projekt gespeichert: {}", filename).ends_with(filename));
        assert_eq!(crate::i18n::format!("{:.2}", 1.234), "1.23");
    }
}
