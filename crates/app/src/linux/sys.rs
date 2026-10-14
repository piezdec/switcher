//! Сведения о системе для контроллера — из кеша рабочего стола (Hyprland, KDE).

use crate::sys::{Sound, Sys};
use engine::Lang;
use platform_linux::desktop::DesktopState;
use platform_linux::misc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Звук переключения — тот же файл, что и в Windows-версии.
const SWITCH_WAV: &[u8] = include_bytes!("../../../platform-win/sounds/switch.wav");

pub struct LinuxSys {
    pub state: Arc<DesktopState>,
    pub caps: Arc<AtomicBool>,
}

impl Sys for LinuxSys {
    fn foreground_window(&self) -> u64 {
        self.state.window()
    }

    fn foreground_lang(&self) -> Option<Lang> {
        self.state.lang()
    }

    fn foreground_app(&self) -> Option<String> {
        self.state.app()
    }

    fn own_app(&self) -> Option<String> {
        // Класс окна настроек (задаётся в settings.rs через app_id).
        Some("switcher".into())
    }

    fn caps_lock_on(&self) -> bool {
        self.caps.load(Ordering::Relaxed)
    }

    fn is_fullscreen(&self) -> bool {
        self.state.fullscreen()
    }

    fn is_game(&self) -> bool {
        if self.state.app().is_some_and(|a| crate::profiles::is_game_app(&a)) {
            return true;
        }
        // Нативные игры Steam и др.: путь к программе окна — по его PID.
        let pid = self.state.pid();
        pid != 0
            && std::fs::read_link(format!("/proc/{pid}/exe"))
                .is_ok_and(|p| crate::profiles::is_game_path(&p.to_string_lossy()))
    }

    fn is_shell(&self) -> bool {
        // Панели с треем: в KDE — plasmashell, в Hyprland waybar окном не становится.
        self.state
            .app()
            .is_some_and(|a| matches!(a.as_str(), "plasmashell" | "org.kde.plasmashell" | "waybar"))
    }

    fn describe(&self) -> String {
        let pid = self.state.pid();
        let path = std::fs::read_link(format!("/proc/{pid}/exe"))
            .map(|p| crate::diag::redact(&p.to_string_lossy()))
            .unwrap_or_else(|_| "путь неизвестен".into());
        format!("{path}; pid {pid}")
    }

    // Поля пароля в Linux пока не распознаются (нужен AT-SPI).
    fn in_password(&self) -> bool {
        false
    }

    fn poke_password(&self) {}

    fn play_sound(&self, sound: Sound) {
        match sound {
            Sound::ToRussian | Sound::ToEnglish => misc::play_wav("switch", SWITCH_WAV),
            Sound::Click => misc::play_wav("click", &click_wav()),
        }
    }
}

/// Короткий щелчок клавиши: затухающий шум (как в Windows-версии).
fn click_wav() -> Vec<u8> {
    const RATE: u32 = 22_050;
    let n = (RATE * 6 / 1000) as usize;
    let mut seed = 0x1234_5678u32;
    let samples: Vec<i16> = (0..n)
        .map(|i| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let noise = seed as f32 / u32::MAX as f32 * 2.0 - 1.0;
            (noise * 0.12 * (1.0 - i as f32 / n as f32) * i16::MAX as f32) as i16
        })
        .collect();
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    for v in [16u32.to_le_bytes(), [1, 0, 1, 0], RATE.to_le_bytes(), (RATE * 2).to_le_bytes(), [2, 0, 16, 0]]
    {
        out.extend_from_slice(&v);
    }
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}
