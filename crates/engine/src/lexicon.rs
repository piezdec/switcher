//! Встроенные в бинарник языковые модели и словари словоформ (строит `tools/trainer`).

use fst::Map;
use langmodel::Model;
use layouts::Lang;
use std::sync::OnceLock;

struct Lexicon {
    model: Model,
    dict: Map<&'static [u8]>,
}

static RU: OnceLock<Lexicon> = OnceLock::new();
static EN: OnceLock<Lexicon> = OnceLock::new();

fn load(lm: &'static [u8], dict: &'static [u8]) -> Lexicon {
    Lexicon {
        model: Model::from_bytes(lm).expect("встроенная модель повреждена"),
        dict: Map::new(dict).expect("встроенный словарь повреждён"),
    }
}

fn get(lang: Lang) -> &'static Lexicon {
    match lang {
        Lang::Ru => {
            RU.get_or_init(|| load(include_bytes!("../data/ru.lm"), include_bytes!("../data/ru.fst")))
        }
        Lang::En => {
            EN.get_or_init(|| load(include_bytes!("../data/en.lm"), include_bytes!("../data/en.fst")))
        }
    }
}

/// Загружает модели заранее, чтобы первый анализ в хуке не тратил на это время.
pub fn warm_up() {
    for lang in Lang::ALL {
        get(lang);
    }
}

pub fn model(lang: Lang) -> &'static Model {
    &get(lang).model
}

/// Частотный класс слова (⌊log2 частоты⌋), `None` — слова нет в словаре. Слово в нижнем регистре.
pub fn freq_class(lang: Lang, word: &str) -> Option<u8> {
    get(lang).dict.get(word).map(|v| v as u8)
}

/// Есть ли в словаре слово с частотным классом не ниже `min_class`, начинающееся с `prefix`.
/// Перебор ограничен; если лимит исчерпан (очень короткий частый префикс), отвечаем «да».
pub fn has_word_with_prefix(lang: Lang, prefix: &str, min_class: u8) -> bool {
    use fst::automaton::{Automaton, Str};
    use fst::{IntoStreamer, Streamer};
    const LIMIT: usize = 4096;
    let matcher = Str::new(prefix).starts_with();
    let mut stream = get(lang).dict.search(matcher).into_stream();
    let mut seen = 0;
    while let Some((_, class)) = stream.next() {
        if class as u8 >= min_class {
            return true;
        }
        seen += 1;
        if seen >= LIMIT {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dictionaries_load() {
        assert!(freq_class(Lang::Ru, "привет").unwrap() > 10);
        assert!(freq_class(Lang::En, "hello").unwrap() > 10);
        assert!(freq_class(Lang::En, "kubectl").is_some());
        assert!(freq_class(Lang::Ru, "ghbdtn").is_none());
    }
}
