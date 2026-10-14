//! Встроенные профили приложений. На этапе 3 они переедут в конфиг и станут редактируемыми.

/// Терминалы и редакторы кода: там много команд, флагов и идентификаторов,
/// поэтому по умолчанию в них работает осторожный режим.
#[rustfmt::skip]
pub const DEV_TOOLS: &[&str] = &[
    // Терминалы
    "windowsterminal.exe", "wt.exe", "openconsole.exe", "conhost.exe", "cmd.exe",
    "powershell.exe", "pwsh.exe", "wezterm-gui.exe", "alacritty.exe", "mintty.exe",
    "tabby.exe", "hyper.exe", "putty.exe", "kitty.exe", "termius.exe", "warp.exe",
    // Редакторы и IDE
    "code.exe", "code - insiders.exe", "cursor.exe", "windsurf.exe", "zed.exe",
    "devenv.exe", "idea64.exe", "pycharm64.exe", "rider64.exe", "clion64.exe",
    "webstorm64.exe", "goland64.exe", "rustrover64.exe", "datagrip64.exe",
    "phpstorm64.exe", "studio64.exe", "fleet.exe", "sublime_text.exe",
    "notepad++.exe", "nvim-qt.exe", "gvim.exe", "neovide.exe",
    // Linux: класс окна
    "kitty", "alacritty", "foot", "footclient", "org.wezfurlong.wezterm", "wezterm",
    "org.kde.konsole", "konsole", "org.gnome.terminal", "gnome-terminal-server", "org.gnome.ptyxis",
    "com.mitchellh.ghostty", "ghostty", "xterm", "urxvt", "tilix", "terminator",
    "code", "code-oss", "codium", "vscodium", "cursor", "dev.zed.zed", "zed", "neovide",
    "org.kde.kate", "kate", "sublime_text",
];

pub fn is_dev_tool(app: &str) -> bool {
    // IDE JetBrains в Linux: jetbrains-idea, jetbrains-pycharm…
    DEV_TOOLS.contains(&app) || app.starts_with("jetbrains-")
}

/// Папки, куда лаунчеры ставят игры (путь в нижнем регистре, с `\` или `/`).
#[rustfmt::skip]
const GAME_DIRS: &[&str] = &[
    "/steamapps/common/", "/epic games/", "/gog galaxy/games/", "/gog games/",
    "/ubisoft game launcher/games/", "/ea games/", "/origin games/", "/riot games/",
    "/xboxgames/", "/itch/apps/",
];

/// Сами лаунчеры лежат рядом с играми, но в них печатают (поиск, чат) — это не игры.
#[rustfmt::skip]
const LAUNCHER_DIRS: &[&str] = &[
    "/epic games/launcher/", "/riot games/riot client/", "/ea desktop/",
];

/// Игра по пути к её exe: установлена лаунчером (Steam, Epic, GOG, Ubisoft, EA, Riot, Xbox, itch).
pub fn is_game_path(path: &str) -> bool {
    let p = path.to_lowercase().replace('\\', "/");
    GAME_DIRS.iter().any(|d| p.contains(d)) && !LAUNCHER_DIRS.iter().any(|d| p.contains(d))
}

/// Игра по классу окна в Linux: Steam через Proton (`steam_app_123`), gamescope.
#[cfg_attr(windows, allow(dead_code))]
pub fn is_game_app(app: &str) -> bool {
    app.starts_with("steam_app_") || app == "gamescope"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn games_by_path() {
        assert!(is_game_path(
            r"D:\SteamLibrary\steamapps\common\Counter-Strike Global Offensive\game\bin\win64\cs2.exe"
        ));
        assert!(is_game_path(
            r"C:\Program Files\Epic Games\Fortnite\FortniteGame\Binaries\Win64\FortniteClient.exe"
        ));
        assert!(is_game_path(r"C:\XboxGames\Forza Horizon 5\Content\ForzaHorizon5.exe"));
        assert!(is_game_path("/home/me/.local/share/Steam/steamapps/common/Dota 2/game/bin/dota2"));
        // Сами лаунчеры — не игры: в них печатают.
        assert!(!is_game_path(r"C:\Program Files (x86)\Steam\steam.exe"));
        assert!(!is_game_path(
            r"C:\Program Files (x86)\Epic Games\Launcher\Portal\Binaries\Win64\EpicGamesLauncher.exe"
        ));
        assert!(!is_game_path(r"C:\Riot Games\Riot Client\RiotClientServices.exe"));
        assert!(!is_game_path(r"C:\Program Files\Microsoft VS Code\Code.exe"));
    }

    #[test]
    fn games_by_window_class() {
        assert!(is_game_app("steam_app_730"));
        assert!(is_game_app("gamescope"));
        assert!(!is_game_app("steam"));
        assert!(!is_game_app("kitty"));
    }
}
