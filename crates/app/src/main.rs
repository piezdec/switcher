// В релизе — без консольного окна; в отладке консоль остаётся для логов.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// Отладочный лог в консоль. В релизной сборке не печатает ничего — набранные слова
/// не должны никуда утекать.
macro_rules! debug_log {
    ($($t:tt)*) => {
        if cfg!(debug_assertions) {
            eprintln!($($t)*);
        }
    };
}

#[macro_use]
mod diag;

mod config;
mod controller;
mod hotkeys;
mod macros;
mod profiles;
mod settings;
mod sys;
mod update;

#[cfg(windows)]
mod win;
#[cfg(windows)]
use win as platform;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as platform;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // Служебное: обновиться сейчас и напечатать итог (для проверки обновлений).
    if args.iter().any(|a| a == "--update-now") {
        println!("{:?}", update::check_and_install());
        return;
    }
    if let Some(i) = args.iter().position(|a| a == "--settings") {
        platform::run_settings(args.get(i + 1).map(String::as_str));
        return;
    }
    // После обновления старая копия ещё выходит — новая подождёт её, а не закроется.
    platform::run(args.iter().any(|a| a == "--after-update"));
}
