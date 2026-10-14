//! Мелочи окружения: выделенный текст, звуки, автозапуск, открытие папки.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Выделенный мышью текст (primary selection): `wl-paste -p` в Wayland, `xclip` в X11.
/// Буфер обмена при этом не трогается.
pub fn selected_text() -> Option<String> {
    let tries: [(&str, &[&str]); 3] = [
        ("wl-paste", &["--primary", "--no-newline"]),
        ("xclip", &["-o", "-selection", "primary"]),
        ("xsel", &["--primary", "--output"]),
    ];
    tries.iter().find_map(|(cmd, args)| {
        let out = Command::new(cmd).args(*args).stderr(Stdio::null()).output().ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    })
}

/// Содержимое буфера обмена (для макроса `{clipboard}`).
pub fn clipboard_text() -> Option<String> {
    let tries: [(&str, &[&str]); 2] =
        [("wl-paste", &["--no-newline"]), ("xclip", &["-o", "-selection", "clipboard"])];
    tries.iter().find_map(|(cmd, args)| {
        let out = Command::new(cmd).args(*args).stderr(Stdio::null()).output().ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    })
}

/// Печать произвольного Unicode-текста там, где это умеет композитор (протокол
/// virtual-keyboard: Hyprland, Sway). В KDE не работает — там печатаем только символы
/// с клавиатуры.
pub fn wtype(text: &str) -> bool {
    Command::new("wtype").arg("--").arg(text).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

/// Проигрывает wav асинхронно: `pw-play` (PipeWire), `paplay` (PulseAudio) или `aplay` (ALSA).
/// Файл кладётся во временный каталог один раз.
pub fn play_wav(name: &str, data: &[u8]) {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let path = dir.join(format!("switcher-{name}.wav"));
    if !path.exists() {
        let Ok(mut f) = std::fs::File::create(&path) else { return };
        if f.write_all(data).is_err() {
            return;
        }
    }
    for player in ["pw-play", "paplay", "aplay"] {
        let started = Command::new(player)
            .arg(&path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        if started.is_ok() {
            return;
        }
    }
}

fn autostart_file() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("autostart").join("switcher.desktop"))
}

/// Автозапуск через `~/.config/autostart/switcher.desktop` (KDE, GNOME и др.).
/// В Hyprland его читает только `dex`/`uwsm`, проще добавить `exec-once = switcher` в hyprland.conf.
pub fn autostart_enabled() -> bool {
    autostart_file().is_some_and(|f| f.exists())
}

pub fn set_autostart(on: bool) -> bool {
    let Some(file) = autostart_file() else { return false };
    if !on {
        return std::fs::remove_file(&file).is_ok() || !file.exists();
    }
    let Ok(exe) = std::env::current_exe() else { return false };
    set_autostart_for(&exe)
}

/// Автозапуск конкретной программы (после переезда обновлённой копии).
pub fn set_autostart_for(exe: &Path) -> bool {
    let Some(file) = autostart_file() else { return false };
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName=Switcher\nComment=Автопереключатель раскладки\nExec=\"{}\"\nX-GNOME-Autostart-enabled=true\nNoDisplay=true\n",
        exe.display()
    );
    file.parent().is_some_and(|d| std::fs::create_dir_all(d).is_ok()) && std::fs::write(&file, entry).is_ok()
}

pub fn open_folder(path: &Path) {
    let _ = Command::new("xdg-open").arg(path).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
}
