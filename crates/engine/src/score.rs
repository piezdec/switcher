//! Балл правдоподобия слова в языке: логарифм вероятности по символьной модели плюс бонус
//! за словарное слово. Баллы двух вариантов слова сравнимы, потому что у них одинаковая
//! длина: каждая клавиша даёт ровно один символ в любой раскладке.

use crate::lexicon;
use layouts::Lang;

/// Настраиваемые коэффициенты. Подбираются по `tools/eval`, менять — только сверяясь с ним.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    /// Насколько (в натах) другой вариант должен быть правдоподобнее набранного.
    pub margin: f32,
    /// Бонус словарному слову: `dict_slope · (частотный класс − dict_floor)`, не меньше нуля.
    /// Редкие слова (опечатки и мусор из субтитров) почти ничего не получают.
    pub dict_floor: f32,
    pub dict_slope: f32,
    /// Цена каждого краевого знака препинания («hello,» → «hello» + запятая).
    pub edge_logp: f32,
    /// Фора языку за каждое из последних слов на нём (начиная со второго, максимум три).
    pub context_weight: f32,
    /// Набранное словарное слово с частотным классом не ниже этого не переводится:
    /// если человек набрал настоящее слово, скорее всего он его и хотел.
    pub known_class: u8,
    /// Раннее переключение посреди слова, не дожидаясь пробела.
    pub early: bool,
    /// Насколько больше обычного порога должен быть перевес для раннего переключения.
    pub early_extra: f32,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            margin: 4.0,
            dict_floor: 4.0,
            dict_slope: 0.5,
            edge_logp: -3.0,
            context_weight: 0.75,
            known_class: 10,
            early: true,
            early_extra: 6.0,
        }
    }
}

/// Слагаемые балла раздельно — чтобы решение можно было объяснить пользователю.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScoreParts {
    /// ln P(слово) по символьной модели.
    pub lm: f32,
    /// Бонус за словарное слово.
    pub dict: f32,
    /// Штраф за знаки по краям.
    pub edge: f32,
    /// Частотный класс словарного слова.
    pub class: Option<u8>,
    /// По краям есть редкие знаки (`` ` ``, `[`, `<`…) — скорее всего это буквы другой раскладки.
    pub odd_edges: bool,
}

impl ScoreParts {
    pub fn total(&self) -> f32 {
        self.lm + self.dict + self.edge
    }
}

pub fn score(word: &str, lang: Lang, p: &Params) -> ScoreParts {
    let lower = word.to_lowercase();
    let core = lower.trim_matches(|c: char| !c.is_alphanumeric());
    let (edges, odd_edges) = edge_cost(&lower, core);
    // Английские сокращения (don't, let's, I'm): в корпусе они разбиты на части, поэтому
    // оцениваем слово без апострофа, а сам апостроф стоит как краевой знак.
    let (scored, class, edges) = match contraction(core, lang) {
        Some((head, joined)) => (joined, lexicon::freq_class(lang, head), edges + 1.0),
        None => (core.to_string(), lexicon::freq_class(lang, core), edges),
    };
    ScoreParts {
        lm: lexicon::model(lang).word_logprob(&scored),
        dict: class.map_or(0.0, |c| (p.dict_slope * (c as f32 - p.dict_floor)).max(0.0)),
        edge: edges * p.edge_logp,
        class,
        odd_edges,
    }
}

/// Цена знаков по краям слова в единицах `edge_logp`: обычная пунктуация — 1,
/// редкие у слова знаки (`<`, `` ` ``, `[`, `~`…) — 3. Они-то и бывают русскими буквами.
/// Второе значение — нашлись ли такие редкие знаки.
fn edge_cost(word: &str, core: &str) -> (f32, bool) {
    let start = word.find(core).unwrap_or(0);
    let edges = word[..start].chars().chain(word[start + core.len()..].chars());
    edges.fold((0.0, false), |(cost, odd), c| {
        if crate::detect::ORDINARY_EDGE.contains(c) {
            (cost + 1.0, odd)
        } else {
            (cost + 3.0, true)
        }
    })
}

/// «don't» → («don», «dont»): голова для словаря и слово без апострофа для модели.
/// `None`, если это не сокращение.
fn contraction(core: &str, lang: Lang) -> Option<(&str, String)> {
    if lang != Lang::En {
        return None;
    }
    let (head, tail) = core.split_once('\'')?;
    let ok = !head.is_empty()
        && head.chars().all(|c| c.is_ascii_alphabetic())
        && matches!(tail, "t" | "s" | "re" | "ll" | "ve" | "d" | "m");
    ok.then(|| (head, format!("{head}{tail}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn total(w: &str, lang: Lang) -> f32 {
        score(w, lang, &Params::default()).total()
    }

    #[test]
    fn russian_word_beats_its_latin_typo() {
        let (ru, en) = (total("привет", Lang::Ru), total("ghbdtn", Lang::En));
        assert!(ru - en > 15.0, "ru={ru} en={en}");
    }

    #[test]
    fn english_word_beats_its_cyrillic_typo() {
        let (en, ru) = (total("hello", Lang::En), total("руддщ", Lang::Ru));
        assert!(en - ru > 10.0, "en={en} ru={ru}");
    }

    #[test]
    fn unknown_but_plausible_words_score_well() {
        // Слов нет в словаре, но модель их узнаёт по устройству.
        assert!(total("перекукарекал", Lang::Ru) > total("gthtreratrfk", Lang::En));
        assert!(total("blorptastic", Lang::En) > total("иддщкзефыешс", Lang::Ru));
    }

    #[test]
    fn junk_inside_latin_word_is_penalised() {
        assert!(total("nfr;t", Lang::En) < total("также", Lang::Ru) - 15.0);
    }
}
