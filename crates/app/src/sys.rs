//! Граница между общей логикой (контроллер, настройки) и платформой (Windows, Linux).

use engine::Lang;

/// Событие клавиатуры в общем виде. `vk` — виртуальный код в терминах Windows (VK_*): им
/// пользуются ядро и горячие клавиши на всех платформах. `raw` — как повторить нажатие на своей
/// платформе (скан-код Windows или код evdev в Linux).
#[derive(Clone, Copy, Debug)]
pub struct KeyEvent {
    pub vk: u16,
    pub up: bool,
    pub raw: RawKey,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RawKey {
    pub code: u16,
    pub extended: bool,
    /// Автоповтор зажатой клавиши (в Linux у него отдельное значение события).
    pub repeat: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    /// Не пропускать событие дальше по системе.
    Swallow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sound {
    ToRussian,
    ToEnglish,
    Click,
}

/// Что контроллеру нужно знать о системе. Вызывается из потока перехвата клавиатуры,
/// поэтому реализации должны отвечать быстро (кешировать, а не опрашивать систему каждый раз).
pub trait Sys {
    /// Идентификатор активного окна — чтобы заметить его смену.
    fn foreground_window(&self) -> u64;
    /// Раскладка активного окна (`None` — не русская и не английская или неизвестна).
    fn foreground_lang(&self) -> Option<Lang>;
    /// Программа активного окна в нижнем регистре: `code.exe` в Windows, `kitty` в Linux.
    fn foreground_app(&self) -> Option<String>;
    /// Как называется наша собственная программа (окно настроек) — её не трогаем.
    fn own_app(&self) -> Option<String>;
    fn caps_lock_on(&self) -> bool;
    /// Активное окно занимает весь экран (игра, полноэкранное видео).
    fn is_fullscreen(&self) -> bool;
    /// Активное окно — игра, установленная лаунчером (Steam, Epic, GOG…), даже не на весь экран.
    fn is_game(&self) -> bool;
    /// Активное окно — оболочка (панель задач, трей, рабочий стол), а не программа: щелчок по
    /// значку Switcher делает её активной, и её нельзя запоминать как «последнюю программу».
    fn is_shell(&self) -> bool;
    /// Фокус в поле пароля.
    fn in_password(&self) -> bool;
    /// Попросить проверить поле пароля как можно скорее (щелчок, Tab).
    fn poke_password(&self);
    /// Подробности об активном окне для журнала (путь без имени пользователя и т.п.).
    fn describe(&self) -> String;
    fn play_sound(&self, sound: Sound);
}

pub const VK_TAB: u16 = 0x09;
pub const VK_CAPITAL: u16 = 0x14;
