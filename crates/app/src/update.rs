//! Автообновление с GitHub Releases этого репозитория — единственное сетевое обращение программы.
//!
//! Проверка: страница последнего релиза (`api.github.com/.../releases/latest`). Если версия новее,
//! скачиваем файл программы для своей платформы и его контрольную сумму (`*.sha256`, её публикует
//! сборка на GitHub), сверяем и ставим новую версию на место текущей. Работающий файл в Windows
//! перезаписать нельзя, но можно переименовать: старый уходит в `*.old`, новый ложится на его
//! место и запускается при следующем старте (или при перезапуске, когда человек отошёл).
//!
//! Если папка программы недоступна для записи (Program Files), новая версия ставится в папку
//! пользователя, а автозапуск переключается на неё.
//!
//! Скачиваем системным `curl` (есть в Windows 10/11 и в любом Linux): без сетевых библиотек
//! внутри программы. Нет интернета — проверка тихо не удаётся и повторяется позже.

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::Duration;

const LATEST: &str = "https://api.github.com/repos/piezdec/switcher/releases/latest";

#[cfg(windows)]
const ASSET: &str = "switcher.exe";
#[cfg(not(windows))]
const ASSET: &str = "switcher-linux-x86_64";

/// Имя файла программы на диске.
#[cfg(windows)]
const EXE_NAME: &str = "switcher.exe";
#[cfg(not(windows))]
const EXE_NAME: &str = "switcher";

/// Через сколько после запуска первая проверка и как часто потом.
const FIRST_CHECK: Duration = Duration::from_secs(20);
const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);

pub type Version = (u32, u32, u32);

/// Что известно об обновлении — для значка, меню и окна настроек.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Idle,
    Checking,
    UpToDate,
    /// Найдена версия новее — можно ставить.
    Available(Release),
    /// Скачивается и ставится.
    Installing,
    /// Новая версия скачана и стоит на месте старой; применится при перезапуске.
    Ready {
        tag: String,
        exe: PathBuf,
    },
    Failed(String),
}

static STATUS: Mutex<Status> = Mutex::new(Status::Idle);

pub fn status() -> Status {
    STATUS.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

fn set_status(s: Status) {
    *STATUS.lock().unwrap_or_else(|e| e.into_inner()) = s;
}

/// «v0.2.4» или «0.2.4» → (0, 2, 4).
pub fn parse_version(s: &str) -> Option<Version> {
    let mut parts = s.trim().trim_start_matches('v').split('.').map(|p| p.parse::<u32>().ok());
    Some((parts.next()??, parts.next()??, parts.next().flatten().unwrap_or(0)))
}

pub fn current_version() -> Version {
    parse_version(env!("CARGO_PKG_VERSION")).unwrap_or((0, 0, 0))
}

/// `curl` без мелькающего окна консоли, с таймаутом и заголовком, который требует GitHub.
fn curl() -> Command {
    let mut c = Command::new("curl");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    c.args(["-fsSL", "--max-time", "120", "-H", "User-Agent: switcher-updater"]);
    c
}

fn fetch(url: &str) -> Result<Vec<u8>, String> {
    let out = curl().arg(url).output().map_err(|e| format!("curl не запустился: {e}"))?;
    if !out.status.success() {
        return Err(format!("нет связи с GitHub ({})", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(out.stdout)
}

/// Последний релиз: версия, ссылка на файл программы и на его контрольную сумму.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub tag: String,
    pub version: Version,
    pub asset_url: String,
    pub asset_size: u64,
    pub sha_url: String,
}

pub fn parse_release(json: &[u8]) -> Result<Release, String> {
    let v: serde_json::Value =
        serde_json::from_slice(json).map_err(|e| format!("ответ GitHub не разобрать: {e}"))?;
    let tag = v["tag_name"].as_str().ok_or("в релизе нет версии")?.to_string();
    let version = parse_version(&tag).ok_or_else(|| format!("непонятная версия {tag}"))?;
    let assets = v["assets"].as_array().ok_or("в релизе нет файлов")?;
    let find = |name: &str| assets.iter().find(|a| a["name"].as_str() == Some(name));
    let asset = find(ASSET).ok_or_else(|| format!("в релизе нет {ASSET}"))?;
    let sha = find(&format!("{ASSET}.sha256")).ok_or("в релизе нет контрольной суммы")?;
    Ok(Release {
        tag,
        version,
        asset_url: asset["browser_download_url"].as_str().unwrap_or_default().to_string(),
        asset_size: asset["size"].as_u64().unwrap_or(0),
        sha_url: sha["browser_download_url"].as_str().unwrap_or_default().to_string(),
    })
}

fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}

/// Куда ставить новую версию: рядом с текущей, если туда можно писать, иначе — в папку
/// пользователя (`%LOCALAPPDATA%\Programs\Switcher` или `~/.local/bin`).
fn install_dir(exe: &Path) -> PathBuf {
    let here = exe.parent().map(Path::to_path_buf).unwrap_or_default();
    let probe = here.join(".switcher-write-test");
    if std::fs::write(&probe, b"").is_ok() {
        let _ = std::fs::remove_file(&probe);
        return here;
    }
    user_install_dir()
}

#[cfg(windows)]
fn user_install_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("Programs").join("Switcher")
}

#[cfg(not(windows))]
fn user_install_dir() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    home.join(".local").join("bin")
}

/// Ставит скачанный файл на место программы в `dir`. Работающий файл отодвигается в `*.old`.
fn put_in_place(dir: &Path, data: &[u8]) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("не создать {}: {e}", dir.display()))?;
    let target = dir.join(EXE_NAME);
    let fresh = dir.join(format!("{EXE_NAME}.new"));
    std::fs::write(&fresh, data).map_err(|e| format!("не записать {}: {e}", fresh.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&fresh, std::fs::Permissions::from_mode(0o755));
    }
    if target.exists() {
        let old = dir.join(format!("{EXE_NAME}.old"));
        let _ = std::fs::remove_file(&old);
        std::fs::rename(&target, &old).map_err(|e| format!("не отодвинуть старую версию: {e}"))?;
    }
    std::fs::rename(&fresh, &target).map_err(|e| format!("не поставить новую версию: {e}"))?;
    Ok(target)
}

/// Удаляет остатки прошлого обновления (`*.old`, `*.new`) рядом с программой. Зовётся, когда
/// прежняя копия уже вышла: файл работающей программы Windows удалить не даёт.
pub fn cleanup() {
    let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) else {
        return;
    };
    for suffix in ["old", "new"] {
        let file = dir.join(format!("{EXE_NAME}.{suffix}"));
        // Прежняя копия могла ещё не до конца завершиться — пробуем до 3 секунд.
        for _ in 0..15 {
            if !file.exists() || std::fs::remove_file(&file).is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }
}

/// Проверяет и, если вышла новая версия, ставит её. Возвращает итог для показа человеку.
/// Только проверка: есть ли версия новее. Ничего не скачивает, кроме описания релиза.
pub fn check() -> Status {
    if matches!(status(), Status::Ready { .. } | Status::Installing) {
        return status(); // уже ставится или стоит, ждём перезапуска
    }
    set_status(Status::Checking);
    let s = match fetch(LATEST).and_then(|json| parse_release(&json)) {
        Ok(r) if r.version > current_version() => Status::Available(r),
        Ok(_) => Status::UpToDate,
        Err(why) => Status::Failed(why),
    };
    set_status(s.clone());
    match &s {
        Status::UpToDate => diag!("обновление: у вас последняя версия {}", env!("CARGO_PKG_VERSION")),
        Status::Available(r) => diag!("обновление: доступна {}", r.tag),
        Status::Failed(why) => diag!("обновление: проверка не получилась — {why}"),
        _ => {}
    }
    s
}

/// Ставит найденную проверкой версию: скачивает, сверяет контрольную сумму, кладёт на место.
pub fn install_available() -> Status {
    let Status::Available(release) = status() else { return status() };
    set_status(Status::Installing);
    let s = install(&release).unwrap_or_else(Status::Failed);
    set_status(s.clone());
    match &s {
        Status::Ready { tag, exe } => diag!(
            "обновление: установлена {tag} в {}; заработает после перезапуска",
            crate::diag::redact(&exe.to_string_lossy())
        ),
        Status::Failed(why) => diag!("обновление: установка не получилась — {why}"),
        _ => {}
    }
    s
}

/// Фоновый путь: проверить и сразу поставить.
pub fn check_and_install() -> Status {
    match check() {
        Status::Available(_) => install_available(),
        other => other,
    }
}

fn install(release: &Release) -> Result<Status, String> {
    let data = fetch(&release.asset_url)?;
    if release.asset_size != 0 && data.len() as u64 != release.asset_size {
        return Err("файл скачался не целиком".into());
    }
    let expected = String::from_utf8_lossy(&fetch(&release.sha_url)?)
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_lowercase();
    if expected.len() != 64 || sha256_hex(&data) != expected {
        return Err("контрольная сумма не совпала — файл не установлен".into());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = install_dir(&exe);
    let moved = exe.parent() != Some(dir.as_path());
    let installed = put_in_place(&dir, &data)?;
    // Переехали в папку пользователя — автозапуск должен запускать уже новую копию.
    if moved && crate::platform::autostart_registered() {
        crate::platform::set_autostart_to(&installed);
    }
    Ok(Status::Ready { tag: release.tag.clone(), exe: installed })
}

/// Из окна настроек: закрыть работающий Switcher и запустить новую версию вместо него.
/// Новая копия дождётся, пока прежняя выйдет.
pub fn restart_main_into_update() -> bool {
    crate::platform::request_main_exit();
    launch_updated()
}

/// Фоновые проверки: через 20 секунд после запуска и раз в 6 часов, пока включены в настройках.
pub fn spawn_auto() {
    let _ = std::thread::Builder::new().name("switcher-update".into()).spawn(|| {
        std::thread::sleep(FIRST_CHECK);
        loop {
            if crate::config::load_config().auto_update {
                check_and_install();
            }
            std::thread::sleep(CHECK_EVERY);
        }
    });
}

/// Сколько человек должен не печатать, чтобы перезапуститься в новую версию незаметно.
const IDLE_BEFORE_RESTART: Duration = Duration::from_secs(10 * 60);

static LAST_INPUT: Mutex<Option<std::time::Instant>> = Mutex::new(None);

/// Отметить нажатие клавиши (дёшево, зовётся на каждое нажатие).
pub fn touch_input() {
    if let Ok(mut t) = LAST_INPUT.try_lock() {
        *t = Some(std::time::Instant::now());
    }
}

/// Новая версия готова, а человек давно не печатает — можно тихо перезапуститься.
pub fn should_restart_now() -> bool {
    let idle =
        LAST_INPUT.lock().map(|t| t.is_none_or(|t| t.elapsed() >= IDLE_BEFORE_RESTART)).unwrap_or(false);
    idle && matches!(status(), Status::Ready { .. })
}

/// Подпись для меню: «Перезапустить и обновить до v0.3.0», если обновление готово.
pub fn ready_tag() -> Option<String> {
    match status() {
        Status::Ready { tag, .. } => Some(tag),
        _ => None,
    }
}

/// Запускает обновлённую программу (она подождёт, пока эта копия выйдет) — вызывающий
/// после этого должен завершиться.
pub fn launch_updated() -> bool {
    let Status::Ready { exe, .. } = status() else { return false };
    let mut c = Command::new(&exe);
    c.arg("--after-update");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0000_0008); // DETACHED_PROCESS
    }
    let ok = c.spawn().is_ok();
    diag!("перезапуск в новую версию: {}", if ok { "запущена" } else { "не запустилась" });
    ok
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(parse_version("v0.2.4"), Some((0, 2, 4)));
        assert_eq!(parse_version("1.10"), Some((1, 10, 0)));
        assert_eq!(parse_version("abc"), None);
        assert!(parse_version("v0.3.0").unwrap() > parse_version("v0.2.9").unwrap());
        assert!(parse_version("v0.2.10").unwrap() > parse_version("v0.2.9").unwrap());
    }

    #[test]
    fn release_json() {
        let json = format!(
            r#"{{"tag_name":"v9.9.9","assets":[
                {{"name":"{ASSET}","size":42,"browser_download_url":"https://x/{ASSET}"}},
                {{"name":"{ASSET}.sha256","size":64,"browser_download_url":"https://x/{ASSET}.sha256"}}]}}"#
        );
        let r = parse_release(json.as_bytes()).unwrap();
        assert_eq!(r.version, (9, 9, 9));
        assert_eq!(r.asset_size, 42);
        assert!(r.sha_url.ends_with(".sha256"));
        // Без контрольной суммы не ставим.
        let no_sha = format!(
            r#"{{"tag_name":"v9.9.9","assets":[{{"name":"{ASSET}","size":1,"browser_download_url":"u"}}]}}"#
        );
        assert!(parse_release(no_sha.as_bytes()).is_err());
    }

    #[test]
    fn checksum() {
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn swap_in_place() {
        let dir = std::env::temp_dir().join(format!("switcher-update-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(EXE_NAME), b"old").unwrap();
        let target = put_in_place(&dir, b"new").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        assert_eq!(std::fs::read(dir.join(format!("{EXE_NAME}.old"))).unwrap(), b"old");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
