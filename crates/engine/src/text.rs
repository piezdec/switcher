//! Исправления текста без смены раскладки: автозамена по сокращениям, регистр, опечатки.

use crate::keys::Stroke;
use layouts::Lang;
use std::collections::HashMap;
use std::sync::OnceLock;

/// Сокращение: набрали `trigger` и границу слова — получили `text`.
/// В тексте можно использовать макросы `{date}`, `{time}`, `{clipboard}`, `{cursor}` —
/// их раскрывает платформенный слой.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snippet {
    pub trigger: String,
    pub text: String,
}

/// Настройки текстовых исправлений.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextOptions {
    pub snippets: Vec<Snippet>,
    /// `ПРивет` → `Привет`, `пРИВЕТ` (нажат Caps Lock) → `Привет` с выключением Caps Lock.
    pub fix_case: bool,
    /// Заглавная буква в начале предложения.
    pub capitalize_sentences: bool,
    /// Частые опечатки по встроенному списку.
    pub fix_typos: bool,
}

impl TextOptions {
    /// Сокращение, которое набрано этими клавишами — в любой из раскладок.
    pub fn snippet_for(&self, renderings: &[String]) -> Option<&Snippet> {
        self.snippets.iter().find(|s| {
            let trigger = s.trigger.to_lowercase();
            !trigger.is_empty() && renderings.iter().any(|r| r.to_lowercase() == trigger)
        })
    }
}

/// Исправление регистра: новый текст и нужно ли выключить Caps Lock.
pub fn fix_case(keys: &[Stroke], text: &str) -> Option<(String, bool)> {
    let chars: Vec<char> = text.chars().collect();
    let letters = chars.iter().filter(|c| c.is_alphabetic()).count();
    if letters < 3 || letters != chars.len() {
        return None;
    }
    // Caps Lock включён случайно: первая буква набрана с Shift (вышла строчной), остальные без.
    let caps_typo = keys.iter().all(|k| k.caps) && keys[0].shift && keys[1..].iter().all(|k| !k.shift);
    if caps_typo {
        let mut out: String = chars[0].to_uppercase().collect();
        out.extend(chars[1..].iter().flat_map(|c| c.to_lowercase()));
        return Some((out, true));
    }
    // Две заглавные в начале: ПРивет → Привет (от четырёх букв, чтобы не трогать «IDs»).
    if chars.len() >= 4
        && chars[0].is_uppercase()
        && chars[1].is_uppercase()
        && chars[2..].iter().all(|c| c.is_lowercase())
    {
        let mut out = String::new();
        out.push(chars[0]);
        out.extend(chars[1].to_lowercase());
        out.extend(&chars[2..]);
        return Some((out, false));
    }
    None
}

/// Слово с заглавной первой буквой (если оно начинается со строчной буквы).
pub fn capitalize(text: &str) -> Option<String> {
    let mut chars = text.chars();
    let first = chars.next()?;
    if !first.is_lowercase() {
        return None;
    }
    Some(first.to_uppercase().chain(chars).collect())
}

const TYPOS: &str = include_str!("../data/typos.txt");

fn typos() -> &'static HashMap<String, String> {
    static MAP: OnceLock<HashMap<String, String>> = OnceLock::new();
    MAP.get_or_init(|| {
        TYPOS
            .lines()
            .filter(|l| !l.trim_start().starts_with('#'))
            .filter_map(|l| l.split_once('='))
            .map(|(wrong, right)| (wrong.trim().to_lowercase(), right.trim().to_string()))
            .collect()
    })
}

/// Исправление частой опечатки с сохранением заглавной первой буквы.
pub fn fix_typo(text: &str, _lang: Lang) -> Option<String> {
    let right = typos().get(&text.to_lowercase())?;
    if text.chars().next().is_some_and(char::is_uppercase) {
        capitalize(right).or_else(|| Some(right.clone()))
    } else {
        Some(right.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(text: &str, caps: bool) -> Vec<Stroke> {
        text.chars()
            .map(|c| {
                let lang = if c.is_ascii() { Lang::En } else { Lang::Ru };
                let (vk, shift) = layouts::locate(c, lang).unwrap();
                Stroke { vk, shift, caps }
            })
            .collect()
    }

    #[test]
    fn double_capital() {
        assert_eq!(fix_case(&keys("ПРивет", false), "ПРивет"), Some(("Привет".into(), false)));
        assert_eq!(fix_case(&keys("IDs", false), "IDs"), None);
        assert_eq!(fix_case(&keys("HTTP", false), "HTTP"), None);
        assert_eq!(fix_case(&keys("Привет", false), "Привет"), None);
    }

    #[test]
    fn caps_lock_typo() {
        // С Caps Lock: Shift+п даёт «п», остальные — заглавные.
        let mut k = keys("привет", true);
        k[0].shift = true;
        assert_eq!(fix_case(&k, "пРИВЕТ"), Some(("Привет".into(), true)));
        // Просто набранное капсом слово без Shift — не трогаем.
        assert_eq!(fix_case(&keys("привет", true), "ПРИВЕТ"), None);
    }

    #[test]
    fn typos_keep_capital() {
        assert_eq!(fix_typo("собщение", Lang::Ru).as_deref(), Some("сообщение"));
        assert_eq!(fix_typo("Teh", Lang::En).as_deref(), Some("The"));
        assert_eq!(fix_typo("сообщение", Lang::Ru), None);
    }

    #[test]
    fn snippet_in_any_layout() {
        let o = TextOptions {
            snippets: vec![Snippet { trigger: ";адр".into(), text: "Москва".into() }],
            ..Default::default()
        };
        assert!(o.snippet_for(&[";адр".into(), "$fl".into()]).is_some());
        assert!(o.snippet_for(&[";flh".into(), "жадр".into()]).is_none());
    }
}
