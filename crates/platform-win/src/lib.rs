//! Windows-примитивы без логики приложения: низкоуровневые хуки, синтез ввода,
//! раскладки окон, значок в трее.

pub mod autostart;
pub mod clipboard;
pub mod hint;
pub mod hook;
pub mod input;
pub mod layout;
pub mod password;
pub mod process;
pub mod sound;
pub mod tray;

/// Строка в UTF-16 с завершающим нулём для WinAPI.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}
