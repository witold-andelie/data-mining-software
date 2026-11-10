//! English source keys with a compiled-in Simplified Chinese translation table.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{OnceLock, RwLock};

pub const TRANSLATIONS: &str = include_str!("../i18n/zh-Hans.toml");

/// Saved preference. The library itself always starts in English.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    English,
    SimplifiedChinese,
    #[default]
    #[serde(other)]
    System,
}

static CHINESE: AtomicBool = AtomicBool::new(false);
static SYSTEM_CHINESE: AtomicBool = AtomicBool::new(false);
static TEST_TRANSLATIONS: RwLock<BTreeMap<&'static str, &'static str>> =
    RwLock::new(BTreeMap::new());

/// Called by the application at startup, before applying the saved preference.
pub fn set_system_language(preferred: Option<&str>) {
    SYSTEM_CHINESE.store(
        preferred.is_some_and(|s| s.starts_with("zh")),
        Ordering::Relaxed,
    );
}

pub fn set_language(language: Language) {
    CHINESE.store(
        match language {
            Language::System => SYSTEM_CHINESE.load(Ordering::Relaxed),
            Language::English => false,
            Language::SimplifiedChinese => true,
        },
        Ordering::Relaxed,
    );
}

/// The resolved language (also used to invalidate native menus).
pub fn language() -> Language {
    if CHINESE.load(Ordering::Relaxed) {
        Language::SimplifiedChinese
    } else {
        Language::English
    }
}

pub fn tr(source: &'static str) -> &'static str {
    if language() == Language::English {
        return source;
    }
    if let Some(value) = TEST_TRANSLATIONS
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(source)
        .copied()
        .filter(|s| !s.is_empty())
    {
        return value;
    }
    static TABLE: OnceLock<Result<BTreeMap<String, String>, String>> = OnceLock::new();
    TABLE
        .get_or_init(|| parse_table(TRANSLATIONS))
        .as_ref()
        .ok()
        .and_then(|table| table.get(source))
        .filter(|value| !value.is_empty())
        .map_or(source, String::as_str)
}

/// Replaces placeholders in a single pass; substituted user text is never interpreted.
pub fn tr_fmt(source: &'static str, args: &[(&str, &str)]) -> String {
    let mut rest = tr(source);
    let mut result = String::new();
    while let Some(start) = rest.find('{') {
        let (prefix, tail) = rest.split_at(start);
        result.push_str(prefix);
        let Some(end) = tail.find('}') else {
            rest = tail;
            break;
        };
        let after = end.saturating_add(1);
        let name = tail.get(1..end).unwrap_or_default();
        let token = tail.get(..after).unwrap_or_default();
        result.push_str(
            args.iter()
                .find(|(key, _)| *key == name)
                .map_or(token, |(_, v)| *v),
        );
        rest = tail.get(after..).unwrap_or_default();
    }
    result.push_str(rest);
    result
}

/// Integration tests can exercise live language changes before WP7 supplies translations.
/// Callers must serialize tests that change the process language and clear overrides afterward.
#[doc(hidden)]
pub fn set_translation_for_test(source: &'static str, value: Option<&'static str>) {
    let mut table = TEST_TRANSLATIONS.write().unwrap_or_else(|e| e.into_inner());
    if let Some(value) = value {
        table.insert(source, value);
    } else {
        table.remove(source);
    }
}

/// The deliberately small table format: one TOML basic-string key/value pair per line.
/// Supports TOML escapes, comments and Unicode without adding a runtime dependency.
pub fn parse_table(text: &str) -> Result<BTreeMap<String, String>, String> {
    let mut table = BTreeMap::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let parse = || -> Option<(String, String)> {
            let (key, rest) = quoted(line)?;
            let rest = rest.trim_start().strip_prefix('=')?.trim_start();
            let (value, rest) = quoted(rest)?;
            let rest = rest.trim_start();
            (rest.is_empty() || rest.starts_with('#')).then_some((key, value))
        };
        let (key, value) = parse()
            .ok_or_else(|| format!("Invalid translation line {}", index.saturating_add(1)))?;
        if table.insert(key, value).is_some() {
            return Err(format!(
                "Duplicate translation on line {}",
                index.saturating_add(1)
            ));
        }
    }
    Ok(table)
}

fn quoted(text: &str) -> Option<(String, &str)> {
    let mut chars = text.strip_prefix('"')?.chars();
    let mut value = String::new();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some((value, chars.as_str())),
            '\\' => value.push(match chars.next()? {
                '"' => '"',
                '\\' => '\\',
                'b' => '\u{8}',
                't' => '\t',
                'n' => '\n',
                'f' => '\u{c}',
                'r' => '\r',
                c @ ('u' | 'U') => {
                    let mut code = 0u32;
                    for _ in 0..if c == 'u' { 4 } else { 8 } {
                        code = code
                            .checked_mul(16)?
                            .checked_add(chars.next()?.to_digit(16)?)?;
                    }
                    char::from_u32(code)?
                }
                _ => return None,
            }),
            c if c.is_control() && c != '\t' => return None,
            c => value.push(c),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn i18n_table_parses_and_rejects_malformed_or_duplicate_entries() {
        assert!(parse_table(TRANSLATIONS).is_ok());
        let table = parse_table("\"a\\\"b\" = \"中文\\n\\u4e2d\\U00006587\" # comment").unwrap();
        assert_eq!(table.get("a\"b").map(String::as_str), Some("中文\n中文"));
        for text in [
            "\"a\" = 3",
            "\"a\" = \"\\uD800\"",
            "\"a\" = \"b\" trailing",
            "\"a\" = \"\"\n\"a\" = \"\"",
        ] {
            assert!(parse_table(text).is_err(), "{text}");
        }
    }
}
