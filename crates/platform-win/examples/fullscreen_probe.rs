//! Печатает, считается ли активное окно полноэкранным (раз в секунду, 5 раз).
//!   cargo run -p platform-win --example fullscreen_probe

fn main() {
    for _ in 0..5 {
        println!(
            "fullscreen: {}, app: {:?}",
            platform_win::process::foreground_is_fullscreen(),
            platform_win::process::foreground_exe()
        );
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}
