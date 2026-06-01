//! Rust-side i18n for surfaces that don't live in Vue (tray menu items,
//! tooltip strings).
//!
//! Strategy: both locale JSONs are embedded at compile time via
//! `include_str!` of the same files vue-i18n imports. They're parsed
//! once on first access. The active locale is stored in a `RwLock` and
//! flipped by [`set_locale`] when the frontend writes
//! `SettingChanged { key: "language" }`.
//!
//! Templated values (e.g. tray tooltip's `"downloading {n}"`) substitute
//! `{name}` placeholders against a small `[(name, value)]` slice via
//! [`t_with`]. For non-templated keys, [`t`] returns a `String` straight
//! from the JSON.

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

use serde_json::Value;

const EN_RAW: &str = include_str!("../../frontend/src/locales/en.json");
const ID_RAW: &str = include_str!("../../frontend/src/locales/id.json");

/// Two-letter code resolved by the frontend before the value lands in
/// the `language` setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Locale {
    En,
    Id,
}

impl Locale {
    /// Resolve from the persisted setting value. `"system"` is treated
    /// as English here — the resolution happens on the frontend before
    /// the value is set to `"en"` or `"id"`. We're tolerant in case the
    /// app was launched before the frontend wrote the resolved value
    /// (first launch, `system` default).
    pub fn from_setting(raw: Option<&str>) -> Self {
        match raw {
            Some("id") => Locale::Id,
            Some("en") | Some("system") | None | Some(_) => Locale::En,
        }
    }
}

fn parsed(locale: Locale) -> &'static Value {
    static EN: OnceLock<Value> = OnceLock::new();
    static ID: OnceLock<Value> = OnceLock::new();
    match locale {
        Locale::En => EN.get_or_init(|| {
            serde_json::from_str(EN_RAW).expect("frontend/src/locales/en.json is not valid JSON")
        }),
        Locale::Id => ID.get_or_init(|| {
            serde_json::from_str(ID_RAW).expect("frontend/src/locales/id.json is not valid JSON")
        }),
    }
}

fn active() -> &'static RwLock<Locale> {
    static SLOT: OnceLock<RwLock<Locale>> = OnceLock::new();
    SLOT.get_or_init(|| RwLock::new(Locale::En))
}

/// Replace the active locale. Subsequent [`t`] / [`t_with`] calls reflect
/// the new value. The tray re-reads the menu strings on the matching
/// `SettingChanged` event.
pub fn set_locale(locale: Locale) {
    *active().write().expect("i18n lock poisoned") = locale;
}

/// Current locale, suitable for read-only debug or fallback checks.
pub fn current() -> Locale {
    *active().read().expect("i18n lock poisoned")
}

fn lookup(namespace: &str, key: &str, locale: Locale) -> Option<String> {
    parsed(locale)
        .get(namespace)
        .and_then(|ns| ns.get(key))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// Look up a translation. Falls back to English if the requested locale
/// is missing the key, then to the literal `key` if both locales lack it
/// (so the caller still sees *something* on the screen rather than an
/// empty string).
pub fn t(namespace: &str, key: &str) -> String {
    let locale = current();
    if let Some(s) = lookup(namespace, key, locale) {
        return s;
    }
    if locale != Locale::En {
        if let Some(s) = lookup(namespace, key, Locale::En) {
            return s;
        }
    }
    format!("{namespace}.{key}")
}

/// Look up a translation and substitute `{placeholder}` tokens. Unknown
/// placeholders in the template are left as-is so the caller can see the
/// drift instead of a blank value.
pub fn t_with(namespace: &str, key: &str, params: &[(&str, &dyn std::fmt::Display)]) -> String {
    let template = t(namespace, key);
    let mut params_map: HashMap<&str, String> = HashMap::with_capacity(params.len());
    for (name, value) in params {
        params_map.insert(name, value.to_string());
    }
    interpolate(&template, &params_map)
}

fn interpolate(template: &str, params: &HashMap<&str, String>) -> String {
    let mut out = String::with_capacity(template.len());
    let mut chars = template.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c != '{' {
            out.push(c);
            continue;
        }
        // Scan to the closing brace. If there isn't one, treat the `{`
        // as a literal so malformed templates don't swallow the rest of
        // the string.
        let rest = &template[i + 1..];
        let Some(end) = rest.find('}') else {
            out.push(c);
            continue;
        };
        let name = &rest[..end];
        match params.get(name) {
            Some(value) => out.push_str(value),
            None => {
                out.push('{');
                out.push_str(name);
                out.push('}');
            }
        }
        // Advance the iterator past the consumed `name}` segment.
        for _ in 0..(end + 1) {
            chars.next();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_from_setting() {
        assert_eq!(Locale::from_setting(Some("en")), Locale::En);
        assert_eq!(Locale::from_setting(Some("id")), Locale::Id);
        assert_eq!(Locale::from_setting(Some("system")), Locale::En);
        assert_eq!(Locale::from_setting(None), Locale::En);
    }

    // The locale is global state shared across the whole process, so
    // tests that flip it must serialize. Cargo runs unit tests in
    // parallel by default; the easiest way to keep them honest is to
    // hold a guard mutex for the duration of each locale-sensitive
    // test.
    fn locale_lock() -> std::sync::MutexGuard<'static, ()> {
        static MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());
        MUTEX.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn tray_strings_resolve_in_english() {
        let _g = locale_lock();
        set_locale(Locale::En);
        assert_eq!(t("tray", "menuShow"), "Show window");
        assert_eq!(t("tray", "menuQuit"), "Quit");
        assert_eq!(t("tray", "tooltipIdle"), "Unduhin — idle");
    }

    #[test]
    fn tray_strings_resolve_in_indonesian() {
        let _g = locale_lock();
        set_locale(Locale::Id);
        assert_eq!(t("tray", "menuShow"), "Tampilkan jendela");
        assert_eq!(t("tray", "menuQuit"), "Keluar");
        // Reset so other tests aren't affected.
        set_locale(Locale::En);
    }

    #[test]
    fn missing_key_falls_back_to_key_path() {
        let _g = locale_lock();
        set_locale(Locale::En);
        assert_eq!(t("tray", "doesNotExist"), "tray.doesNotExist");
    }

    #[test]
    fn placeholders_interpolate() {
        let _g = locale_lock();
        set_locale(Locale::En);
        let out = t_with("tray", "tooltipDownloading", &[("n", &3u32)]);
        assert_eq!(out, "Unduhin — downloading 3");
    }

    #[test]
    fn unknown_placeholder_is_left_literal() {
        let map: HashMap<&str, String> = HashMap::new();
        assert_eq!(interpolate("hello {name}", &map), "hello {name}");
    }

    #[test]
    fn empty_template_returns_empty() {
        let map: HashMap<&str, String> = HashMap::new();
        assert_eq!(interpolate("", &map), "");
    }
}
