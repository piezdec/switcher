//! Обучение на действиях пользователя. Для последовательности клавиш запоминается,
//! в каком языке её оставлять, и насколько мы в этом уверены.
//!
//! - Pause на слове, которое мы не тронули, — «переводи такое» (+1, срабатывает со второго раза).
//! - Pause сразу после нашего переключения — «не трогай» (+2, срабатывает сразу).
//!
//! Одно и то же слово в обеих раскладках — одна запись: «гига» и `ubuf` набираются одними клавишами.

use crate::keys::Stroke;
use layouts::Lang;
use std::collections::HashMap;

/// С какого счёта предпочтение начинает действовать.
pub const THRESHOLD: u8 = 2;
/// Максимальная уверенность; её получают слова, добавленные вручную.
pub const MAX_SCORE: u8 = 10;
/// Прибавка за отмену нашего переключения: действует сразу.
pub const UNDO_WEIGHT: u8 = THRESHOLD;
/// Прибавка за ручной перевод: действует со второго раза.
pub const MANUAL_WEIGHT: u8 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Язык, в котором слово надо оставить.
    pub lang: Lang,
    pub score: u8,
}

#[derive(Clone, Debug, Default)]
pub struct Learning {
    entries: HashMap<String, Entry>,
    dirty: bool,
}

/// Ключ записи: клавиши слова, как они выглядят в английской раскладке без регистра.
pub fn key(keys: &[Stroke]) -> String {
    keys.iter().filter_map(|k| layouts::render(k.vk, false, false, Lang::En)).collect()
}

/// Ключ и язык для слова, написанного так, как оно должно остаться: «гига» → (`ubuf`, RU),
/// «xfl» → (`xfl`, EN). `None`, если в слове смешаны алфавиты или есть символы не с клавиатуры.
pub fn key_for_word(word: &str) -> Option<(String, Lang)> {
    let word = word.trim();
    let cyrillic = word.chars().any(|c| matches!(c, 'а'..='я' | 'А'..='Я' | 'ё' | 'Ё'));
    let latin = word.chars().any(|c| c.is_ascii_alphabetic());
    let lang = match (cyrillic, latin) {
        (true, false) => Lang::Ru,
        (false, true) => Lang::En,
        _ => return None,
    };
    let keys: Option<Vec<Stroke>> = word
        .chars()
        .map(|c| layouts::locate(c, lang).map(|(vk, shift)| Stroke { vk, shift, caps: false }))
        .collect();
    Some((key(&keys?), lang))
}

impl Learning {
    pub fn from_entries(entries: impl IntoIterator<Item = (String, Entry)>) -> Self {
        Learning { entries: entries.into_iter().filter(|(_, e)| e.score > 0).collect(), dirty: false }
    }

    /// Выученный язык для слова, если уверенность достаточна.
    pub fn preference(&self, keys: &[Stroke]) -> Option<Lang> {
        self.entries.get(&key(keys)).filter(|e| e.score >= THRESHOLD).map(|e| e.lang)
    }

    /// Есть ли выученное «оставлять в `lang`» слово, начинающееся с этих клавиш.
    /// Нужно, чтобы раннее переключение не трогало слова, которые пользователь просил не трогать.
    pub fn keeps_prefix(&self, keys: &[Stroke], lang: Lang) -> bool {
        let prefix = key(keys);
        self.entries.iter().any(|(k, e)| e.lang == lang && e.score >= THRESHOLD && k.starts_with(&prefix))
    }

    /// Учитывает действие пользователя: слово `keys` он хочет видеть в `lang`.
    /// Противоположные действия гасят друг друга (Pause туда и сразу обратно — ноль).
    pub fn learn(&mut self, keys: &[Stroke], lang: Lang, weight: u8) {
        let k = key(keys);
        if k.is_empty() {
            return;
        }
        match self.entries.get_mut(&k) {
            Some(e) if e.lang == lang => e.score = e.score.saturating_add(weight).min(MAX_SCORE),
            Some(e) if e.score > weight => e.score -= weight,
            Some(e) if e.score == weight => {
                self.entries.remove(&k);
            }
            Some(e) => *e = Entry { lang, score: weight - e.score },
            None => {
                self.entries.insert(k, Entry { lang, score: weight });
            }
        }
        self.dirty = true;
    }

    /// Прямая запись (добавление слова вручную в настройках).
    pub fn set(&mut self, key: String, entry: Entry) {
        self.entries.insert(key, entry);
        self.dirty = true;
    }

    pub fn forget(&mut self, key: &str) {
        if self.entries.remove(key).is_some() {
            self.dirty = true;
        }
    }

    pub fn entries(&self) -> impl Iterator<Item = (&String, &Entry)> {
        self.entries.iter()
    }

    /// Были ли изменения с прошлого вызова (для сохранения на диск).
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(s: &str) -> Vec<Stroke> {
        s.chars()
            .map(|c| {
                let (vk, shift) = layouts::locate(c, Lang::En).unwrap();
                Stroke { vk, shift, caps: false }
            })
            .collect()
    }

    #[test]
    fn manual_needs_two_times() {
        let mut l = Learning::default();
        l.learn(&word("ubuf"), Lang::Ru, MANUAL_WEIGHT);
        assert_eq!(l.preference(&word("ubuf")), None);
        l.learn(&word("ubuf"), Lang::Ru, MANUAL_WEIGHT);
        assert_eq!(l.preference(&word("Ubuf")), Some(Lang::Ru));
    }

    #[test]
    fn undo_works_at_once_and_back_and_forth_cancels() {
        let mut l = Learning::default();
        l.learn(&word("xfl"), Lang::En, UNDO_WEIGHT);
        assert_eq!(l.preference(&word("xfl")), Some(Lang::En));
        l.learn(&word("xfl"), Lang::Ru, UNDO_WEIGHT);
        assert_eq!(l.preference(&word("xfl")), None);
        assert_eq!(l.entries().count(), 0);
    }

    #[test]
    fn key_for_written_word() {
        assert_eq!(key_for_word("Гига"), Some(("ubuf".into(), Lang::Ru)));
        assert_eq!(key_for_word(" xfl "), Some(("xfl".into(), Lang::En)));
        assert_eq!(key_for_word("gigaгига"), None);
        assert_eq!(key_for_word("123"), None);
    }

    #[test]
    fn dirty_flag() {
        let mut l = Learning::default();
        assert!(!l.take_dirty());
        l.learn(&word("abc"), Lang::Ru, 1);
        assert!(l.take_dirty());
        assert!(!l.take_dirty());
    }
}
