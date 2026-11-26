//! Preferences, kept as `key=value` lines in `prefs.txt` in the OS config
//! folder (macOS: ~/Library/Application Support/com.github.ericdano.AE30PatchEditor,
//! Windows: %APPDATA%\ericdano\AE30PatchEditor\config).

use eframe::egui::Color32;

use super::commands::ThemeChoice;
use super::theme::{Look, MAX_CORNERS, SCALES};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KnobSpeed {
    Slow,
    Normal,
    Fast,
}

impl KnobSpeed {
    pub const ALL: [(KnobSpeed, &'static str); 3] = [
        (KnobSpeed::Slow, "Slow"),
        (KnobSpeed::Normal, "Normal"),
        (KnobSpeed::Fast, "Fast"),
    ];

    /// Pixels of mouse travel for a knob's whole range.
    pub fn pixels(self) -> f32 {
        match self {
            KnobSpeed::Slow => 300.0,
            KnobSpeed::Normal => 160.0,
            KnobSpeed::Fast => 90.0,
        }
    }

    fn key(self) -> &'static str {
        match self {
            KnobSpeed::Slow => "slow",
            KnobSpeed::Normal => "normal",
            KnobSpeed::Fast => "fast",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Prefs {
    pub theme: ThemeChoice,
    pub look: Look,
    pub knob_speed: KnobSpeed,
    /// Leave INIT SCENE slots out of the scene list.
    pub hide_empty: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            theme: ThemeChoice::System,
            look: Look::default(),
            knob_speed: KnobSpeed::Normal,
            hide_empty: false,
        }
    }
}

fn path() -> Option<std::path::PathBuf> {
    let dirs = directories::ProjectDirs::from("com.github", "ericdano", "AE30PatchEditor")?;
    Some(dirs.config_dir().join("prefs.txt"))
}

fn parse_hex(v: &str) -> Option<Color32> {
    let n = u32::from_str_radix(v.trim().trim_start_matches('#'), 16).ok()?;
    (v.trim().trim_start_matches('#').len() == 6)
        .then(|| Color32::from_rgb((n >> 16) as u8, (n >> 8) as u8, n as u8))
}

impl Prefs {
    pub fn parse(text: &str) -> Self {
        let mut p = Prefs::default();
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let v = v.trim();
            match k.trim() {
                "theme" => {
                    if let Some(&(t, _)) = ThemeChoice::ALL.iter().find(|(t, _)| t.key() == v) {
                        p.theme = t;
                    }
                }
                "accent" => p.look.accent = parse_hex(v),
                "corners" => {
                    if let Ok(n) = v.parse::<u8>() {
                        p.look.corners = n.min(MAX_CORNERS);
                    }
                }
                "scale" => {
                    if let Some(n) = v.parse::<u32>().ok().filter(|n| SCALES.contains(n)) {
                        p.look.scale = n as f32 / 100.0;
                    }
                }
                "slider_fill" => p.look.slider_fill = v != "0",
                "knob_speed" => {
                    if let Some(&(s, _)) = KnobSpeed::ALL.iter().find(|(s, _)| s.key() == v) {
                        p.knob_speed = s;
                    }
                }
                "hide_empty" => p.hide_empty = v == "1",
                _ => {}
            }
        }
        p
    }

    pub fn to_text(&self) -> String {
        let mut t = format!("theme={}\n", self.theme.key());
        if let Some(c) = self.look.accent {
            t += &format!("accent={:02x}{:02x}{:02x}\n", c.r(), c.g(), c.b());
        }
        t += &format!(
            "corners={}\nscale={}\nslider_fill={}\nknob_speed={}\nhide_empty={}\n",
            self.look.corners,
            (self.look.scale * 100.0).round() as u32,
            u8::from(self.look.slider_fill),
            self.knob_speed.key(),
            u8::from(self.hide_empty),
        );
        t
    }

    pub fn load() -> Self {
        path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|t| Prefs::parse(&t))
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let Some(p) = path() {
            let _ = std::fs::create_dir_all(p.parent().unwrap());
            let _ = std::fs::write(p, self.to_text());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefs_round_trip() {
        let p = Prefs {
            theme: ThemeChoice::Dark,
            look: Look {
                accent: Some(Color32::from_rgb(0x12, 0xA1, 0x8F)),
                corners: 3,
                scale: 1.25,
                slider_fill: false,
            },
            knob_speed: KnobSpeed::Slow,
            hide_empty: true,
        };
        assert_eq!(Prefs::parse(&p.to_text()), p);
        assert_eq!(
            Prefs::parse("junk\ncorners=99"),
            Prefs {
                look: Look {
                    corners: MAX_CORNERS,
                    ..Look::default()
                },
                ..Prefs::default()
            }
        );
    }
}
