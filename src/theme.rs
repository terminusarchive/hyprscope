//! The twenty-six built-in themes. A theme is a `Palette`: one colour for each
//! thing hyprscope colours. `hyprland` is today's palette; the other twenty-five
//! are Crosswalk's ten-colour themes, copied from its `src/ui/theme.rs` and
//! mapped onto the palette (a later change in one program is carried to the
//! other by hand).
use std::cell::Cell;

use ratatui::style::Color;

use crate::graph::NEWEST;
use crate::ui::{
    ACCENT, BAD, BLUE, BLUE_DEEP, CYAN, CYAN_DEEP, DIM, FLAG, FLAG_DEEP, GOOD, GUIDE, MONITOR,
    MUTED, PANEL_BORDER, SELECTED,
};

/// One colour for each thing hyprscope colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    /// Plain text; `Reset` leaves the terminal's own.
    pub text: Color,
    /// The screen's background; `Reset` leaves the terminal's own.
    pub background: Color,
    pub cyan: Color,
    pub blue: Color,
    pub accent: Color,
    pub panel_border: Color,
    pub monitor: Color,
    pub flag: Color,
    pub cyan_deep: Color,
    pub blue_deep: Color,
    pub flag_deep: Color,
    pub muted: Color,
    pub guide: Color,
    pub dim: Color,
    pub bad: Color,
    pub good: Color,
    pub selected: Color,
    pub newest: Color,
}

/// The built-in themes, in the order `t` cycles them.
pub const NAMES: [&str; 26] = [
    "hyprland",
    "synthwave",
    "dracula",
    "nord",
    "gruvbox",
    "catppuccin",
    "tokyo-night",
    "solarized",
    "monokai",
    "one-dark",
    "everforest",
    "rose-pine",
    "kanagawa",
    "ayu",
    "night-owl",
    "github-dark",
    "palenight",
    "solarized-light",
    "gruvbox-light",
    "catppuccin-latte",
    "github-light",
    "mono",
    "amber",
    "green",
    "paper",
    "terminal",
];

/// Crosswalk's ten colours per theme, as `0xrrggbb`: background, text, muted,
/// inbound, outbound, accent, warning, danger, ok, highlight.
const TABLE: [(&str, [u32; 10]); 24] = [
    ("synthwave", [
        0x1a0b2e, 0xf0e6ff, 0x8a7aaa, 0x00f0ff, 0xff2a8a,
        0xb05cff, 0xff9a3c, 0xff3860, 0x3cff9a, 0xffe66d,
    ]),
    ("dracula", [
        0x282a36, 0xf8f8f2, 0x6272a4, 0x8be9fd, 0xff79c6,
        0xbd93f9, 0xffb86c, 0xff5555, 0x50fa7b, 0xf1fa8c,
    ]),
    ("nord", [
        0x2e3440, 0xd8dee9, 0x616e88, 0x88c0d0, 0xb48ead,
        0x81a1c1, 0xd08770, 0xbf616a, 0xa3be8c, 0xebcb8b,
    ]),
    ("gruvbox", [
        0x282828, 0xebdbb2, 0x928374, 0x83a598, 0xd3869b,
        0x8ec07c, 0xfe8019, 0xfb4934, 0xb8bb26, 0xfabd2f,
    ]),
    ("catppuccin", [
        0x1e1e2e, 0xcdd6f4, 0x7f849c, 0x89dceb, 0xf5c2e7,
        0xcba6f7, 0xfab387, 0xf38ba8, 0xa6e3a1, 0xf9e2af,
    ]),
    ("tokyo-night", [
        0x1a1b26, 0xc0caf5, 0x565f89, 0x7dcfff, 0xbb9af7,
        0x7aa2f7, 0xff9e64, 0xf7768e, 0x9ece6a, 0xe0af68,
    ]),
    ("solarized", [
        0x002b36, 0x93a1a1, 0x586e75, 0x2aa198, 0xd33682,
        0x268bd2, 0xcb4b16, 0xdc322f, 0x859900, 0xb58900,
    ]),
    ("monokai", [
        0x272822, 0xf8f8f2, 0x75715e, 0x66d9ef, 0xae81ff,
        0xe6db74, 0xfd971f, 0xf92672, 0xa6e22e, 0xf4bf75,
    ]),
    ("one-dark", [
        0x282c34, 0xabb2bf, 0x5c6370, 0x56b6c2, 0xc678dd,
        0x61afef, 0xd19a66, 0xe06c75, 0x98c379, 0xe5c07b,
    ]),
    ("everforest", [
        0x2d353b, 0xd3c6aa, 0x859289, 0x7fbbb3, 0xd699b6,
        0x83c092, 0xe69875, 0xe67e80, 0xa7c080, 0xdbbc7f,
    ]),
    ("rose-pine", [
        0x191724, 0xe0def4, 0x6e6a86, 0x9ccfd8, 0xebbcba,
        0xc4a7e7, 0xea9a97, 0xeb6f92, 0x31748f, 0xf6c177,
    ]),
    ("kanagawa", [
        0x1f1f28, 0xdcd7ba, 0x727169, 0x7fb4ca, 0x957fb8,
        0x7e9cd8, 0xffa066, 0xe46876, 0x98bb6c, 0xe6c384,
    ]),
    ("ayu", [
        0x0b0e14, 0xbfbdb6, 0x626a73, 0x39bae6, 0xd2a6ff,
        0x59c2ff, 0xff8f40, 0xf07178, 0xaad94c, 0xe6b450,
    ]),
    ("night-owl", [
        0x011627, 0xd6deeb, 0x637777, 0x7fdbca, 0xc792ea,
        0x82aaff, 0xf78c6c, 0xef5350, 0xaddb67, 0xecc48d,
    ]),
    ("github-dark", [
        0x0d1117, 0xc9d1d9, 0x8b949e, 0x39c5cf, 0xbc8cff,
        0x58a6ff, 0xd29922, 0xf85149, 0x3fb950, 0xe3b341,
    ]),
    ("palenight", [
        0x292d3e, 0xa6accd, 0x676e95, 0x89ddff, 0xc792ea,
        0x82aaff, 0xf78c6c, 0xff5370, 0xc3e88d, 0xffcb6b,
    ]),
    ("solarized-light", [
        0xfdf6e3, 0x657b83, 0x93a1a1, 0x2aa198, 0xd33682,
        0x268bd2, 0xcb4b16, 0xdc322f, 0x859900, 0xb58900,
    ]),
    ("gruvbox-light", [
        0xfbf1c7, 0x3c3836, 0x928374, 0x076678, 0x8f3f71,
        0x427b58, 0xaf3a03, 0x9d0006, 0x79740e, 0xb57614,
    ]),
    ("catppuccin-latte", [
        0xeff1f5, 0x4c4f69, 0x8c8fa1, 0x04a5e5, 0xea76cb,
        0x8839ef, 0xfe640b, 0xd20f39, 0x40a02b, 0xdf8e1d,
    ]),
    ("github-light", [
        0xffffff, 0x24292f, 0x6e7781, 0x1b7c83, 0x8250df,
        0x0969da, 0xbc4c00, 0xcf222e, 0x1a7f37, 0x9a6700,
    ]),
    ("mono", [
        0x000000, 0xd0d0d0, 0x707070, 0xffffff, 0xa0a0a0,
        0xffffff, 0xc0c0c0, 0xffffff, 0x909090, 0xffffff,
    ]),
    ("amber", [
        0x140c00, 0xffb000, 0x8a5f00, 0xffcc4d, 0xc88a00,
        0xffd166, 0xffc233, 0xfff0c2, 0xb37c00, 0xffe08a,
    ]),
    ("green", [
        0x020f02, 0x33ff33, 0x1f8f1f, 0x66ff66, 0x22bb22,
        0x99ff99, 0x55ee55, 0xccffcc, 0x2aa82a, 0xb3ffb3,
    ]),
    ("paper", [
        0xf2f0e9, 0x222222, 0x8a8a8a, 0x000000, 0x555555,
        0x000000, 0x444444, 0x000000, 0x777777, 0x000000,
    ]),
];

/// The share of a colour kept when mixing it toward the background.
const BOTTOM: f32 = 0.35;
const GUIDE_KEPT: f32 = 0.50;
const DIM_KEPT: f32 = 0.65;
const SELECTED_KEPT: f32 = 0.30;

/// `colour` with `kept` of it left, the rest the background.
fn mix(colour: Color, background: Color, kept: f32) -> Color {
    let (Color::Rgb(r, g, b), Color::Rgb(br, bg, bb)) = (colour, background) else {
        return colour;
    };
    let channel = |c: u8, back: u8| (back as f32 + (c as f32 - back as f32) * kept).round() as u8;
    Color::Rgb(channel(r, br), channel(g, bg), channel(b, bb))
}

/// A theme from Crosswalk's ten colours, by the spec's mapping table.
fn from_ten(hex: [u32; 10]) -> Palette {
    let [background, text, muted, inbound, outbound, accent, warning, danger, ok, highlight] =
        hex.map(|h| Color::Rgb((h >> 16) as u8, (h >> 8) as u8, h as u8));
    Palette {
        text,
        background,
        cyan: inbound,
        blue: accent,
        accent: highlight,
        panel_border: muted,
        monitor: warning,
        flag: outbound,
        cyan_deep: mix(inbound, background, BOTTOM),
        blue_deep: mix(accent, background, BOTTOM),
        flag_deep: mix(outbound, background, BOTTOM),
        muted,
        guide: mix(muted, background, GUIDE_KEPT),
        dim: mix(muted, background, DIM_KEPT),
        bad: danger,
        good: ok,
        selected: mix(accent, background, SELECTED_KEPT),
        newest: text,
    }
}

/// Today's palette, exactly.
fn hyprland() -> Palette {
    Palette {
        text: Color::Reset,
        background: Color::Reset,
        cyan: CYAN,
        blue: BLUE,
        accent: ACCENT,
        panel_border: PANEL_BORDER,
        monitor: MONITOR,
        flag: FLAG,
        cyan_deep: CYAN_DEEP,
        blue_deep: BLUE_DEEP,
        flag_deep: FLAG_DEEP,
        muted: MUTED,
        guide: GUIDE,
        dim: DIM,
        bad: BAD,
        good: GOOD,
        selected: SELECTED,
        newest: NEWEST,
    }
}

/// The terminal's own text and background and its named colours, with no mixing.
fn terminal() -> Palette {
    Palette {
        text: Color::Reset,
        background: Color::Reset,
        cyan: Color::Cyan,
        blue: Color::LightBlue,
        accent: Color::LightYellow,
        panel_border: Color::DarkGray,
        monitor: Color::Yellow,
        flag: Color::Magenta,
        cyan_deep: Color::Cyan,
        blue_deep: Color::LightBlue,
        flag_deep: Color::Magenta,
        muted: Color::DarkGray,
        guide: Color::DarkGray,
        dim: Color::DarkGray,
        bad: Color::Red,
        good: Color::Green,
        selected: Color::Blue,
        newest: Color::Reset,
    }
}

/// The built-in theme of that name.
pub fn named(name: &str) -> Option<Palette> {
    match name {
        "hyprland" => Some(hyprland()),
        "terminal" => Some(terminal()),
        _ => TABLE.iter().find(|(n, _)| *n == name).map(|(_, hex)| from_ten(*hex)),
    }
}

thread_local! {
    static SHOWING: Cell<usize> = const { Cell::new(0) };
}

/// The palette of the theme showing.
pub fn current() -> Palette {
    named(name()).expect("the showing index is one of NAMES")
}

/// The name of the theme showing.
pub fn name() -> &'static str {
    NAMES[SHOWING.get()]
}

/// Show the theme of that name; false, and nothing changes, when there is none.
pub fn set(name: &str) -> bool {
    match NAMES.iter().position(|n| *n == name) {
        Some(i) => {
            SHOWING.set(i);
            true
        }
        None => false,
    }
}

/// Show the next theme, or the previous, going round.
pub fn step(forward: bool) {
    let n = NAMES.len();
    SHOWING.set((SHOWING.get() + if forward { 1 } else { n - 1 }) % n);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_goes_round_the_names_both_ways() {
        assert_eq!(name(), "hyprland");
        for want in NAMES.iter().skip(1).chain(NAMES.iter().take(1)) {
            step(true);
            assert_eq!(name(), *want);
        }
        step(false);
        assert_eq!(name(), "terminal");
        assert_eq!(current(), named("terminal").unwrap());
    }

    #[test]
    fn set_of_an_unknown_name_changes_nothing() {
        assert!(set("nord"));
        assert!(!set("nope"));
        assert_eq!(name(), "nord");
    }

    fn rgb(h: u32) -> Color {
        Color::Rgb((h >> 16) as u8, (h >> 8) as u8, h as u8)
    }

    fn all(p: &Palette) -> [Color; 18] {
        [
            p.text, p.background, p.cyan, p.blue, p.accent, p.panel_border, p.monitor, p.flag,
            p.cyan_deep, p.blue_deep, p.flag_deep, p.muted, p.guide, p.dim, p.bad, p.good,
            p.selected, p.newest,
        ]
    }

    #[test]
    fn the_names_are_the_twenty_six_in_order() {
        let expected = "hyprland synthwave dracula nord gruvbox catppuccin tokyo-night solarized \
            monokai one-dark everforest rose-pine kanagawa ayu night-owl github-dark palenight \
            solarized-light gruvbox-light catppuccin-latte github-light mono amber green paper \
            terminal";
        assert_eq!(NAMES.to_vec(), expected.split_whitespace().collect::<Vec<_>>());
        for name in NAMES {
            assert!(named(name).is_some(), "{name}");
        }
    }

    #[test]
    fn an_unknown_name_gives_none() {
        assert_eq!(named("nope"), None);
        assert_eq!(named(""), None);
        assert_eq!(named("Dracula"), None);
    }

    #[test]
    fn hyprland_is_todays_palette_with_the_terminals_text_and_background() {
        let p = named("hyprland").unwrap();
        assert_eq!(p.text, Color::Reset);
        assert_eq!(p.background, Color::Reset);
        assert_eq!(
            [
                p.cyan, p.blue, p.accent, p.panel_border, p.monitor, p.flag, p.cyan_deep,
                p.blue_deep, p.flag_deep, p.muted, p.guide, p.dim, p.bad, p.good, p.selected,
                p.newest
            ],
            [
                rgb(0x3fe0ff), rgb(0x4a6cff), rgb(0xffb454), rgb(0x3670b8), rgb(0x8fa8ff),
                rgb(0xb89cff), rgb(0x14507a), rgb(0x1a2a78), rgb(0x3b2a78), rgb(0x8a93a6),
                rgb(0x4b5563), rgb(0x5c6370), rgb(0xff6b6b), rgb(0x6be08a), rgb(0x1a2a55),
                rgb(0xf2f8ff)
            ]
        );
    }

    #[test]
    fn each_theme_has_the_background_and_text_of_the_table() {
        let table: [(&str, u32, u32); 24] = [
            ("synthwave", 0x1a0b2e, 0xf0e6ff), ("dracula", 0x282a36, 0xf8f8f2),
            ("nord", 0x2e3440, 0xd8dee9), ("gruvbox", 0x282828, 0xebdbb2),
            ("catppuccin", 0x1e1e2e, 0xcdd6f4), ("tokyo-night", 0x1a1b26, 0xc0caf5),
            ("solarized", 0x002b36, 0x93a1a1), ("monokai", 0x272822, 0xf8f8f2),
            ("one-dark", 0x282c34, 0xabb2bf), ("everforest", 0x2d353b, 0xd3c6aa),
            ("rose-pine", 0x191724, 0xe0def4), ("kanagawa", 0x1f1f28, 0xdcd7ba),
            ("ayu", 0x0b0e14, 0xbfbdb6), ("night-owl", 0x011627, 0xd6deeb),
            ("github-dark", 0x0d1117, 0xc9d1d9), ("palenight", 0x292d3e, 0xa6accd),
            ("solarized-light", 0xfdf6e3, 0x657b83), ("gruvbox-light", 0xfbf1c7, 0x3c3836),
            ("catppuccin-latte", 0xeff1f5, 0x4c4f69), ("github-light", 0xffffff, 0x24292f),
            ("mono", 0x000000, 0xd0d0d0), ("amber", 0x140c00, 0xffb000),
            ("green", 0x020f02, 0x33ff33), ("paper", 0xf2f0e9, 0x222222),
        ];
        for (name, background, text) in table {
            let p = named(name).unwrap();
            assert_eq!(p.background, rgb(background), "{name} background");
            assert_eq!(p.text, rgb(text), "{name} text");
        }
        let t = named("terminal").unwrap();
        assert_eq!((t.background, t.text), (Color::Reset, Color::Reset));
    }

    #[test]
    fn dracula_maps_every_row_of_the_mapping_table() {
        let p = named("dracula").unwrap();
        assert_eq!(p.text, rgb(0xf8f8f2));
        assert_eq!(p.cyan, rgb(0x8be9fd)); // inbound: titles, first graph, gradient start
        assert_eq!(p.blue, rgb(0xbd93f9)); // accent: second graph, gradient end
        assert_eq!(p.flag, rgb(0xff79c6)); // outbound: flags, third graph
        assert_eq!(p.accent, rgb(0xf1fa8c)); // highlight: the focused window
        assert_eq!(p.monitor, rgb(0xffb86c)); // warning
        assert_eq!(p.bad, rgb(0xff5555)); // danger
        assert_eq!(p.good, rgb(0x50fa7b)); // ok
        assert_eq!(p.muted, rgb(0x6272a4));
        assert_eq!(p.panel_border, rgb(0x6272a4));
        assert_eq!(p.newest, rgb(0xf8f8f2));
        let bg = rgb(0x282a36);
        assert_eq!(p.guide, mix(rgb(0x6272a4), bg, 0.50));
        assert_eq!(p.dim, mix(rgb(0x6272a4), bg, 0.65));
        assert_eq!(p.cyan_deep, mix(rgb(0x8be9fd), bg, 0.35));
        assert_eq!(p.blue_deep, mix(rgb(0xbd93f9), bg, 0.35));
        assert_eq!(p.flag_deep, mix(rgb(0xff79c6), bg, 0.35));
        assert_eq!(p.selected, mix(rgb(0xbd93f9), bg, 0.30));
        // A shade is between the background and its colour, and not either of them.
        assert_ne!(p.guide, bg);
        assert_ne!(p.guide, p.muted);
    }

    #[test]
    fn terminal_holds_no_24_bit_colour_and_selects_in_blue() {
        let p = named("terminal").unwrap();
        for c in all(&p) {
            assert!(!matches!(c, Color::Rgb(..)), "{c:?}");
        }
        assert_eq!(p.selected, Color::Blue);
        assert_eq!(p.cyan_deep, p.cyan);
        assert_eq!(p.blue_deep, p.blue);
        assert_eq!(p.flag_deep, p.flag);
    }

    #[test]
    fn the_selection_and_the_tree_shades_stand_out_in_every_theme_with_a_background() {
        for name in NAMES {
            let p = named(name).unwrap();
            if p.background == Color::Reset {
                continue;
            }
            assert_ne!(p.selected, p.background, "{name} selected vs background");
            assert_ne!(p.selected, p.text, "{name} selected vs text");
            assert_ne!(p.guide, p.background, "{name} guide");
            assert_ne!(p.dim, p.background, "{name} dim");
        }
    }
}
