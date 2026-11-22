//! Процесс активного окна — для профилей приложений.

use windows_sys::Win32::Foundation::CloseHandle;
use windows_sys::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

/// Имя исполняемого файла активного окна в нижнем регистре (`windowsterminal.exe`).
/// `None` для окон процессов с повышенными правами, если мы сами запущены без них.
/// Вызов не мгновенный — не для процедуры хука.
pub fn foreground_exe() -> Option<String> {
    foreground_exe_path().and_then(|p| p.rsplit('\\').next().map(str::to_string))
}

/// Полный путь к программе активного окна в нижнем регистре (`c:\games\...\game.exe`).
pub fn foreground_exe_path() -> Option<String> {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return None;
        }
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len);
        CloseHandle(process);
        if ok == 0 {
            return None;
        }
        Some(String::from_utf16_lossy(&buf[..len as usize]).to_lowercase())
    }
}

/// Что Windows говорит о полноэкранном режиме — для журнала. Это общесистемный признак: он
/// бывает «да», когда на весь экран развёрнуто что-то другое (видео на втором мониторе),
/// поэтому решение о паузе по нему не принимаем.
pub fn system_reports_fullscreen() -> Option<&'static str> {
    use windows_sys::Win32::UI::Shell::{
        SHQueryUserNotificationState, QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN,
    };
    let mut state = 0;
    if unsafe { SHQueryUserNotificationState(&mut state) } != 0 {
        return None;
    }
    match state {
        QUNS_BUSY => Some("полноэкранное приложение"),
        QUNS_RUNNING_D3D_FULL_SCREEN => Some("игра в монопольном полноэкранном режиме"),
        QUNS_PRESENTATION_MODE => Some("режим презентации"),
        _ => None,
    }
}

/// Активное окно занимает весь свой монитор: игра, полноэкранное видео, презентация.
///
/// Смотрим именно на активное окно: окно без рамки на весь монитор (так идёт большинство игр,
/// и монопольный полноэкранный режим тоже). Развёрнутое обычное окно (браузер, редактор)
/// полноэкранным не считается.
pub fn foreground_is_fullscreen() -> bool {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetClassNameW, GetWindowRect, IsZoomed};

    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() || IsZoomed(hwnd) != 0 {
            return false;
        }
        // Рабочий стол и панель задач тоже «на весь экран», но это не игра.
        let mut class = [0u16; 64];
        let len = GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32).max(0) as usize;
        let class = String::from_utf16_lossy(&class[..len]);
        if matches!(class.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd") {
            return false;
        }
        let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        if GetWindowRect(hwnd, &mut r) == 0 {
            return false;
        }
        let mut mi: MONITORINFO = std::mem::zeroed();
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut mi) == 0 {
            return false;
        }
        let m = mi.rcMonitor;
        let covers = r.left <= m.left && r.top <= m.top && r.right >= m.right && r.bottom >= m.bottom;
        // Переключатель окон (Alt+Tab) и «Представление задач» — окна Проводника на весь экран
        // на долю секунды: это не игра, значок из-за них мигать не должен.
        covers && foreground_exe().as_deref() != Some("explorer.exe")
    }
}

/// Активное окно — оболочка Windows, а не программа: панель задач и трей (туда щёлкают,
/// чтобы открыть меню Switcher), рабочий стол, меню «Пуск», поиск, центр уведомлений.
/// Окна Проводника с папками сюда не входят, хоть это тоже `explorer.exe`.
pub fn foreground_is_shell() -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::GetClassNameW;

    const SHELL_CLASSES: &[&str] = &[
        "Shell_TrayWnd",
        "Shell_SecondaryTrayWnd",
        "NotifyIconOverflowWindow",
        "TopLevelWindowForOverflowXamlIsland",
        "Progman",
        "WorkerW",
    ];
    const SHELL_HOSTS: &[&str] =
        &["shellexperiencehost.exe", "startmenuexperiencehost.exe", "searchhost.exe", "searchapp.exe"];

    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.is_null() {
        return true;
    }
    let mut class = [0u16; 64];
    let len = unsafe { GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32) }.max(0) as usize;
    let class = String::from_utf16_lossy(&class[..len]);
    SHELL_CLASSES.contains(&class.as_str())
        || foreground_exe().is_some_and(|e| SHELL_HOSTS.contains(&e.as_str()))
}

/// Активное окно запущено с бо́льшими правами, чем мы (от имени администратора): Windows не
/// пускает в такие окна наш ввод, а хук не видит их нажатий. Если узнать не удалось — `false`.
pub fn foreground_above_us() -> bool {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() {
            return false;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return false;
        }
        let theirs = integrity_level(process);
        CloseHandle(process);
        let ours = integrity_level(windows_sys::Win32::System::Threading::GetCurrentProcess());
        matches!((theirs, ours), (Some(t), Some(o)) if t > o)
    }
}

/// Уровень целостности процесса: 0x2000 — обычный, 0x3000 — администратор, 0x4000 — система.
unsafe fn integrity_level(process: windows_sys::Win32::Foundation::HANDLE) -> Option<u32> {
    use windows_sys::Win32::Security::{
        GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TokenIntegrityLevel,
        TOKEN_MANDATORY_LABEL, TOKEN_QUERY,
    };
    use windows_sys::Win32::System::Threading::OpenProcessToken;

    let mut token = std::ptr::null_mut();
    if OpenProcessToken(process, TOKEN_QUERY, &mut token) == 0 {
        return None;
    }
    // Метка с SID внутри: 64 байт с запасом хватает.
    let mut buf = [0u64; 16];
    let mut len = 0u32;
    let ok = GetTokenInformation(
        token,
        TokenIntegrityLevel,
        buf.as_mut_ptr().cast(),
        std::mem::size_of_val(&buf) as u32,
        &mut len,
    );
    CloseHandle(token);
    if ok == 0 {
        return None;
    }
    let label = &*(buf.as_ptr() as *const TOKEN_MANDATORY_LABEL);
    let sid = label.Label.Sid;
    let count = *GetSidSubAuthorityCount(sid);
    if count == 0 {
        return None;
    }
    Some(*GetSidSubAuthority(sid, u32::from(count) - 1))
}

#[cfg(test)]
mod tests {
    #[test]
    fn own_integrity_level_is_known() {
        let own = unsafe {
            super::integrity_level(windows_sys::Win32::System::Threading::GetCurrentProcess())
        };
        assert!(own.is_some_and(|l| l >= 0x2000), "{own:?}");
    }
}
