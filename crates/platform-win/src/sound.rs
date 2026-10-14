//! Короткие звуки: переключение раскладки (встроенный wav) и щелчок клавиши (синтезируется кодом).

use std::os::windows::ffi::OsStrExt;
use std::sync::OnceLock;
use windows_sys::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_FILENAME, SND_MEMORY, SND_NODEFAULT};

const RATE: u32 = 22_050;

/// Звук переключения раскладки — готовый wav (44,1 кГц, моно), встроен в exe.
const SWITCH_WAV: &[u8] = include_bytes!("../sounds/switch.wav");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sound {
    /// Переключение на русский.
    ToRussian,
    /// Переключение на английский (тот же звук).
    ToEnglish,
    /// Тихий щелчок при наборе.
    Click,
}

/// WAV (PCM 16 бит, моно) из отсчётов в диапазоне −1…1.
fn wav(samples: &[f32]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // моно
    out.extend_from_slice(&RATE.to_le_bytes());
    out.extend_from_slice(&(RATE * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).to_le_bytes());
    }
    out
}

/// Короткий щелчок клавиши: затухающий шум с низкой громкостью.
fn click_wav() -> Vec<u8> {
    let n = (RATE * 6 / 1000) as usize;
    let mut seed = 0x1234_5678u32;
    let samples: Vec<f32> = (0..n)
        .map(|i| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let noise = seed as f32 / u32::MAX as f32 * 2.0 - 1.0;
            noise * 0.12 * (1.0 - i as f32 / n as f32)
        })
        .collect();
    wav(&samples)
}

/// Буферы живут всё время работы: асинхронное проигрывание читает их уже после возврата.
fn buffer(sound: Sound) -> &'static [u8] {
    static CLICK: OnceLock<Vec<u8>> = OnceLock::new();
    match sound {
        Sound::ToRussian | Sound::ToEnglish => SWITCH_WAV,
        Sound::Click => CLICK.get_or_init(click_wav),
    }
}

/// Проигрывает wav-файл асинхронно. `false`, если файла нет или он не читается.
pub fn play_file(path: &std::path::Path) -> bool {
    if !path.is_file() {
        return false;
    }
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    unsafe { PlaySoundW(wide.as_ptr(), std::ptr::null_mut(), SND_FILENAME | SND_ASYNC | SND_NODEFAULT) != 0 }
}

/// Проигрывает звук асинхронно (предыдущий при этом обрывается).
pub fn play(sound: Sound) {
    let buf = buffer(sound);
    unsafe {
        PlaySoundW(buf.as_ptr().cast(), std::ptr::null_mut(), SND_MEMORY | SND_ASYNC | SND_NODEFAULT);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_header_is_consistent() {
        let w = click_wav();
        assert_eq!(&w[..4], b"RIFF");
        let riff_len = u32::from_le_bytes([w[4], w[5], w[6], w[7]]) as usize;
        assert_eq!(riff_len + 8, w.len());
    }
}
