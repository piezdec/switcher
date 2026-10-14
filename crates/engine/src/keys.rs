//! Нажатия клавиш в терминах ядра: что считается частью слова, что — границей.

use layouts::Lang;

pub const VK_BACK: u16 = 0x08;
pub const VK_TAB: u16 = 0x09;
pub const VK_RETURN: u16 = 0x0D;
pub const VK_SPACE: u16 = 0x20;
pub const VK_OEM_MINUS: u16 = 0xBD;

/// Одно нажатие, сохранённое в буфере слова. Хранится физическая клавиша, а не символ,
/// поэтому слово можно «отрисовать» в любой раскладке.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stroke {
    pub vk: u16,
    pub shift: bool,
    pub caps: bool,
}

impl Stroke {
    pub fn render(self, lang: Lang) -> Option<char> {
        layouts::render(self.vk, self.shift, self.caps, lang)
    }
}

/// Отрисовка последовательности нажатий в заданной раскладке.
pub fn render_word(keys: &[Stroke], lang: Lang) -> String {
    keys.iter().filter_map(|k| k.render(lang)).collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyClass {
    /// Печатает символ, который входит в слово хотя бы в одной раскладке.
    Word,
    /// Завершает слово: пробел, Enter, Tab, `/`, `\`, `-`.
    Boundary,
    Backspace,
    /// Shift, Ctrl, Alt, Win, Caps Lock и т.п. — на буфер не влияют.
    Modifier,
    /// Стрелки, Home, Delete, F-клавиши и всё прочее — каретка могла уехать, буфер сбрасывается.
    Reset,
}

pub fn classify(vk: u16, shift: bool) -> KeyClass {
    match vk {
        VK_BACK => KeyClass::Backspace,
        VK_SPACE | VK_RETURN | VK_TAB => KeyClass::Boundary,
        // `/` и `\` — знаки в обеих раскладках.
        0xBF | 0xDC | 0xE2 => KeyClass::Boundary,
        // Дефис делит слово («что-то» → «что» + «то»), а `_` держит идентификатор целиком.
        VK_OEM_MINUS => {
            if shift {
                KeyClass::Word
            } else {
                KeyClass::Boundary
            }
        }
        0x30..=0x39 | 0x41..=0x5A => KeyClass::Word,
        // ; = , . ` [ ] ' — в русской раскладке часть из них буквы, остальные держат токен (a=b).
        0xBA | 0xBB | 0xBC | 0xBE | 0xC0 | 0xDB | 0xDD | 0xDE => KeyClass::Word,
        // Shift, Ctrl, Alt (общие и левые/правые), Win, Caps, NumLock, ScrollLock.
        0x10 | 0x11 | 0x12 | 0xA0..=0xA5 | 0x5B | 0x5C | 0x14 | 0x90 | 0x91 => KeyClass::Modifier,
        _ => KeyClass::Reset,
    }
}
