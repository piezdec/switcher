//! Транслитерация: «Привет» ⇄ «Privet». Направление определяется по преобладающему алфавиту.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Style {
    /// Как в загранпаспорте (ICAO 9303): щ → shch, ю → iu, я → ia, й → i.
    Passport,
    /// Как обычно пишут в интернете: щ → sch, ю → yu, я → ya, й → y, ь → '.
    #[default]
    Simple,
}

#[rustfmt::skip]
const PASSPORT: &[(char, &str)] = &[
    ('а', "a"), ('б', "b"), ('в', "v"), ('г', "g"), ('д', "d"), ('е', "e"), ('ё', "e"),
    ('ж', "zh"), ('з', "z"), ('и', "i"), ('й', "i"), ('к', "k"), ('л', "l"), ('м', "m"),
    ('н', "n"), ('о', "o"), ('п', "p"), ('р', "r"), ('с', "s"), ('т', "t"), ('у', "u"),
    ('ф', "f"), ('х', "kh"), ('ц', "ts"), ('ч', "ch"), ('ш', "sh"), ('щ', "shch"),
    ('ъ', "ie"), ('ы', "y"), ('ь', ""), ('э', "e"), ('ю', "iu"), ('я', "ia"),
];

#[rustfmt::skip]
const SIMPLE: &[(char, &str)] = &[
    ('а', "a"), ('б', "b"), ('в', "v"), ('г', "g"), ('д', "d"), ('е', "e"), ('ё', "yo"),
    ('ж', "zh"), ('з', "z"), ('и', "i"), ('й', "y"), ('к', "k"), ('л', "l"), ('м', "m"),
    ('н', "n"), ('о', "o"), ('п', "p"), ('р', "r"), ('с', "s"), ('т', "t"), ('у', "u"),
    ('ф', "f"), ('х', "kh"), ('ц', "ts"), ('ч', "ch"), ('ш', "sh"), ('щ', "sch"),
    ('ъ', "\""), ('ы', "y"), ('ь', "'"), ('э', "e"), ('ю', "yu"), ('я', "ya"),
];

/// Обратное направление: сначала длинные сочетания. Понимает обе схемы.
#[rustfmt::skip]
const TO_CYRILLIC: &[(&str, char)] = &[
    ("shch", 'щ'), ("sch", 'щ'), ("zh", 'ж'), ("kh", 'х'), ("ts", 'ц'), ("ch", 'ч'),
    ("sh", 'ш'), ("yu", 'ю'), ("iu", 'ю'), ("ya", 'я'), ("ia", 'я'), ("yo", 'ё'), ("ye", 'е'),
    ("a", 'а'), ("b", 'б'), ("v", 'в'), ("g", 'г'), ("d", 'д'), ("e", 'е'), ("z", 'з'),
    ("i", 'и'), ("j", 'й'), ("k", 'к'), ("l", 'л'), ("m", 'м'), ("n", 'н'), ("o", 'о'),
    ("p", 'п'), ("r", 'р'), ("s", 'с'), ("t", 'т'), ("u", 'у'), ("f", 'ф'), ("h", 'х'),
    ("c", 'ц'), ("y", 'ы'), ("x", 'х'), ("w", 'в'), ("q", 'к'), ("'", 'ь'),
];

fn is_cyrillic(c: char) -> bool {
    matches!(c, 'а'..='я' | 'А'..='Я' | 'ё' | 'Ё')
}

/// Транслитерирует текст в другую сторону: кириллицу в латиницу и наоборот.
pub fn transliterate(text: &str, style: Style) -> String {
    let cyrillic = text.chars().filter(|&c| is_cyrillic(c)).count();
    let latin = text.chars().filter(char::is_ascii_alphabetic).count();
    if cyrillic >= latin {
        to_latin(text, style)
    } else {
        to_cyrillic(text)
    }
}

pub fn to_latin(text: &str, style: Style) -> String {
    let table = match style {
        Style::Passport => PASSPORT,
        Style::Simple => SIMPLE,
    };
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len() * 2);
    for (i, &c) in chars.iter().enumerate() {
        let lower = c.to_lowercase().next().unwrap_or(c);
        let Some(&(_, lat)) = table.iter().find(|(ru, _)| *ru == lower) else {
            out.push(c);
            continue;
        };
        if c.is_uppercase() {
            // «Щука» → «Shchuka», но «ЩУКА» → «SHCHUKA»: смотрим на соседнюю букву.
            let next_upper = chars.get(i + 1).is_some_and(|n| n.is_uppercase());
            let prev_upper = i > 0 && chars[i - 1].is_uppercase();
            if next_upper || (prev_upper && chars.get(i + 1).is_none_or(|n| !n.is_alphabetic())) {
                out.push_str(&lat.to_uppercase());
            } else {
                let mut it = lat.chars();
                if let Some(first) = it.next() {
                    out.extend(first.to_uppercase());
                    out.extend(it);
                }
            }
        } else {
            out.push_str(lat);
        }
    }
    out
}

pub fn to_cyrillic(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let matched = TO_CYRILLIC.iter().find(|(lat, _)| {
            let n = lat.chars().count();
            i + n <= chars.len() && chars[i..i + n].iter().flat_map(|c| c.to_lowercase()).eq(lat.chars())
        });
        match matched {
            Some(&(lat, ru)) => {
                let upper = chars[i].is_uppercase();
                if upper {
                    out.extend(ru.to_uppercase());
                } else {
                    out.push(ru);
                }
                i += lat.chars().count();
            }
            None => {
                out.push(chars[i]);
                i += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn russian_to_latin() {
        assert_eq!(to_latin("Привет, мир!", Style::Simple), "Privet, mir!");
        assert_eq!(to_latin("Щука и Юля", Style::Simple), "Schuka i Yulya");
        assert_eq!(to_latin("Щука и Юля", Style::Passport), "Shchuka i Iulia");
        assert_eq!(to_latin("ЖУК", Style::Simple), "ZHUK");
        assert_eq!(to_latin("Объявление", Style::Passport), "Obieiavlenie");
    }

    #[test]
    fn latin_to_russian() {
        assert_eq!(to_cyrillic("Privet, mir!"), "Привет, мир!");
        assert_eq!(to_cyrillic("Shchuka"), "Щука");
        assert_eq!(to_cyrillic("Schuka i Yulya"), "Щука и Юля");
        assert_eq!(to_cyrillic("zhuk"), "жук");
    }

    #[test]
    fn direction_by_alphabet() {
        assert_eq!(transliterate("Привет", Style::Simple), "Privet");
        assert_eq!(transliterate("Privet", Style::Simple), "Привет");
    }
}
