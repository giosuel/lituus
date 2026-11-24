//! The palette: the built-in dark and light themes, and palettes made from matugen colour roles,
//! mapped exactly as in the mockups. Warnings keep a fixed amber, so they never read as errors.

use std::collections::HashMap;
use std::path::Path;

use crate::config::{Builtin, ColourSource};

/// RGB, 0–255.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub fn parse(hex: &str) -> Option<Self> {
        let hex = hex.strip_prefix('#')?;
        if hex.len() != 6 {
            return None;
        }
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
        Some(Self(byte(0)?, byte(2)?, byte(4)?))
    }

    const fn hex(value: u32) -> Self {
        Self((value >> 16) as u8, (value >> 8) as u8, value as u8)
    }

    /// WCAG relative luminance.
    fn luminance(self) -> f64 {
        let channel = |c: u8| {
            let c = f64::from(c) / 255.0;
            if c <= 0.03928 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * channel(self.0) + 0.7152 * channel(self.1) + 0.0722 * channel(self.2)
    }

    /// WCAG contrast ratio, 1–21.
    pub fn contrast(self, other: Self) -> f64 {
        let (a, b) = (self.luminance(), other.luminance());
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }
}

macro_rules! palette {
    ($($name:ident),* $(,)?) => {
        /// Every colour the UI uses; the names match the `Theme` global in ui/theme.slint.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub struct Palette { $(pub $name: Rgb),* }
    };
}

palette!(
    bg,
    panel,
    surface,
    border,
    border2,
    ctrl_border,
    ctrl_bg,
    input_bg,
    text,
    muted,
    faint,
    icon_soft,
    accent,
    on_accent,
    accent_soft_bg,
    accent_soft_border,
    off_track,
    knob_off,
    bar_off,
    sel_bg,
    danger,
    warn_bg,
    warn_border,
    warn_text,
    glow,
);

const AMBER_DARK: (Rgb, Rgb, Rgb) = (Rgb::hex(0x2a2518), Rgb::hex(0x5a4c25), Rgb::hex(0xe9cf8f));
const AMBER_LIGHT: (Rgb, Rgb, Rgb) = (Rgb::hex(0xfbf4e3), Rgb::hex(0xe8d49b), Rgb::hex(0x6b5410));

/// The Material roles a palette is made from, in the order of the tables below.
const ROLES: [&str; 15] = [
    "surface",
    "surface_container_lowest",
    "surface_container_low",
    "surface_container",
    "surface_container_high",
    "surface_container_highest",
    "on_surface",
    "on_surface_variant",
    "outline",
    "outline_variant",
    "primary",
    "on_primary",
    "primary_container",
    "secondary_container",
    "error",
];

/// The built-in themes as Material roles, so they go through the same mapping as matugen's.
/// Made with a tonal palette per hue (CIELCh, tone = L*), as matugen does. Every dark theme has
/// the neutral greys of the website's hero image (Teal first, the owner, 2026-11-18; the others
/// too, 2026-11-23), each with its own accent; its containers are the accent mixed into the
/// surface, 30 % and 12 %, as Teal's are.
const BUILTIN_ROLES: [(&str, [&str; 15]); 10] = [
    (
        "teal-dark",
        [
            "#141619", "#1f2226", "#111315", "#1b1e21", "#23272b", "#2f343a", "#e8eaed", "#a0a7af", "#8b929a",
            "#353a40", "#5cc8b0", "#0c1513", "#2c4a42", "#1f2a27", "#f08a7a",
        ],
    ),
    (
        "teal-light",
        [
            "#f1fcf8", "#ffffff", "#ebf6f3", "#e6f0ed", "#e0eae7", "#dae5e2", "#161d1b", "#394a45", "#5c6e69",
            "#b7cbc5", "#036b59", "#ffffff", "#82f7da", "#c1ebdf", "#ba1a1a",
        ],
    ),
    (
        "blue-dark",
        [
            "#141619", "#1f2226", "#111315", "#1b1e21", "#23272b", "#2f343a", "#e8eaed", "#a0a7af", "#8b929a",
            "#353a40", "#9ecafc", "#003256", "#3d4c5d", "#252c34", "#ffb4ab",
        ],
    ),
    (
        "blue-light",
        [
            "#f5faff", "#ffffff", "#eef4fb", "#e8eff5", "#e2e9ef", "#dde3ea", "#171c21", "#3c4852", "#606c77",
            "#bbc8d5", "#09658f", "#ffffff", "#cbe6fe", "#cbe6fe", "#ba1a1a",
        ],
    ),
    (
        "lavender-dark",
        [
            "#141619", "#1f2226", "#111315", "#1b1e21", "#23272b", "#2f343a", "#e8eaed", "#a0a7af", "#8b929a",
            "#353a40", "#d2bbff", "#352661", "#4d485e", "#2b2a35", "#ffb4ab",
        ],
    ),
    (
        "lavender-light",
        [
            "#fbf8fe", "#ffffff", "#f5f2fa", "#efedf4", "#eae7ee", "#e4e1e9", "#1d1b20", "#494551", "#6d6875",
            "#cac4d3", "#675394", "#ffffff", "#e9ddff", "#e9defb", "#ba1a1a",
        ],
    ),
    (
        "coral-dark",
        [
            "#141619", "#1f2226", "#111315", "#1b1e21", "#23272b", "#2f343a", "#e8eaed", "#a0a7af", "#8b929a",
            "#353a40", "#ffb4a7", "#6a0002", "#5a4544", "#30292a", "#ffb4ab",
        ],
    ),
    (
        "coral-light",
        [
            "#fef8f7", "#ffffff", "#fcf1ef", "#f6ece9", "#f1e6e4", "#ebe0de", "#211a18", "#544340", "#796763",
            "#d6c2be", "#ac3229", "#ffffff", "#ffdad4", "#ffdad4", "#ba1a1a",
        ],
    ),
    (
        "mono-dark",
        [
            "#141619", "#1f2226", "#111315", "#1b1e21", "#23272b", "#2f343a", "#e8eaed", "#a0a7af", "#8b929a",
            "#353a40", "#e2e2e2", "#1b1b1b", "#525355", "#2d2e31", "#ffb4ab",
        ],
    ),
    (
        "mono-light",
        [
            "#f9f9f9", "#ffffff", "#f3f3f3", "#eeeeee", "#e8e8e8", "#e2e2e2", "#1b1b1b", "#474747", "#6a6a6a",
            "#c6c6c6", "#303030", "#ffffff", "#d4d4d4", "#dddddd", "#ba1a1a",
        ],
    ),
];

impl Palette {
    pub fn builtin(builtin: Builtin, dark: bool) -> Self {
        let key = format!("{}-{}", builtin.key(), if dark { "dark" } else { "light" });
        let (_, values) = BUILTIN_ROLES.iter().find(|(k, _)| *k == key).expect("every built-in theme has both modes");
        let roles: HashMap<String, String> =
            ROLES.iter().zip(values).map(|(r, v)| ((*r).to_owned(), (*v).to_owned())).collect();
        Self::from_material(&roles).expect("the built-in themes are readable")
    }

    /// From matugen's Material colour roles (as in ML4W's colors.json). `None` if a role is
    /// missing or the result isn't readable (text below 4.5:1 on its background).
    pub fn from_material(roles: &HashMap<String, String>) -> Option<Self> {
        let role = |name: &str| roles.get(name).and_then(|hex| Rgb::parse(hex));
        // Warnings keep their amber, a dark or a light one to suit the surface.
        let dark = role("surface")?.luminance() < 0.2;
        let amber = if dark { AMBER_DARK } else { AMBER_LIGHT };
        let backgrounds = [
            role("surface")?,
            role("surface_container_low")?,
            role("surface_container")?,
            role("surface_container_lowest")?,
        ];
        let palette = Self {
            bg: role("surface")?,
            panel: role("surface_container_low")?,
            surface: role("surface_container")?,
            border: role("surface_container_high")?,
            border2: role("surface_container_highest")?,
            ctrl_border: control_edge(role("outline_variant")?, role("outline")?, &backgrounds),
            ctrl_bg: role("surface_container_high")?,
            input_bg: role("surface_container_lowest")?,
            text: role("on_surface")?,
            muted: role("on_surface_variant")?,
            faint: role("outline")?,
            icon_soft: role("on_surface_variant")?,
            accent: role("primary")?,
            on_accent: role("on_primary")?,
            accent_soft_bg: role("secondary_container")?,
            accent_soft_border: role("primary_container")?,
            off_track: role("outline_variant")?,
            knob_off: role("outline")?,
            bar_off: role("outline_variant")?,
            sel_bg: role("surface_container_high")?,
            danger: role("error")?,
            warn_bg: amber.0,
            warn_border: amber.1,
            warn_text: amber.2,
            glow: mix(role("surface_container")?, role("primary_container")?, if dark { 0.45 } else { 0.85 }),
        };
        palette.readable().then_some(palette)
    }

    /// From the wallpaper's colours (matugen's roles): only the accent. The surfaces, text and
    /// edges are the built-in neutral greys, Teal dark's or Mono light's (the owner, 2026-11-23,
    /// D-45: "can we make it so the wallpaper only influences the accent colors, kinda like our
    /// own themes do?"); the containers are the accent mixed into the surface, as the built-in
    /// dark themes' are.
    pub fn from_wallpaper(roles: &HashMap<String, String>) -> Option<Self> {
        let role = |name: &str| roles.get(name).and_then(|hex| Rgb::parse(hex));
        let dark = role("surface")?.luminance() < 0.2;
        let mut p = Self::builtin(if dark { Builtin::Teal } else { Builtin::Mono }, dark);
        let primary = role("primary")?;
        p.accent = primary;
        p.on_accent = role("on_primary")?;
        p.accent_soft_border = mix(p.bg, primary, 0.3);
        p.accent_soft_bg = mix(p.bg, primary, 0.12);
        p.glow = mix(p.surface, p.accent_soft_border, if dark { 0.45 } else { 0.85 });
        p.readable().then_some(p)
    }

    fn readable(&self) -> bool {
        [self.bg, self.panel, self.surface].iter().all(|&bg| self.text.contrast(bg) >= 4.5)
            && self.on_accent.contrast(self.accent) >= 4.5
    }
}

/// The edge of a control (fields, buttons, an off switch): `variant` moved towards `outline` until
/// it stands out 3:1 from every background a control sits on (WCAG 1.4.11), or `outline` itself.
/// `outline_variant` alone was 1.5–1.75:1, and `outline` all the way is heavier than needed.
fn control_edge(variant: Rgb, outline: Rgb, backgrounds: &[Rgb]) -> Rgb {
    (0..=20)
        .map(|i| mix(variant, outline, f64::from(i) / 20.0))
        .find(|edge| backgrounds.iter().all(|&bg| edge.contrast(bg) >= 3.0))
        .unwrap_or(outline)
}

/// `a` moved towards `b` by `f` (0–1).
fn mix(a: Rgb, b: Rgb, f: f64) -> Rgb {
    let m = |x: u8, y: u8| (f64::from(x) * (1.0 - f) + f64::from(y) * f).round() as u8;
    Rgb(m(a.0, b.0), m(a.1, b.1), m(a.2, b.2))
}

/// What a colour source gave.
#[derive(Clone, Debug, PartialEq)]
pub enum SourceStatus {
    Missing,
    /// It exists but isn't a usable palette: not JSON, a role missing, or unreadable colours.
    Unusable,
    Used(Palette),
}

pub fn read_source(path: &Path) -> SourceStatus {
    let Ok(text) = std::fs::read_to_string(path) else { return SourceStatus::Missing };
    let Ok(roles) = serde_json::from_str::<HashMap<String, serde_json::Value>>(&text) else {
        return SourceStatus::Unusable;
    };
    let roles: HashMap<String, String> =
        roles.into_iter().filter_map(|(k, v)| v.as_str().map(|s| (k, s.to_owned()))).collect();
    Palette::from_wallpaper(&roles).map_or(SourceStatus::Unusable, SourceStatus::Used)
}

/// Each source's status, in order, and the palette of the first usable one; `None` (the
/// built-in theme) when there's none, or when following the wallpaper is off.
pub fn resolve(sources: &[ColourSource], follow: bool) -> (Vec<SourceStatus>, Option<Palette>) {
    let statuses: Vec<SourceStatus> = sources.iter().map(|s| read_source(&s.resolved())).collect();
    let palette = statuses.iter().find_map(|s| match s {
        SourceStatus::Used(p) => Some(*p),
        _ => None,
    });
    (statuses, palette.filter(|_| follow))
}

/// Why the built-in themes are greyed out: wallpaper colours are in use. Empty when they aren't.
pub fn builtin_note(follow: bool, statuses: &[SourceStatus]) -> String {
    if follow && statuses.iter().any(|s| matches!(s, SourceStatus::Used(_))) {
        "Wallpaper colours are in use. Turn off Follow wallpaper to pick one.".to_owned()
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_in_themes_are_greyed_out_while_wallpaper_colours_are_in_use() {
        let used = SourceStatus::Used(Palette::from_material(&blue()).unwrap());
        assert_eq!(builtin_note(true, &[SourceStatus::Missing, SourceStatus::Missing]), "");
        assert!(
            builtin_note(true, &[SourceStatus::Missing, used.clone()]).starts_with("Wallpaper colours are in use.")
        );
        assert_eq!(builtin_note(false, &[used]), "");
    }

    /// The owner's blue ML4W palette from the mockups.
    fn blue() -> HashMap<String, String> {
        [
            ("surface", "#101418"),
            ("surface_container_lowest", "#0b0e12"),
            ("surface_container_low", "#191c20"),
            ("surface_container", "#1d2024"),
            ("surface_container_high", "#272a2f"),
            ("surface_container_highest", "#32353a"),
            ("on_surface", "#e0e2e8"),
            ("on_surface_variant", "#c2c7cf"),
            ("outline", "#8c9199"),
            ("outline_variant", "#42474e"),
            ("primary", "#9ecafc"),
            ("on_primary", "#003256"),
            ("primary_container", "#164974"),
            ("secondary_container", "#3b4857"),
            ("error", "#ffb4ab"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect()
    }

    /// A wallpaper with strongly tinted surfaces gives only its accent; the rest is neutral.
    #[test]
    fn the_wallpaper_gives_only_the_accent() {
        let mut roles = blue();
        roles.insert("surface".into(), "#1a1030".into());
        roles.insert("surface_container".into(), "#2a1850".into());
        roles.insert("on_surface".into(), "#efe6ff".into());
        let p = Palette::from_wallpaper(&roles).unwrap();
        let teal = Palette::builtin(Builtin::Teal, true);
        assert_eq!((p.bg, p.surface, p.text, p.muted), (teal.bg, teal.surface, teal.text, teal.muted));
        assert_eq!((p.accent, p.on_accent), (Rgb::parse("#9ecafc").unwrap(), Rgb::parse("#003256").unwrap()));
        assert_ne!(p.accent_soft_bg, teal.accent_soft_bg);
        // A light wallpaper scheme: Mono light's greys.
        let mut light = roles.clone();
        light.insert("surface".into(), "#fdf7ff".into());
        light.insert("primary".into(), "#6b4fa0".into());
        light.insert("on_primary".into(), "#ffffff".into());
        let p = Palette::from_wallpaper(&light).unwrap();
        assert_eq!(p.bg, Palette::builtin(Builtin::Mono, false).bg);
    }

    #[test]
    fn hex_colours() {
        assert_eq!(Rgb::parse("#5cc8b0"), Some(Rgb(0x5c, 0xc8, 0xb0)));
        assert_eq!(Rgb::parse("5cc8b0"), None);
        assert_eq!(Rgb::parse("#5cc8b"), None);
        assert_eq!(Rgb::parse("#zzzzzz"), None);
    }

    #[test]
    fn contrast_ratio() {
        assert!((Rgb(0, 0, 0).contrast(Rgb(255, 255, 255)) - 21.0).abs() < 0.01);
        assert!((Rgb(10, 10, 10).contrast(Rgb(10, 10, 10)) - 1.0).abs() < 0.001);
    }

    #[test]
    fn material_roles_map_like_the_mockups() {
        let p = Palette::from_material(&blue()).expect("a usable palette");
        assert_eq!(p.bg, Rgb::parse("#101418").unwrap());
        assert_eq!(p.accent, Rgb::parse("#9ecafc").unwrap());
        assert_eq!(p.accent_soft_bg, Rgb::parse("#3b4857").unwrap());
        assert_eq!(p.danger, Rgb::parse("#ffb4ab").unwrap());
        // Warnings stay amber whatever the wallpaper.
        assert_eq!(p.warn_text, AMBER_DARK.2);
    }

    #[test]
    fn missing_roles_or_unreadable_text_fall_back() {
        let mut roles = blue();
        roles.remove("primary");
        assert_eq!(Palette::from_material(&roles), None);
        let mut roles = blue();
        roles.insert("on_surface".into(), "#202428".into());
        assert_eq!(Palette::from_material(&roles), None);
    }

    #[test]
    fn built_in_palettes_are_readable() {
        for builtin in Builtin::ALL {
            for dark in [true, false] {
                let p = Palette::builtin(builtin, dark);
                assert!(p.readable(), "{builtin:?} dark={dark}");
                assert_eq!(p.bg.luminance() < 0.2, dark, "{builtin:?} dark={dark}");
            }
        }
        // Every dark theme has Teal's neutral greys, and its own accent.
        let teal = Palette::builtin(Builtin::Teal, true);
        for builtin in Builtin::ALL {
            let p = Palette::builtin(builtin, true);
            assert_eq!(
                (p.bg, p.surface, p.text, p.muted),
                (teal.bg, teal.surface, teal.text, teal.muted),
                "{builtin:?}"
            );
            assert!(builtin == Builtin::Teal || p.accent != teal.accent, "{builtin:?}");
        }
    }

    #[test]
    fn control_edges_stand_out_3_to_1_in_every_built_in_theme() {
        for builtin in Builtin::ALL {
            for dark in [true, false] {
                let p = Palette::builtin(builtin, dark);
                for bg in [p.bg, p.panel, p.surface, p.input_bg] {
                    let ratio = p.ctrl_border.contrast(bg);
                    assert!(ratio >= 3.0, "{builtin:?} dark={dark}: {ratio:.2}");
                }
            }
        }
        // Not more than needed: it stays nearer the variant than the outline in the owner's.
        let p = Palette::from_material(&blue()).unwrap();
        assert!(p.ctrl_border.contrast(p.surface) < p.faint.contrast(p.surface));
    }

    #[test]
    fn first_usable_source_wins() {
        let dir = std::env::temp_dir().join(format!("lituus-theme-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let broken = dir.join("broken.json");
        std::fs::write(&broken, "{ not json").unwrap();
        let good = dir.join("colors.json");
        std::fs::write(&good, serde_json::to_string(&blue()).unwrap()).unwrap();
        let sources = [
            ColourSource { path: dir.join("missing.json").display().to_string() },
            ColourSource { path: broken.display().to_string() },
            ColourSource { path: good.display().to_string() },
        ];
        let (statuses, palette) = resolve(&sources, true);
        assert_eq!(resolve(&sources, false).1, None);
        assert_eq!(statuses[0], SourceStatus::Missing);
        assert_eq!(statuses[1], SourceStatus::Unusable);
        assert!(matches!(statuses[2], SourceStatus::Used(_)));
        assert_eq!(palette.map(|p| p.accent), Rgb::parse("#9ecafc"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
