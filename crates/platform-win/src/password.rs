//! Наблюдение за полями пароля через UI Automation (`IsPassword`). Работает в своём потоке:
//! COM-запросы к чужим процессам могут подвисать, поток хука их ждать не должен.
//!
//! Поток проверяет фокус сразу после «тычка» (щелчок, Tab, смена окна) и раз в
//! [`POLL_INTERVAL`] на всякий случай: в браузере фокус переходит между полями внутри одного окна.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};
use windows::Win32::UI::Accessibility::{CUIAutomation, IUIAutomation};

const POLL_INTERVAL: Duration = Duration::from_millis(300);

#[derive(Default)]
pub struct PasswordWatch {
    /// Фокус сейчас в поле пароля.
    pub in_password: AtomicBool,
    poke: Mutex<bool>,
    wake: Condvar,
    stop: AtomicBool,
}

impl PasswordWatch {
    /// Запускает поток наблюдения. Если UI Automation недоступна, флаг просто остаётся `false`.
    pub fn start() -> Arc<PasswordWatch> {
        let watch = Arc::new(PasswordWatch::default());
        let w = Arc::clone(&watch);
        let _ = std::thread::Builder::new().name("switcher-password".into()).spawn(move || w.run());
        watch
    }

    /// Проверить фокус как можно скорее. Дёшево, можно звать из хука.
    pub fn poke(&self) {
        if let Ok(mut p) = self.poke.try_lock() {
            *p = true;
            self.wake.notify_one();
        }
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        self.poke();
    }

    fn run(&self) {
        let automation: IUIAutomation = unsafe {
            if CoInitializeEx(None, COINIT_MULTITHREADED).is_err() {
                return;
            }
            match CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) {
                Ok(a) => a,
                Err(_) => return,
            }
        };
        while !self.stop.load(Ordering::Relaxed) {
            let is_password = unsafe {
                automation
                    .GetFocusedElement()
                    .and_then(|e| e.CurrentIsPassword())
                    .map(|b| b.as_bool())
                    .unwrap_or(false)
            };
            self.in_password.store(is_password, Ordering::Relaxed);
            let guard = self.poke.lock().unwrap_or_else(|e| e.into_inner());
            let (mut guard, _) = self
                .wake
                .wait_timeout_while(guard, POLL_INTERVAL, |poked| !*poked)
                .unwrap_or_else(|e| e.into_inner());
            *guard = false;
        }
    }
}
