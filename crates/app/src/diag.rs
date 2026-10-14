//! Журнал работы (`switcher.log` в папке настроек) — чтобы разбирать непонятные ситуации:
//! почему программа на паузе, что не удалось, где упала.
//!
//! Главное правило: **набранный текст в журнал не пишется никогда** — ни слова, ни буквы.
//! Только события («переключил EN→RU, слово из 6 символов»). Пути с именем пользователя
//! маскируются, чтобы журнал можно было переслать.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Сколько хранить: текущий файл до 1 МБ плюс одна предыдущая часть (`switcher.log.1`).
const MAX_BYTES: u64 = 1024 * 1024;

struct Log {
    path: PathBuf,
    file: File,
    written: u64,
}

static LOG: Mutex<Option<Log>> = Mutex::new(None);

pub fn path() -> PathBuf {
    crate::config::data_dir().join("switcher.log")
}

/// Открывает журнал и ставит запись паник в него. Зовётся один раз при старте.
pub fn init(role: &str) {
    let path = path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    rotate_if_big(&path);
    if let Ok(file) = OpenOptions::new().create(true).append(true).open(&path) {
        let written = file.metadata().map(|m| m.len()).unwrap_or(0);
        *LOG.lock().unwrap_or_else(|e| e.into_inner()) = Some(Log { path, file, written });
    }
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let place = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_default();
        write(&format!("СБОЙ: программа упала в {place}: {message}"));
        default_hook(info);
    }));
    write(&format!(
        "==== запуск ({role}) — Switcher {} на {} {}",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH
    ));
}

fn rotate_if_big(path: &PathBuf) {
    if std::fs::metadata(path).is_ok_and(|m| m.len() > MAX_BYTES) {
        let _ = std::fs::rename(path, path.with_extension("log.1"));
    }
}

/// Записывает строку с отметкой времени (UTC). Ничего не делает, если журнал не открыт.
pub fn write(line: &str) {
    let mut guard = LOG.lock().unwrap_or_else(|e| e.into_inner());
    let Some(log) = guard.as_mut() else { return };
    let text = format!("{} {line}\n", timestamp());
    if log.file.write_all(text.as_bytes()).is_ok() {
        log.written += text.len() as u64;
    }
    if log.written > MAX_BYTES {
        let path = log.path.clone();
        *guard = None;
        rotate_if_big(&path);
        if let Ok(file) = OpenOptions::new().create(true).append(true).open(&path) {
            *guard = Some(Log { path, file, written: 0 });
        }
    }
}

/// Запись в журнал в стиле `format!`.
macro_rules! diag {
    ($($t:tt)*) => {
        $crate::diag::write(&format!($($t)*))
    };
}

/// Время в UTC: `2026-10-14 09:30:05Z`.
fn timestamp() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let (days, rem) = (secs / 86_400, secs % 86_400);
    let (y, m, d) = civil_from_days(days as i64);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// Дата по числу дней от 1970-01-01 (алгоритм Говарда Хиннанта).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// Где программа и её настройки, и сами настройки — в начало журнала после запуска.
pub fn log_start(cfg: &crate::config::Config) {
    let exe = std::env::current_exe().map(|p| redact(&p.to_string_lossy())).unwrap_or_default();
    write(&format!("программа: {exe}; настройки: {}", redact(&crate::config::data_dir().to_string_lossy())));
    write(&format!(
        "настройки: автопереключение {}, режим {:?}, осторожно в терминалах {}, пауза в играх {}, \
         автообновление {}, исключения [{}]",
        on_off(cfg.auto),
        cfg.mode,
        on_off(cfg.careful_in_dev_tools),
        on_off(cfg.pause_in_fullscreen),
        on_off(cfg.auto_update),
        cfg.excluded_apps.join(", ")
    ));
}

fn on_off(b: bool) -> &'static str {
    if b {
        "вкл"
    } else {
        "выкл"
    }
}

/// Путь без имени пользователя: домашняя папка заменяется на `~`.
pub fn redact(path: &str) -> String {
    let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_default();
    if home.len() > 3 && path.to_lowercase().starts_with(&home.to_lowercase()) {
        format!("~{}", &path[home.len()..])
    } else {
        path.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_740), (2026, 10, 14));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
    }

    #[test]
    fn home_is_hidden() {
        let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap();
        let p = format!("{home}/AppData/x.exe");
        assert!(redact(&p).starts_with("~"));
        assert!(!redact(&p).contains(&home));
        assert_eq!(redact("C:/Games/a.exe"), "C:/Games/a.exe");
    }
}
