//! Символьная языковая модель (интерполированный Kneser-Ney) и её двоичный формат.
//!
//! Модель отвечает на вопрос «насколько правдоподобно это слово в языке», даже если слова нет
//! в словаре: фамилии, сленг, новые термины. Обучается `tools/trainer` на частотных списках,
//! читается ядром из встроенных в бинарник байтов.
//!
//! Формат (всё little-endian `u32`, кроме `f32` скидок):
//! ```text
//! "SWLM" version order alphabet_len alphabet[alphabet_len]
//! для n = 1..=order:  discount:f32  count  (key, count)[count]       — отсортировано по key
//! для n = 2..=order:  count  (key, total, types)[count]              — контексты длины n-1
//! ```
//! Ключ n-граммы — индексы символов, упакованные по 6 бит (0 — граница слова).

use std::fmt;

pub const MAGIC: &[u8; 4] = b"SWLM";
pub const VERSION: u32 = 1;
/// Индекс символа границы слова.
pub const BOUNDARY: u8 = 0;
/// Логарифм вероятности символа вне алфавита языка (≈ 1e-5).
pub const OOV_LOGP: f32 = -11.5;
const BITS: u32 = 6;
pub const MAX_ORDER: usize = 5;

/// Алфавит модели языка: только эти символы она умеет оценивать.
pub fn alphabet(lang: &str) -> &'static str {
    match lang {
        "ru" => "абвгдеёжзийклмнопрстуфхцчшщъыьэюя",
        "en" => "abcdefghijklmnopqrstuvwxyz'",
        _ => panic!("неизвестный язык {lang}"),
    }
}

/// Упаковка последовательности индексов в ключ.
pub fn key(symbols: &[u8]) -> u32 {
    symbols.iter().fold(0u32, |k, &s| (k << BITS) | s as u32)
}

#[derive(Debug)]
pub struct FormatError(&'static str);

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "повреждённая языковая модель: {}", self.0)
    }
}

impl std::error::Error for FormatError {}

/// Таблицы модели. Индекс внешних векторов — порядок n-граммы (элемент 0 не используется).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tables {
    pub order: usize,
    pub alphabet: Vec<char>,
    pub discount: Vec<f32>,
    /// `(ключ n-граммы, счётчик)`; для младших порядков — счётчики продолжений Kneser-Ney.
    pub grams: Vec<Vec<(u32, u32)>>,
    /// `(ключ контекста, сумма счётчиков, число разных продолжений)`.
    pub contexts: Vec<Vec<(u32, u32, u32)>>,
}

impl Tables {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut put = |v: u32| out.extend_from_slice(&v.to_le_bytes());
        put(u32::from_le_bytes(*MAGIC));
        put(VERSION);
        put(self.order as u32);
        put(self.alphabet.len() as u32);
        for &c in &self.alphabet {
            put(c as u32);
        }
        for n in 1..=self.order {
            put(self.discount[n].to_bits());
            put(self.grams[n].len() as u32);
            for &(k, c) in &self.grams[n] {
                put(k);
                put(c);
            }
        }
        for n in 2..=self.order {
            put(self.contexts[n].len() as u32);
            for &(k, t, ty) in &self.contexts[n] {
                put(k);
                put(t);
                put(ty);
            }
        }
        out
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Tables, FormatError> {
        let mut words = bytes.as_chunks::<4>().0.iter().map(|&c| u32::from_le_bytes(c));
        let mut next = || words.next().ok_or(FormatError("файл обрезан"));
        if next()? != u32::from_le_bytes(*MAGIC) {
            return Err(FormatError("не тот формат"));
        }
        if next()? != VERSION {
            return Err(FormatError("не та версия"));
        }
        let order = next()? as usize;
        if !(1..=MAX_ORDER).contains(&order) {
            return Err(FormatError("недопустимый порядок"));
        }
        let alen = next()? as usize;
        if alen >= (1 << BITS) {
            return Err(FormatError("слишком большой алфавит"));
        }
        let mut alphabet = Vec::with_capacity(alen);
        for _ in 0..alen {
            alphabet.push(char::from_u32(next()?).ok_or(FormatError("плохой символ"))?);
        }
        let mut t = Tables {
            order,
            alphabet,
            discount: vec![0.0; order + 1],
            grams: vec![Vec::new(); order + 1],
            contexts: vec![Vec::new(); order + 1],
        };
        for n in 1..=order {
            t.discount[n] = f32::from_bits(next()?);
            let count = next()? as usize;
            let mut v = Vec::with_capacity(count);
            for _ in 0..count {
                v.push((next()?, next()?));
            }
            t.grams[n] = v;
        }
        for n in 2..=order {
            let count = next()? as usize;
            let mut v = Vec::with_capacity(count);
            for _ in 0..count {
                v.push((next()?, next()?, next()?));
            }
            t.contexts[n] = v;
        }
        Ok(t)
    }
}

/// Готовая к запросам модель.
pub struct Model {
    t: Tables,
    unigram_total: f64,
}

impl Model {
    pub fn new(t: Tables) -> Model {
        let unigram_total = t.grams[1].iter().map(|&(_, c)| c as f64).sum();
        Model { t, unigram_total }
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Model, FormatError> {
        Tables::from_bytes(bytes).map(Model::new)
    }

    pub fn order(&self) -> usize {
        self.t.order
    }

    /// Индекс символа в алфавите (1-based), `None` — символ вне алфавита.
    fn symbol(&self, c: char) -> Option<u8> {
        self.t.alphabet.iter().position(|&a| a == c).map(|i| i as u8 + 1)
    }

    /// Логарифм (натуральный) вероятности слова целиком, включая границы.
    /// Слово должно быть в нижнем регистре. Символы вне алфавита стоят [`OOV_LOGP`]
    /// и обрывают контекст, как граница слова.
    pub fn word_logprob(&self, word: &str) -> f32 {
        self.logprob(word, true)
    }

    /// То же для начала слова: без вероятности конца слова.
    pub fn prefix_logprob(&self, prefix: &str) -> f32 {
        self.logprob(prefix, false)
    }

    fn logprob(&self, word: &str, with_end: bool) -> f32 {
        let mut seq = Vec::with_capacity(word.len() + 2);
        let mut oov = Vec::with_capacity(word.len() + 2);
        seq.push(BOUNDARY);
        oov.push(false);
        for c in word.chars() {
            match self.symbol(c) {
                Some(s) => {
                    seq.push(s);
                    oov.push(false);
                }
                None => {
                    seq.push(BOUNDARY);
                    oov.push(true);
                }
            }
        }
        if with_end {
            seq.push(BOUNDARY);
            oov.push(false);
        }

        let mut total = 0.0f64;
        let mut start = 0;
        for i in 1..seq.len() {
            if oov[i] {
                total += OOV_LOGP as f64;
                start = i;
                continue;
            }
            let from = start.max(i.saturating_sub(self.t.order - 1));
            total += self.prob(&seq[from..i], seq[i]).ln();
        }
        total as f32
    }

    /// P(c | h) с интерполяцией к младшим порядкам.
    fn prob(&self, h: &[u8], c: u8) -> f64 {
        let n = h.len() + 1;
        if n == 1 {
            let cnt = lookup(&self.t.grams[1], key(&[c])).unwrap_or(0) as f64;
            let vocab = self.t.alphabet.len() as f64 + 1.0;
            return (cnt + 1.0) / (self.unigram_total + vocab);
        }
        let lower = self.prob(&h[1..], c);
        let Some((total, types)) = lookup_ctx(&self.t.contexts[n], key(h)) else {
            return lower;
        };
        let mut gram = h.to_vec();
        gram.push(c);
        let cnt = lookup(&self.t.grams[n], key(&gram)).unwrap_or(0) as f64;
        let d = self.t.discount[n] as f64;
        ((cnt - d).max(0.0) + d * types as f64 * lower) / total as f64
    }
}

fn lookup(v: &[(u32, u32)], k: u32) -> Option<u32> {
    v.binary_search_by_key(&k, |e| e.0).ok().map(|i| v[i].1)
}

fn lookup_ctx(v: &[(u32, u32, u32)], k: u32) -> Option<(u32, u32)> {
    v.binary_search_by_key(&k, |e| e.0).ok().map(|i| (v[i].1, v[i].2))
}

pub mod train;

#[cfg(test)]
mod tests {
    use super::*;

    fn toy() -> Model {
        let words = [("привет", 100), ("при", 50), ("ветер", 30), ("пример", 20)];
        Model::new(train::train("ru", 4, words.iter().map(|&(w, c)| (w, c))))
    }

    #[test]
    fn roundtrip() {
        let t = train::train("ru", 3, [("мама", 3u32), ("рама", 2)].into_iter());
        assert_eq!(Tables::from_bytes(&t.to_bytes()).unwrap(), t);
    }

    #[test]
    fn seen_beats_unseen() {
        let m = toy();
        assert!(m.word_logprob("привет") > m.word_logprob("рпвите"));
        assert!(m.word_logprob("привет") > m.word_logprob("ghbdtn"));
    }

    #[test]
    fn prefix_is_likelier_than_word() {
        let m = toy();
        assert!(m.prefix_logprob("при") > m.word_logprob("при") - 1e-3);
        assert!(m.prefix_logprob("прив") > m.prefix_logprob("рпви"));
    }

    #[test]
    fn probabilities_sum_to_one() {
        let m = toy();
        let a = m.t.alphabet.len() as u8;
        for h in [&[BOUNDARY][..], &[BOUNDARY, 16], &[16, 17, 9]] {
            let s: f64 = (0..=a).map(|c| m.prob(h, c)).sum();
            assert!((s - 1.0).abs() < 1e-6, "h={h:?} sum={s}");
        }
    }
}
