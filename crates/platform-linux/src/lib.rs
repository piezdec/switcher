//! Linux-примитивы для Switcher: перехват клавиатуры через evdev и uinput, раскладка и активное
//! окно в Hyprland и KDE, значок в трее, выделенный текст, звуки, автозапуск.

#[cfg(target_os = "linux")]
pub mod desktop;
#[cfg(target_os = "linux")]
pub mod input;
pub mod keymap;
#[cfg(target_os = "linux")]
pub mod misc;
#[cfg(target_os = "linux")]
pub mod tray;
