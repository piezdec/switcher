//! Сведения о системе для контроллера — через WinAPI.

use crate::sys::{Sound, Sys};
use engine::Lang;
use platform_win::password::PasswordWatch;
use platform_win::{layout, process, sound};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CAPITAL};

pub struct WinSys {
    password: Arc<PasswordWatch>,
    own_exe: Option<String>,
}

impl WinSys {
    pub fn new(password: Arc<PasswordWatch>) -> Self {
        let own_exe = std::env::current_exe()
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
        WinSys { password, own_exe }
    }
}

impl Sys for WinSys {
    fn foreground_window(&self) -> u64 {
        layout::foreground_window() as u64
    }

    fn foreground_lang(&self) -> Option<Lang> {
        layout::foreground_lang()
    }

    fn foreground_app(&self) -> Option<String> {
        process::foreground_exe()
    }

    fn own_app(&self) -> Option<String> {
        self.own_exe.clone()
    }

    fn caps_lock_on(&self) -> bool {
        unsafe { GetKeyState(VK_CAPITAL as i32) & 1 != 0 }
    }

    fn is_fullscreen(&self) -> bool {
        process::foreground_is_fullscreen()
    }

    fn is_game(&self) -> bool {
        process::foreground_exe_path().is_some_and(|p| crate::profiles::is_game_path(&p))
    }

    fn is_shell(&self) -> bool {
        process::foreground_is_shell()
    }

    fn describe(&self) -> String {
        let path = process::foreground_exe_path()
            .map(|p| crate::diag::redact(&p))
            .unwrap_or_else(|| "путь неизвестен".into());
        match process::system_reports_fullscreen() {
            Some(s) => format!("{path}; Windows сообщает: {s}"),
            None => path,
        }
    }

    fn in_password(&self) -> bool {
        self.password.in_password.load(Ordering::Relaxed)
    }

    fn poke_password(&self) {
        self.password.poke();
    }

    fn play_sound(&self, s: Sound) {
        super::ui::post_sound(match s {
            Sound::ToRussian => sound::Sound::ToRussian,
            Sound::ToEnglish => sound::Sound::ToEnglish,
            Sound::Click => sound::Sound::Click,
        });
    }
}
