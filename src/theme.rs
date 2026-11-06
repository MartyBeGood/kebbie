// kebbie draws in a fixed Catppuccin Mocha palette (board::widgets). With theme.follow_herdr on, the colors come from herdr instead: herdr has no command that reports its active theme, so this file resolves it from herdr's config.toml the way herdr does (herdr 0.9.5, src/app/mod.rs and src/config/theme.rs), and every cell in a finished frame that holds one of the palette colors is repainted in the herdr color for that role.

use std::path::Path;
use std::sync::OnceLock;

use ratatui::buffer::Buffer;
use ratatui::style::Color;
use toml::{Table, Value};

use crate::board::widgets::{BLUE, FAINT, GREEN, LAVENDER, MANTLE, MAUVE, MUTED, PEACH, RED, SUBTEXT, SURFACE, TEAL, TEXT, YELLOW};

// Each palette color with the herdr theme token that stands for it, in the order of the colors in BUILT_IN. The darkest surface is panel_bg, the theme's background; sidebar_bg colors only herdr's own sidebar and is Reset in every built-in theme.
const ROLES: [(Color, &str); 14] = [
    (TEXT, "text"),
    (SUBTEXT, "subtext0"),
    (MUTED, "overlay1"),
    (FAINT, "surface1"),
    (SURFACE, "surface0"),
    (MANTLE, "panel_bg"),
    (MAUVE, "mauve"),
    (LAVENDER, "accent"),
    (PEACH, "peach"),
    (YELLOW, "yellow"),
    (GREEN, "green"),
    (BLUE, "blue"),
    (TEAL, "teal"),
    (RED, "red"),
];

// herdr's built-in themes, only the colors kebbie draws with, copied from herdr 0.9.5 (src/app/state.rs). Catppuccin is not in the list: kebbie's own palette is Catppuccin Mocha, and herdr's accent for it is blue where kebbie draws lavender, so the default theme changes nothing.
#[rustfmt::skip]
const BUILT_IN: [(&str, [Color; 14]); 17] = [
    ("catppuccin-latte", [Color::Rgb(76, 79, 105), Color::Rgb(108, 111, 133), Color::Rgb(140, 143, 161), Color::Rgb(188, 192, 204), Color::Rgb(204, 208, 218), Color::Rgb(239, 241, 245), Color::Rgb(136, 57, 239), Color::Rgb(30, 102, 245), Color::Rgb(254, 100, 11), Color::Rgb(223, 142, 29), Color::Rgb(64, 160, 43), Color::Rgb(30, 102, 245), Color::Rgb(23, 146, 153), Color::Rgb(210, 15, 57)]),
    ("terminal", [Color::Reset, Color::Gray, Color::White, Color::DarkGray, Color::Reset, Color::Reset, Color::Gray, Color::Blue, Color::Yellow, Color::Yellow, Color::Green, Color::Blue, Color::Cyan, Color::LightRed]),
    ("tokyo-night", [Color::Rgb(192, 202, 245), Color::Rgb(169, 177, 214), Color::Rgb(105, 113, 150), Color::Rgb(65, 72, 104), Color::Rgb(36, 40, 59), Color::Rgb(26, 27, 38), Color::Rgb(187, 154, 247), Color::Rgb(122, 162, 247), Color::Rgb(255, 158, 100), Color::Rgb(224, 175, 104), Color::Rgb(158, 206, 106), Color::Rgb(122, 162, 247), Color::Rgb(125, 207, 255), Color::Rgb(247, 118, 142)]),
    ("tokyo-night-day", [Color::Rgb(55, 96, 191), Color::Rgb(97, 114, 176), Color::Rgb(104, 112, 154), Color::Rgb(168, 174, 203), Color::Rgb(196, 200, 218), Color::Rgb(225, 226, 231), Color::Rgb(120, 71, 189), Color::Rgb(46, 125, 233), Color::Rgb(177, 92, 0), Color::Rgb(140, 108, 62), Color::Rgb(88, 117, 57), Color::Rgb(46, 125, 233), Color::Rgb(17, 140, 116), Color::Rgb(245, 42, 101)]),
    ("dracula", [Color::Rgb(248, 248, 242), Color::Rgb(210, 210, 220), Color::Rgb(130, 140, 180), Color::Rgb(98, 114, 164), Color::Rgb(68, 71, 90), Color::Rgb(40, 42, 54), Color::Rgb(255, 121, 198), Color::Rgb(189, 147, 249), Color::Rgb(255, 184, 108), Color::Rgb(241, 250, 140), Color::Rgb(80, 250, 123), Color::Rgb(139, 233, 253), Color::Rgb(139, 233, 253), Color::Rgb(255, 85, 85)]),
    ("nord", [Color::Rgb(236, 239, 244), Color::Rgb(216, 222, 233), Color::Rgb(100, 110, 130), Color::Rgb(67, 76, 94), Color::Rgb(59, 66, 82), Color::Rgb(46, 52, 64), Color::Rgb(180, 142, 173), Color::Rgb(136, 192, 208), Color::Rgb(208, 135, 112), Color::Rgb(235, 203, 139), Color::Rgb(163, 190, 140), Color::Rgb(129, 161, 193), Color::Rgb(143, 188, 187), Color::Rgb(191, 97, 106)]),
    ("gruvbox", [Color::Rgb(235, 219, 178), Color::Rgb(213, 196, 161), Color::Rgb(168, 153, 132), Color::Rgb(80, 73, 69), Color::Rgb(60, 56, 54), Color::Rgb(40, 40, 40), Color::Rgb(211, 134, 155), Color::Rgb(215, 153, 33), Color::Rgb(254, 128, 25), Color::Rgb(250, 189, 47), Color::Rgb(184, 187, 38), Color::Rgb(131, 165, 152), Color::Rgb(142, 192, 124), Color::Rgb(251, 73, 52)]),
    ("gruvbox-light", [Color::Rgb(60, 56, 54), Color::Rgb(80, 73, 69), Color::Rgb(124, 111, 100), Color::Rgb(213, 196, 161), Color::Rgb(235, 219, 178), Color::Rgb(251, 241, 199), Color::Rgb(143, 63, 113), Color::Rgb(7, 102, 120), Color::Rgb(175, 58, 3), Color::Rgb(181, 118, 20), Color::Rgb(121, 116, 14), Color::Rgb(7, 102, 120), Color::Rgb(66, 123, 88), Color::Rgb(157, 0, 6)]),
    ("one-dark", [Color::Rgb(171, 178, 191), Color::Rgb(150, 156, 168), Color::Rgb(115, 122, 135), Color::Rgb(62, 68, 81), Color::Rgb(44, 49, 58), Color::Rgb(40, 44, 52), Color::Rgb(198, 120, 221), Color::Rgb(97, 175, 239), Color::Rgb(209, 154, 102), Color::Rgb(229, 192, 123), Color::Rgb(152, 195, 121), Color::Rgb(97, 175, 239), Color::Rgb(86, 182, 194), Color::Rgb(224, 108, 117)]),
    ("one-light", [Color::Rgb(56, 58, 66), Color::Rgb(104, 107, 119), Color::Rgb(104, 107, 119), Color::Rgb(229, 229, 230), Color::Rgb(240, 240, 241), Color::Rgb(250, 250, 250), Color::Rgb(166, 38, 164), Color::Rgb(64, 120, 242), Color::Rgb(152, 104, 1), Color::Rgb(193, 132, 1), Color::Rgb(80, 161, 79), Color::Rgb(64, 120, 242), Color::Rgb(1, 132, 188), Color::Rgb(228, 86, 73)]),
    ("solarized", [Color::Rgb(147, 161, 161), Color::Rgb(131, 148, 150), Color::Rgb(101, 123, 131), Color::Rgb(88, 110, 117), Color::Rgb(7, 54, 66), Color::Rgb(0, 43, 54), Color::Rgb(211, 54, 130), Color::Rgb(38, 139, 210), Color::Rgb(203, 75, 22), Color::Rgb(181, 137, 0), Color::Rgb(133, 153, 0), Color::Rgb(38, 139, 210), Color::Rgb(42, 161, 152), Color::Rgb(220, 50, 47)]),
    ("solarized-light", [Color::Rgb(101, 123, 131), Color::Rgb(131, 148, 150), Color::Rgb(88, 110, 117), Color::Rgb(147, 161, 161), Color::Rgb(238, 232, 213), Color::Rgb(253, 246, 227), Color::Rgb(211, 54, 130), Color::Rgb(38, 139, 210), Color::Rgb(203, 75, 22), Color::Rgb(181, 137, 0), Color::Rgb(133, 153, 0), Color::Rgb(38, 139, 210), Color::Rgb(42, 161, 152), Color::Rgb(220, 50, 47)]),
    ("kanagawa", [Color::Rgb(220, 215, 186), Color::Rgb(200, 195, 170), Color::Rgb(135, 134, 125), Color::Rgb(54, 54, 70), Color::Rgb(42, 42, 55), Color::Rgb(31, 31, 40), Color::Rgb(149, 127, 184), Color::Rgb(126, 156, 216), Color::Rgb(255, 160, 102), Color::Rgb(192, 163, 110), Color::Rgb(118, 148, 106), Color::Rgb(126, 156, 216), Color::Rgb(127, 180, 202), Color::Rgb(195, 64, 67)]),
    ("kanagawa-lotus", [Color::Rgb(84, 84, 100), Color::Rgb(67, 67, 108), Color::Rgb(138, 137, 128), Color::Rgb(201, 203, 209), Color::Rgb(220, 213, 172), Color::Rgb(242, 236, 188), Color::Rgb(98, 76, 131), Color::Rgb(77, 105, 155), Color::Rgb(204, 109, 0), Color::Rgb(119, 113, 63), Color::Rgb(111, 137, 78), Color::Rgb(77, 105, 155), Color::Rgb(78, 140, 162), Color::Rgb(200, 64, 83)]),
    ("rose-pine", [Color::Rgb(224, 222, 244), Color::Rgb(200, 197, 220), Color::Rgb(144, 140, 170), Color::Rgb(38, 35, 58), Color::Rgb(31, 29, 46), Color::Rgb(25, 23, 36), Color::Rgb(196, 167, 231), Color::Rgb(196, 167, 231), Color::Rgb(234, 154, 151), Color::Rgb(246, 193, 119), Color::Rgb(49, 116, 143), Color::Rgb(49, 116, 143), Color::Rgb(156, 207, 216), Color::Rgb(235, 111, 146)]),
    ("rose-pine-dawn", [Color::Rgb(70, 66, 97), Color::Rgb(121, 117, 147), Color::Rgb(121, 117, 147), Color::Rgb(255, 250, 243), Color::Rgb(242, 233, 225), Color::Rgb(250, 244, 237), Color::Rgb(144, 122, 169), Color::Rgb(144, 122, 169), Color::Rgb(215, 130, 126), Color::Rgb(234, 157, 52), Color::Rgb(40, 105, 131), Color::Rgb(40, 105, 131), Color::Rgb(86, 148, 159), Color::Rgb(180, 99, 122)]),
    ("vesper", [Color::Rgb(255, 255, 255), Color::Rgb(160, 160, 160), Color::Rgb(126, 126, 126), Color::Rgb(40, 40, 40), Color::Rgb(35, 35, 35), Color::Rgb(26, 26, 26), Color::Rgb(255, 209, 168), Color::Rgb(255, 199, 153), Color::Rgb(255, 199, 153), Color::Rgb(255, 199, 153), Color::Rgb(153, 255, 228), Color::Rgb(176, 176, 176), Color::Rgb(102, 221, 204), Color::Rgb(255, 128, 128)]),
];

#[derive(Default)]
pub struct Theme {
    swaps: Vec<(Color, Color)>,
}

impl Theme {
    // The colors herdr resolves from its config text for a dark or light appearance. A config that does not parse changes nothing.
    pub fn from_config(config: &str, dark: bool) -> Theme {
        let Ok(config) = config.parse::<Table>() else { return Theme::default() };
        let theme = config.get("theme").and_then(Value::as_table);
        let text = |key: &str| theme.and_then(|theme| theme.get(key)).and_then(Value::as_str);
        let name = text("name").unwrap_or("catppuccin");
        let auto_switch = theme.and_then(|theme| theme.get("auto_switch")).and_then(Value::as_bool).unwrap_or(false);
        let custom = theme.and_then(|theme| theme.get("custom")).and_then(Value::as_table);
        let (default_dark, default_light) = sibling_theme_names(name);
        // herdr: with auto_switch the dark or light name, else the manual one; an unknown name falls back to Catppuccin (Latte for light). Only auto_switch layers the mode table.
        let (base_name, fallback, mode) = match (auto_switch, dark) {
            (false, _) => (name, "catppuccin", None),
            (true, true) => (text("dark_name").unwrap_or(&default_dark), "catppuccin", custom.and_then(|custom| custom.get("dark"))),
            (true, false) => (text("light_name").unwrap_or(&default_light), "catppuccin-latte", custom.and_then(|custom| custom.get("light"))),
        };
        let mode = mode.and_then(Value::as_table);
        let base = built_in(base_name).or_else(|| built_in(fallback)).unwrap_or_else(own_colors);
        // herdr's legacy [ui] accent stands in for a custom accent that is not set.
        let legacy_accent = config.get("ui").and_then(Value::as_table).and_then(|ui| ui.get("accent")).and_then(Value::as_str);
        let legacy_accent = legacy_accent.filter(|accent| *accent != "cyan" && custom.and_then(|custom| custom.get("accent")).and_then(Value::as_str).is_none());
        fn value<'a>(table: Option<&'a Table>, token: &str) -> Option<&'a str> {
            table.and_then(|table| table.get(token)).and_then(Value::as_str)
        }
        let swaps = ROLES.iter().zip(base)
            .filter_map(|((palette, token), base)| {
                let set = value(mode, token).or(value(custom, token)).or(legacy_accent.filter(|_| *token == "accent"));
                let color = set.map_or(base, parse_color);
                (color != *palette).then_some((*palette, color))
            })
            .collect();
        Theme { swaps }
    }

    pub fn apply(&self, buffer: &mut Buffer) {
        if self.swaps.is_empty() {
            return;
        }
        let swap = |color: Color| self.swaps.iter().find(|(palette, _)| *palette == color).map_or(color, |(_, themed)| *themed);
        for cell in &mut buffer.content {
            cell.fg = swap(cell.fg);
            cell.bg = swap(cell.bg);
        }
    }
}

fn own_colors() -> [Color; 14] {
    ROLES.map(|(color, _)| color)
}

fn built_in(name: &str) -> Option<[Color; 14]> {
    let name = canonical_theme_name(name)?;
    if name == "catppuccin" {
        return Some(own_colors());
    }
    BUILT_IN.iter().find(|(candidate, _)| *candidate == name).map(|(_, colors)| *colors)
}

// herdr's canonical_theme_name.
fn canonical_theme_name(name: &str) -> Option<&'static str> {
    match name.to_lowercase().replace([' ', '_'], "-").as_str() {
        "catppuccin" | "catppuccin-mocha" => Some("catppuccin"),
        "catppuccin-latte" | "latte" | "light" => Some("catppuccin-latte"),
        "terminal" => Some("terminal"),
        "tokyo-night" | "tokyonight" => Some("tokyo-night"),
        "tokyo-night-day" | "tokyo-day" | "tokyonight-day" => Some("tokyo-night-day"),
        "dracula" => Some("dracula"),
        "nord" => Some("nord"),
        "gruvbox" | "gruvbox-dark" => Some("gruvbox"),
        "gruvbox-light" => Some("gruvbox-light"),
        "one-dark" | "onedark" => Some("one-dark"),
        "one-light" | "onelight" => Some("one-light"),
        "solarized" | "solarized-dark" => Some("solarized"),
        "solarized-light" => Some("solarized-light"),
        "kanagawa" => Some("kanagawa"),
        "kanagawa-lotus" | "lotus" => Some("kanagawa-lotus"),
        "rose-pine" | "rosepine" => Some("rose-pine"),
        "rose-pine-dawn" | "rosepine-dawn" | "dawn" => Some("rose-pine-dawn"),
        "vesper" => Some("vesper"),
        _ => None,
    }
}

// herdr's sibling_theme_names: the dark and light theme that go with a theme name when dark_name or light_name is not set.
fn sibling_theme_names(name: &str) -> (String, String) {
    let pair = |dark: &str, light: &str| (dark.to_string(), light.to_string());
    match name.to_lowercase().replace([' ', '_'], "-").as_str() {
        "catppuccin" | "catppuccin-mocha" | "catppuccin-latte" | "latte" | "light" => pair("catppuccin", "catppuccin-latte"),
        "tokyo-night" | "tokyonight" | "tokyo-night-day" | "tokyo-day" | "tokyonight-day" => pair("tokyo-night", "tokyo-night-day"),
        "gruvbox" | "gruvbox-dark" | "gruvbox-light" => pair("gruvbox", "gruvbox-light"),
        "one-dark" | "onedark" | "one-light" | "onelight" => pair("one-dark", "one-light"),
        "solarized" | "solarized-dark" | "solarized-light" => pair("solarized", "solarized-light"),
        "kanagawa" | "kanagawa-lotus" | "lotus" => pair("kanagawa", "kanagawa-lotus"),
        "rose-pine" | "rosepine" | "rose-pine-dawn" | "rosepine-dawn" | "dawn" => pair("rose-pine", "rose-pine-dawn"),
        _ => (name.to_string(), name.to_string()),
    }
}

// herdr's parse_color: hex (#rrggbb, #rgb), rgb(r,g,b), reset aliases and color names; anything else is cyan.
fn parse_color(text: &str) -> Color {
    let text = text.trim().to_lowercase();
    if matches!(text.as_str(), "reset" | "default" | "none" | "transparent") {
        return Color::Reset;
    }
    if let Some(hex) = text.strip_prefix('#') {
        let channel = |at: usize| hex.get(at..at + 2).and_then(|digits| u8::from_str_radix(digits, 16).ok());
        if hex.len() == 6 {
            if let (Some(r), Some(g), Some(b)) = (channel(0), channel(2), channel(4)) {
                return Color::Rgb(r, g, b);
            }
        } else if hex.len() == 3 {
            let digits: Vec<u8> = hex.chars().filter_map(|digit| digit.to_digit(16)).map(|digit| digit as u8 * 17).collect();
            if let [r, g, b] = digits[..] {
                return Color::Rgb(r, g, b);
            }
        }
    }
    if let Some(inner) = text.strip_prefix("rgb(").and_then(|rest| rest.strip_suffix(')')) {
        let parts: Vec<Option<u8>> = inner.split(',').map(|part| part.trim().parse().ok()).collect();
        if let [Some(r), Some(g), Some(b)] = parts[..] {
            return Color::Rgb(r, g, b);
        }
    }
    match text.as_str() {
        "black" => Color::Black,
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" | "purple" => Color::Magenta,
        "white" => Color::White,
        "gray" | "grey" => Color::Gray,
        "darkgray" | "darkgrey" => Color::DarkGray,
        "lightred" => Color::LightRed,
        "lightgreen" => Color::LightGreen,
        "lightyellow" => Color::LightYellow,
        "lightblue" => Color::LightBlue,
        "lightmagenta" => Color::LightMagenta,
        "lightcyan" => Color::LightCyan,
        _ => Color::Cyan,
    }
}

// Dark unless macOS says otherwise; herdr asks the host terminal for the appearance and assumes dark when it gets no answer.
fn dark_appearance() -> bool {
    if !cfg!(target_os = "macos") {
        return true;
    }
    let style = std::process::Command::new("defaults").args(["read", "-g", "AppleInterfaceStyle"]).output();
    style.map_or(true, |output| !output.status.success() || output.stdout.starts_with(b"Dark"))
}

fn load() -> Theme {
    // Tests draw with the palette as it is, whatever config the machine has.
    if cfg!(test) || !crate::config::follow_herdr_enabled().unwrap_or(false) {
        return Theme::default();
    }
    let Some(home) = std::env::var_os("HOME") else { return Theme::default() };
    let path = crate::setup::herdr_config_path(Path::new(&home));
    std::fs::read_to_string(path).map_or_else(|_| Theme::default(), |config| Theme::from_config(&config, dark_appearance()))
}

// Read once, when the first frame is drawn: a changed setting, herdr config or appearance shows after the board restarts.
pub fn apply(buffer: &mut Buffer) {
    static THEME: OnceLock<Theme> = OnceLock::new();
    THEME.get_or_init(load).apply(buffer);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;

    fn swapped(theme: &Theme, color: Color) -> Color {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 1, 1));
        buffer[(0, 0)].fg = color;
        theme.apply(&mut buffer);
        buffer[(0, 0)].fg
    }

    fn resolved(config: &str, dark: bool, color: Color) -> Color {
        swapped(&Theme::from_config(config, dark), color)
    }

    #[test]
    fn no_theme_set_keeps_the_catppuccin_palette() {
        for config in ["", "onboarding = false\n", "[theme]\nname = \"catppuccin\"\n", "[theme]\nname = \"Catppuccin Mocha\"\nauto_switch = true\n", "[theme]\nname = \"no-such-theme\"\n", "[theme\ntext = ", "theme = 3\n"] {
            assert!(Theme::from_config(config, true).swaps.is_empty(), "{config:?}");
        }
    }

    #[test]
    fn a_named_theme_supplies_the_colors_nothing_overrides() {
        let config = "[theme]\nname = \"nord\"\n";
        assert_eq!(resolved(config, true, TEXT), Color::Rgb(236, 239, 244));
        assert_eq!(resolved(config, true, MANTLE), Color::Rgb(46, 52, 64));
        assert_eq!(resolved(config, true, LAVENDER), Color::Rgb(136, 192, 208));
    }

    #[test]
    fn theme_names_resolve_like_herdr_does() {
        assert_eq!(resolved("[theme]\nname = \"Tokyo_Night\"\n", true, TEXT), Color::Rgb(192, 202, 245));
        assert_eq!(resolved("[theme]\nname = \"latte\"\n", true, TEXT), Color::Rgb(76, 79, 105));
    }

    #[test]
    fn a_custom_color_wins_over_the_named_theme() {
        let config = "[theme]\nname = \"nord\"\n[theme.custom]\ntext = \"#111111\"\n";
        assert_eq!(resolved(config, true, TEXT), Color::Rgb(0x11, 0x11, 0x11));
        assert_eq!(resolved(config, true, SUBTEXT), Color::Rgb(216, 222, 233));
    }

    #[test]
    fn a_custom_color_over_catppuccin_leaves_the_other_palette_colors() {
        let config = "[theme.custom]\nmauve = \"#bf5af2\"\n";
        assert_eq!(resolved(config, true, MAUVE), Color::Rgb(0xbf, 0x5a, 0xf2));
        assert_eq!(resolved(config, true, GREEN), GREEN);
    }

    #[test]
    fn the_mode_table_wins_over_custom_and_the_theme_only_with_auto_switch() {
        let config = "[theme]\nname = \"nord\"\nauto_switch = true\ndark_name = \"dracula\"\n[theme.custom]\ntext = \"#111111\"\nred = \"#222222\"\n[theme.custom.dark]\ntext = \"#333333\"\n[theme.custom.light]\ntext = \"#444444\"\n";
        assert_eq!(resolved(config, true, TEXT), Color::Rgb(0x33, 0x33, 0x33));
        assert_eq!(resolved(config, false, TEXT), Color::Rgb(0x44, 0x44, 0x44));
        assert_eq!(resolved(config, true, RED), Color::Rgb(0x22, 0x22, 0x22));
        assert_eq!(resolved(config, true, GREEN), Color::Rgb(80, 250, 123), "dracula, the dark_name");
        let manual = config.replace("auto_switch = true", "auto_switch = false");
        assert_eq!(resolved(&manual, true, TEXT), Color::Rgb(0x11, 0x11, 0x11));
        assert_eq!(resolved(&manual, true, GREEN), Color::Rgb(163, 190, 140), "nord, the name");
    }

    #[test]
    fn auto_switch_picks_the_sibling_of_the_theme_name_unless_dark_and_light_name_are_set() {
        let config = "[theme]\nname = \"gruvbox\"\nauto_switch = true\n";
        assert_eq!(resolved(config, true, MANTLE), Color::Rgb(40, 40, 40));
        assert_eq!(resolved(config, false, MANTLE), Color::Rgb(251, 241, 199));
        let named = "[theme]\nname = \"gruvbox\"\nauto_switch = true\nlight_name = \"nord\"\n";
        assert_eq!(resolved(named, false, MANTLE), Color::Rgb(46, 52, 64));
    }

    #[test]
    fn an_unknown_theme_name_falls_back_to_catppuccin_and_latte_for_light() {
        let config = "[theme]\nauto_switch = true\ndark_name = \"nope\"\nlight_name = \"nope\"\n";
        assert!(Theme::from_config(config, true).swaps.is_empty());
        assert_eq!(resolved(config, false, TEXT), Color::Rgb(76, 79, 105));
    }

    #[test]
    fn the_legacy_ui_accent_stands_in_for_a_custom_accent_that_is_not_set() {
        assert_eq!(resolved("[ui]\naccent = \"#ff0000\"\n", true, LAVENDER), Color::Rgb(255, 0, 0));
        assert_eq!(resolved("[ui]\naccent = \"cyan\"\n", true, LAVENDER), LAVENDER);
        let custom = "[ui]\naccent = \"#ff0000\"\n[theme.custom]\naccent = \"#00ff00\"\n";
        assert_eq!(resolved(custom, true, LAVENDER), Color::Rgb(0, 255, 0));
    }

    #[test]
    fn the_panel_color_is_the_darkest_surface_and_the_sidebar_color_is_not_used() {
        let config = "[theme.custom]\npanel_bg = \"#111111\"\nsidebar_bg = \"#222222\"\n";
        assert_eq!(resolved(config, true, MANTLE), Color::Rgb(0x11, 0x11, 0x11));
    }

    #[test]
    fn colors_are_read_the_way_herdr_reads_them() {
        assert_eq!(parse_color("#ABC"), Color::Rgb(0xaa, 0xbb, 0xcc));
        assert_eq!(parse_color(" rgb( 1, 2 ,3) "), Color::Rgb(1, 2, 3));
        assert_eq!(parse_color("RED"), Color::Red);
        assert_eq!(parse_color("reset"), Color::Reset);
        assert_eq!(parse_color("#12345"), Color::Cyan);
        assert_eq!(parse_color("#ééé"), Color::Cyan);
        assert_eq!(parse_color("rgb(1,2,300)"), Color::Cyan);
        assert_eq!(parse_color("nonsense"), Color::Cyan);
        assert_eq!(resolved("[theme.custom]\ntext = \"reset\"\n", true, TEXT), Color::Reset);
    }

    #[test]
    fn a_value_that_is_not_text_changes_nothing() {
        let theme = Theme::from_config("[theme.custom]\ngreen = 5\nred = true\n", true);
        assert!(theme.swaps.is_empty());
    }

    #[test]
    fn the_terminal_theme_draws_in_the_terminal_colors() {
        let config = "[theme]\nname = \"terminal\"\n";
        assert_eq!(resolved(config, true, TEXT), Color::Reset);
        assert_eq!(resolved(config, true, RED), Color::LightRed);
    }

    #[test]
    fn apply_repaints_foreground_and_background_and_leaves_other_colors() {
        let theme = Theme::from_config("[theme.custom]\ntext = \"#ffffff\"\naccent = \"#0a84ff\"\n", true);
        let mut buffer = Buffer::empty(Rect::new(0, 0, 2, 1));
        buffer[(0, 0)].fg = TEXT;
        buffer[(0, 0)].bg = LAVENDER;
        buffer[(1, 0)].fg = Color::Rgb(1, 2, 3);
        buffer[(1, 0)].bg = Color::Reset;
        theme.apply(&mut buffer);
        assert_eq!((buffer[(0, 0)].fg, buffer[(0, 0)].bg), (Color::Rgb(0xff, 0xff, 0xff), Color::Rgb(0x0a, 0x84, 0xff)));
        assert_eq!((buffer[(1, 0)].fg, buffer[(1, 0)].bg), (Color::Rgb(1, 2, 3), Color::Reset));
    }
}
