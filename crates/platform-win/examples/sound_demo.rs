//! Проигрывает звуки Switcher по очереди: переключение на русский, на английский, щелчок.
//!   cargo run -p platform-win --example sound_demo

use platform_win::sound::{play, Sound};
use std::time::Duration;

fn main() {
    for s in [Sound::ToRussian, Sound::ToEnglish, Sound::Click, Sound::Click, Sound::Click] {
        println!("{s:?}");
        play(s);
        std::thread::sleep(Duration::from_millis(600));
    }
}
