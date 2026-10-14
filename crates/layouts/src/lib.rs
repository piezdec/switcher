//! Физическая модель клавиатуры: какой символ даёт клавиша (виртуальный код + Shift + Caps Lock)
//! в каждом из поддерживаемых языков.
//!
//! Раскладки описаны данными, а не кодом: чтобы добавить язык, достаточно ещё одной колонки
//! в таблице [`KEYS`] и варианта в [`Lang`].

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Lang {
    En,
    Ru,
}

impl Lang {
    pub const ALL: [Lang; 2] = [Lang::En, Lang::Ru];

    /// Парный язык (пока языков два, переключение — это всегда «на другой»).
    pub fn other(self) -> Lang {
        match self {
            Lang::En => Lang::Ru,
            Lang::Ru => Lang::En,
        }
    }

    /// Короткая подпись для значка и индикатора.
    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "EN",
            Lang::Ru => "RU",
        }
    }

    /// Первичный идентификатор языка Windows (PRIMARYLANGID).
    pub fn primary_lang_id(self) -> u16 {
        match self {
            Lang::En => 0x09,
            Lang::Ru => 0x19,
        }
    }

    pub fn from_primary_lang_id(id: u16) -> Option<Lang> {
        Lang::ALL.into_iter().find(|l| l.primary_lang_id() == id)
    }

    fn column(self) -> usize {
        match self {
            Lang::En => 0,
            Lang::Ru => 1,
        }
    }
}

impl fmt::Display for Lang {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

/// Описание одной физической клавиши: `[без Shift, с Shift]` для каждого языка.
struct KeyDef {
    vk: u16,
    chars: [[char; 2]; 2],
}

const fn k(vk: u16, en: [char; 2], ru: [char; 2]) -> KeyDef {
    KeyDef { vk, chars: [en, ru] }
}

/// Клавиши, дающие печатный символ в обеих раскладках (ЙЦУКЕН ↔ QWERTY).
#[rustfmt::skip]
const KEYS: &[KeyDef] = &[
    // Буквы A–Z
    k(0x41, ['a', 'A'], ['ф', 'Ф']), k(0x42, ['b', 'B'], ['и', 'И']),
    k(0x43, ['c', 'C'], ['с', 'С']), k(0x44, ['d', 'D'], ['в', 'В']),
    k(0x45, ['e', 'E'], ['у', 'У']), k(0x46, ['f', 'F'], ['а', 'А']),
    k(0x47, ['g', 'G'], ['п', 'П']), k(0x48, ['h', 'H'], ['р', 'Р']),
    k(0x49, ['i', 'I'], ['ш', 'Ш']), k(0x4A, ['j', 'J'], ['о', 'О']),
    k(0x4B, ['k', 'K'], ['л', 'Л']), k(0x4C, ['l', 'L'], ['д', 'Д']),
    k(0x4D, ['m', 'M'], ['ь', 'Ь']), k(0x4E, ['n', 'N'], ['т', 'Т']),
    k(0x4F, ['o', 'O'], ['щ', 'Щ']), k(0x50, ['p', 'P'], ['з', 'З']),
    k(0x51, ['q', 'Q'], ['й', 'Й']), k(0x52, ['r', 'R'], ['к', 'К']),
    k(0x53, ['s', 'S'], ['ы', 'Ы']), k(0x54, ['t', 'T'], ['е', 'Е']),
    k(0x55, ['u', 'U'], ['г', 'Г']), k(0x56, ['v', 'V'], ['м', 'М']),
    k(0x57, ['w', 'W'], ['ц', 'Ц']), k(0x58, ['x', 'X'], ['ч', 'Ч']),
    k(0x59, ['y', 'Y'], ['н', 'Н']), k(0x5A, ['z', 'Z'], ['я', 'Я']),
    // Знаки, которые в русской раскладке — буквы
    k(0xC0, ['`', '~'], ['ё', 'Ё']),  // VK_OEM_3
    k(0xDB, ['[', '{'], ['х', 'Х']),  // VK_OEM_4
    k(0xDD, [']', '}'], ['ъ', 'Ъ']),  // VK_OEM_6
    k(0xBA, [';', ':'], ['ж', 'Ж']),  // VK_OEM_1
    k(0xDE, ['\'', '"'], ['э', 'Э']), // VK_OEM_7
    k(0xBC, [',', '<'], ['б', 'Б']),  // VK_OEM_COMMA
    k(0xBE, ['.', '>'], ['ю', 'Ю']),  // VK_OEM_PERIOD
    // Знаки в обеих раскладках
    k(0xBF, ['/', '?'], ['.', ',']),  // VK_OEM_2
    k(0xDC, ['\\', '|'], ['\\', '/']), // VK_OEM_5
    k(0xE2, ['\\', '|'], ['\\', '/']), // VK_OEM_102 (клавиша у левого Shift)
    k(0xBD, ['-', '_'], ['-', '_']),  // VK_OEM_MINUS
    k(0xBB, ['=', '+'], ['=', '+']),  // VK_OEM_PLUS
    // Цифровой ряд
    k(0x30, ['0', ')'], ['0', ')']), k(0x31, ['1', '!'], ['1', '!']),
    k(0x32, ['2', '@'], ['2', '"']), k(0x33, ['3', '#'], ['3', '№']),
    k(0x34, ['4', '$'], ['4', ';']), k(0x35, ['5', '%'], ['5', '%']),
    k(0x36, ['6', '^'], ['6', ':']), k(0x37, ['7', '&'], ['7', '?']),
    k(0x38, ['8', '*'], ['8', '*']), k(0x39, ['9', '('], ['9', '(']),
    k(0x20, [' ', ' '], [' ', ' ']),  // VK_SPACE
];

fn def(vk: u16) -> Option<&'static KeyDef> {
    KEYS.iter().find(|d| d.vk == vk)
}

/// Символ, который клавиша `vk` напечатает в раскладке `lang`.
///
/// Caps Lock влияет только на буквы, причём «буквенность» зависит от раскладки:
/// клавиша `;` с Caps Lock в русской раскладке даёт `Ж`, а в английской — по-прежнему `;`.
pub fn render(vk: u16, shift: bool, caps: bool, lang: Lang) -> Option<char> {
    let pair = def(vk)?.chars[lang.column()];
    let upper = if pair[0].is_alphabetic() { shift ^ caps } else { shift };
    Some(pair[upper as usize])
}

/// Обратный поиск: какой клавишей (и с Shift ли) набирается символ `c` в раскладке `lang`.
pub fn locate(c: char, lang: Lang) -> Option<(u16, bool)> {
    let col = lang.column();
    KEYS.iter().find_map(|d| {
        let pair = d.chars[col];
        if pair[0] == c {
            Some((d.vk, false))
        } else if pair[1] == c {
            Some((d.vk, true))
        } else {
            None
        }
    })
}

/// Перекодировка одного символа «как если бы его набрали в другой раскладке».
/// Символы, которых нет в таблице, возвращаются как есть.
pub fn convert_char(c: char, from: Lang, to: Lang) -> char {
    match locate(c, from) {
        Some((vk, shift)) => render(vk, shift, false, to).unwrap_or(c),
        None => c,
    }
}

/// Перекодировка строки посимвольно; регистр сохраняется.
pub fn convert_str(s: &str, from: Lang, to: Lang) -> String {
    s.chars().map(|c| convert_char(c, from, to)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_letters_with_case() {
        assert_eq!(render(0x51, false, false, Lang::Ru), Some('й'));
        assert_eq!(render(0x51, true, false, Lang::Ru), Some('Й'));
        assert_eq!(render(0x51, false, true, Lang::En), Some('Q'));
        assert_eq!(render(0x51, true, true, Lang::En), Some('q'));
    }

    #[test]
    fn caps_affects_only_letters_of_that_layout() {
        assert_eq!(render(0xBA, false, true, Lang::Ru), Some('Ж'));
        assert_eq!(render(0xBA, false, true, Lang::En), Some(';'));
        assert_eq!(render(0x36, true, false, Lang::Ru), Some(':'));
        assert_eq!(render(0x36, true, false, Lang::En), Some('^'));
    }

    #[test]
    fn converts_both_ways_preserving_case() {
        assert_eq!(convert_str("Ghbdtn", Lang::En, Lang::Ru), "Привет");
        assert_eq!(convert_str("Привет", Lang::Ru, Lang::En), "Ghbdtn");
        assert_eq!(convert_str("nfr;t", Lang::En, Lang::Ru), "также");
        assert_eq!(convert_str("ghbdtn,", Lang::En, Lang::Ru), "приветб");
    }

    #[test]
    fn conversion_is_an_involution_for_letters() {
        let en = "The quick brown fox jumps over the lazy dog";
        let ru = convert_str(en, Lang::En, Lang::Ru);
        assert_eq!(convert_str(&ru, Lang::Ru, Lang::En), en);
    }

    #[test]
    fn lang_ids() {
        assert_eq!(Lang::from_primary_lang_id(0x19), Some(Lang::Ru));
        assert_eq!(Lang::from_primary_lang_id(0x07), None);
    }
}
