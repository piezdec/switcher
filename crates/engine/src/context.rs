//! Контекст фразы: языки последних слов. Если человек только что набрал 2–3 русских слова,
//! следующее спорное слово скорее тоже русское.

use crate::score::Params;
use layouts::Lang;
use std::collections::VecDeque;

const WINDOW: usize = 5;

#[derive(Clone, Debug, Default)]
pub struct PhraseContext {
    recent: VecDeque<Lang>,
    /// Язык предыдущего слова, если оно — обычное словарное слово этого языка (`press`).
    /// Случайный набор вроде `Nt?` фразу английской не делает.
    last_word: Option<Lang>,
}

impl PhraseContext {
    pub fn push(&mut self, lang: Lang) {
        if self.recent.len() == WINDOW {
            self.recent.pop_front();
        }
        self.recent.push_back(lang);
    }

    /// Запоминает предыдущее слово: `Some(язык)`, если оно словарное, иначе `None`.
    pub fn set_last_word(&mut self, lang: Option<Lang>) {
        self.last_word = lang;
    }

    /// Язык предыдущего слова фразы, если это настоящее слово.
    pub fn last_word(&self) -> Option<Lang> {
        self.last_word
    }

    pub fn clear(&mut self) {
        self.recent.clear();
        self.last_word = None;
    }

    /// Фора языку: 0, если он встречался реже двух раз, иначе `context_weight` за каждое вхождение,
    /// но не больше трёх. Фора заметно меньше порога, так что одна она переключение не вызовет.
    pub fn bias(&self, lang: Lang, p: &Params) -> f32 {
        let n = self.recent.iter().filter(|&&l| l == lang).count();
        if n < 2 {
            0.0
        } else {
            n.min(3) as f32 * p.context_weight
        }
    }
}
