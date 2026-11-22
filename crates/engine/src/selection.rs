//! Перевод выделенного текста. Переводятся только слова, набранные не в той раскладке:
//! правильный текст, английские названия и термины внутри фразы остаются как есть.
//! Если правильным выглядит всё — человек всё равно просил перевести, переводим целиком.

use crate::context::PhraseContext;
use crate::detect;
use crate::keys::Stroke;
use crate::lexicon;
use crate::score::Params;
use layouts::Lang;

/// Слово выделения: где оно в тексте и что с ним делать.
struct Token {
    start: usize,
    end: usize,
    /// Раскладка, в которой слово набрано (по его буквам).
    from: Lang,
    letters: usize,
    convert: bool,
}

/// Результат: новый текст и раскладка, на которую стоит переключиться (`None` — не трогать).
pub fn convert(text: &str) -> (String, Option<Lang>) {
    let p = Params::default();
    let mut tokens = tokenize(text);
    let mut ctx = PhraseContext::default();
    for t in tokens.iter_mut() {
        let word = &text[t.start..t.end];
        t.convert = should_convert(word, t.from, &ctx, &p);
        // Контекст — как при наборе: язык слова, каким оно будет на экране.
        let shown = if t.convert { t.from.other() } else { t.from };
        if t.letters >= 2 {
            ctx.push(shown);
        }
        let core = word.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase();
        let known = lexicon::freq_class(t.from, &core).is_some_and(|c| c >= p.known_class);
        ctx.set_last_word((known || t.convert).then_some(shown));
    }
    // Короткие слова («в», «и», `d`, `b`) сами по себе не решить: переводим, если в выделении
    // переводится хоть одно длинное слово, набранное в той же раскладке.
    let converted_from =
        |lang: Lang| tokens.iter().any(|t| t.convert && t.from == lang && t.letters > 2);
    let (ru, en) = (converted_from(Lang::Ru), converted_from(Lang::En));
    for t in tokens.iter_mut().filter(|t| t.letters <= 2) {
        t.convert |= match t.from {
            Lang::Ru => ru,
            Lang::En => en,
        };
    }
    if !tokens.iter().any(|t| t.convert) {
        // Ничего не выглядит ошибкой — переводим всё в сторону преобладающих букв.
        let (cyr, lat) = tokens.iter().fold((0, 0), |(c, l), t| match t.from {
            Lang::Ru => (c + t.letters, l),
            Lang::En => (c, l + t.letters),
        });
        if cyr + lat == 0 {
            return (text.to_string(), None);
        }
        let from = if cyr > lat { Lang::Ru } else { Lang::En };
        for t in tokens.iter_mut() {
            t.convert = t.from == from;
        }
    }

    let mut out = String::with_capacity(text.len());
    let mut pos = 0;
    let (mut to_ru, mut to_en) = (0, 0);
    for t in &tokens {
        out.push_str(&text[pos..t.start]);
        let word = &text[t.start..t.end];
        if t.convert {
            out.push_str(&layouts::convert_str(word, t.from, t.from.other()));
            match t.from.other() {
                Lang::Ru => to_ru += t.letters,
                Lang::En => to_en += t.letters,
            }
        } else {
            out.push_str(word);
        }
        pos = t.end;
    }
    out.push_str(&text[pos..]);
    let switch_to = match (to_ru, to_en) {
        (0, 0) => None,
        (ru, en) if ru >= en => Some(Lang::Ru),
        _ => Some(Lang::En),
    };
    (out, switch_to)
}

/// Слова — куски между пробелами, в которых есть буквы; раскладка — по преобладающим буквам.
fn tokenize(text: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut start = None;
    let push = |s: usize, e: usize, tokens: &mut Vec<Token>| {
        let word = &text[s..e];
        let cyr = word.chars().filter(|&c| is_cyrillic(c)).count();
        let lat = word.chars().filter(char::is_ascii_alphabetic).count();
        if cyr + lat > 0 {
            let from = if cyr > lat { Lang::Ru } else { Lang::En };
            tokens.push(Token { start: s, end: e, from, letters: cyr + lat, convert: false });
        }
    };
    for (i, c) in text.char_indices() {
        match (c.is_whitespace(), start) {
            (true, Some(s)) => {
                push(s, i, &mut tokens);
                start = None;
            }
            (false, None) => start = Some(i),
            _ => {}
        }
    }
    if let Some(s) = start {
        push(s, text.len(), &mut tokens);
    }
    tokens
}

fn is_cyrillic(c: char) -> bool {
    matches!(c, 'а'..='я' | 'А'..='Я' | 'ё' | 'Ё')
}

/// Набрано ли слово не в той раскладке. То же решение, что при наборе, плюс случай,
/// когда набранное — не слово, а в другой раскладке получается словарное слово.
fn should_convert(word: &str, from: Lang, ctx: &PhraseContext, p: &Params) -> bool {
    let Some(keys) = word
        .chars()
        .map(|c| layouts::locate(c, from).map(|(vk, shift)| Stroke { vk, shift, caps: false }))
        .collect::<Option<Vec<_>>>()
    else {
        // Символы, которых нет на клавиатуре этой раскладки (`№`, `€`, другая азбука): не трогаем.
        return false;
    };
    if detect::decide(&keys, from, ctx, p).switch_to.is_some() {
        return true;
    }
    let core = |s: &str| s.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase();
    let typed_known = lexicon::freq_class(from, &core(word)).is_some();
    let other = layouts::convert_str(word, from, from.other());
    let other_known =
        lexicon::freq_class(from.other(), &core(&other)).is_some_and(|c| c >= p.known_class);
    !typed_known && other_known && !detect::is_protected(word)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conv(s: &str) -> String {
        convert(s).0
    }

    #[test]
    fn whole_wrong_phrase() {
        assert_eq!(convert("ghbdtn rfr ltkf?"), ("привет как дела,".to_string(), Some(Lang::Ru)));
        assert_eq!(conv("руддщ цщкдв"), "hello world");
    }

    #[test]
    fn correct_words_inside_stay() {
        assert_eq!(conv("ecnfyjdbk Windows b GitHub"), "установил Windows и GitHub");
        assert_eq!(conv("Привет ghbdtn"), "Привет привет");
    }

    #[test]
    fn mixed_directions() {
        assert_eq!(conv("ghbdtn руддщ"), "привет hello");
    }

    #[test]
    fn correct_text_is_converted_anyway() {
        assert_eq!(convert("привет"), ("ghbdtn".to_string(), Some(Lang::En)));
        assert_eq!(conv("hello world"), "руддщ цщкдв");
    }

    #[test]
    fn layout_and_spacing_kept() {
        assert_eq!(conv("  ghbdtn?\n\tvbh  "), "  привет,\n\tмир  ");
        assert_eq!(convert("123 !!"), ("123 !!".to_string(), None));
    }
}
