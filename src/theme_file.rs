//! Custom themes: `themes/<name>.toml` (or `.json`) next to lt's
//! `config.toml`, in the shared Ligus theme format (`ligus.theme.v1`, the
//! same file the desktop imports). Colours only: every value is a colour in
//! a strict grammar, checked here.
//!
//! ```toml
//! name = "Heather"
//! extends = "pollen"      # the palette that supplies every token left out
//! [light]
//! ink = "#2b2118"
//! accentCool = "oklch(0.45 0.12 320)"
//! [dark]
//! accentCool = "#d7a6e8"
//! ```
//!
//! An error (a bad colour, a value that isn't a string, a file too big)
//! stops the theme loading and names the line; an unknown key, or a pair of
//! colours with too little contrast, is a warning and the theme still
//! loads. `lt themes check FILE` prints both.

use std::path::{Path, PathBuf};

use crate::theme_tokens::{PALETTES, SchemeTokens, TokenColor};

/// Files bigger than this are refused before they are parsed.
pub const MAX_FILE_BYTES: u64 = 64 * 1024;
/// The longest colour value accepted.
pub const MAX_VALUE_CHARS: usize = 64;
/// The longest display name.
pub const MAX_NAME_CHARS: usize = 40;
/// At most this many theme files are read from the themes directory.
pub const MAX_FILES: usize = 64;

/// Tokens lt draws with, and which of its colours each sets.
pub const LT_TOKENS: &[(&str, &str)] = &[
    ("canvas", "the background contrast is checked against (lt never paints it)"),
    ("ink", "text"),
    ("inkMuted", "secondary text: hints, timestamps"),
    ("accentCool", "the accent: headings, focus, the selected view"),
    ("accentWarm", "needs you and @mentions"),
    ("border", "pane borders"),
    ("selection", "the selected row's background"),
    ("agent", "agents' names"),
    ("ok", "done"),
    ("info", "working, running"),
    ("warn", "queued, warnings"),
    ("danger", "failed, errors"),
    ("neutral", "paused, no data"),
    ("diffAddInk", "added lines"),
    ("diffDelInk", "removed lines"),
    ("diffComment", "the review-comment marker"),
];

/// Tokens only the desktop draws with: known, so never a warning, and
/// ignored here.
pub const DESKTOP_TOKENS: &[&str] =
    &["panel", "mist", "line", "onAccent", "sidebarTop", "sidebarBottom", "sidebarInk", "sidebarInkMuted", "diffAddBg", "diffDelBg"];

/// Top-level keys of a theme file.
const TOP_KEYS: &[&str] = &["$schema", "format", "name", "extends", "light", "dark"];

/// Names a custom theme can't take: the palettes, mono and the old
/// `theme =` values.
pub const RESERVED: &[&str] = &["auto", "dark", "light", "mono"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Error,
    Warning,
}

/// One finding: an error or a warning, with the key and line it is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diag {
    pub level: Level,
    pub line: Option<usize>,
    pub key: String,
    pub message: String,
}

impl Diag {
    // Keys, names and quoted values come from the file: never let them
    // reach a terminal as escape sequences or bidi controls.
    fn error(line: Option<usize>, key: impl Into<String>, message: impl Into<String>) -> Diag {
        Diag { level: Level::Error, line, key: safe(&key.into()), message: safe(&message.into()) }
    }
    fn warn(line: Option<usize>, key: impl Into<String>, message: impl Into<String>) -> Diag {
        Diag { level: Level::Warning, line, key: safe(&key.into()), message: safe(&message.into()) }
    }

    /// `line 4: light.ink: …`, for stderr.
    pub fn text(&self) -> String {
        let at = self.line.map(|l| format!("line {l}: ")).unwrap_or_default();
        let key = if self.key.is_empty() { String::new() } else { format!("{}: ", self.key) };
        format!("{at}{key}{}", self.message)
    }
}

/// Text from a theme file (a key, a name, a file name) as it may reach a
/// terminal: control characters (ANSI, OSC, C1) and bidi controls become
/// U+FFFD.
pub fn safe(s: &str) -> String {
    s.chars().map(|c| if c.is_control() || crate::plain::is_bidi(c) { '\u{FFFD}' } else { c }).collect()
}

/// A path as it may reach a terminal.
pub fn safe_path(p: &Path) -> String {
    safe(&p.display().to_string())
}

/// A parsed colour: sRGB channels 0..255 (floats) and alpha 0..1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba {
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub a: f64,
}

impl Rgba {
    /// Rounded, clipped channels.
    pub fn rgb(self) -> (u8, u8, u8) {
        let c = |v: f64| v.round().clamp(0.0, 255.0) as u8;
        (c(self.r), c(self.g), c(self.b))
    }

    /// This colour painted over an opaque background.
    pub fn over(self, bg: (u8, u8, u8)) -> (u8, u8, u8) {
        let (r, g, b) = self.rgb();
        let mix = |f: u8, b: u8| (f64::from(f) * self.a + f64::from(b) * (1.0 - self.a)).round().clamp(0.0, 255.0) as u8;
        (mix(r, bg.0), mix(g, bg.1), mix(b, bg.2))
    }
}

/// A plain decimal: `12`, `0.5`, `.5` (no sign, no exponent).
fn number(s: &str) -> Option<f64> {
    let s = s.trim();
    let (int, frac) = match s.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (s, None),
    };
    let digits = |p: &str| !p.is_empty() && p.len() <= 12 && p.bytes().all(|b| b.is_ascii_digit());
    let ok = match frac {
        None => digits(int),
        Some(f) => (int.is_empty() || digits(int)) && digits(f),
    };
    if !ok {
        return None;
    }
    s.parse::<f64>().ok().filter(|v| v.is_finite())
}

fn channel(s: &str) -> Result<f64, String> {
    let s = s.trim();
    if s.is_empty() || s.len() > 3 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!("“{s}” isn't a channel (a whole number 0-255)"));
    }
    let v: f64 = s.parse().map_err(|_| format!("“{s}” isn't a channel"))?;
    if v > 255.0 { Err(format!("{s} is over 255")) } else { Ok(v) }
}

fn alpha(s: &str) -> Result<f64, String> {
    let v = number(s).ok_or_else(|| format!("“{}” isn't an alpha (0-1)", s.trim()))?;
    if v > 1.0 { Err(format!("alpha {} is over 1", s.trim())) } else { Ok(v) }
}

/// The text between `name(` and the closing `)`, case-insensitively, if
/// `s` is that function and nothing else.
fn function<'a>(s: &'a str, name: &str) -> Option<&'a str> {
    let open = s.find('(')?;
    if !s[..open].eq_ignore_ascii_case(name) || !s.ends_with(')') {
        return None;
    }
    let inner = &s[open + 1..s.len() - 1];
    // One pair of parentheses, nothing nested.
    if inner.contains(['(', ')']) {
        return None;
    }
    Some(inner)
}

/// Parse a colour: `#rgb`, `#rrggbb`, `#rrggbbaa`; `rgb(r, g, b)`,
/// `rgba(r, g, b, a)` or `rgb(r g b / a)`; `oklch(L C H)` or
/// `oklch(L C H / a)` with L 0-1 or 0%-100%, C 0-0.5, H 0-360.
pub fn parse_color(raw: &str) -> Result<Rgba, String> {
    if raw.chars().count() > MAX_VALUE_CHARS {
        return Err(format!("longer than {MAX_VALUE_CHARS} characters"));
    }
    let s = raw.trim();
    let bad = || format!("“{s}” isn't a colour: use #rrggbb, rgb(r, g, b) or oklch(L C H)");
    if s.chars().any(|c| c.is_control()) {
        return Err("has a control character".into());
    }
    if let Some(hex) = s.strip_prefix('#') {
        if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(bad());
        }
        let n = |i: usize, w: usize| u8::from_str_radix(&hex[i..i + w], 16).map(f64::from).map_err(|_| bad());
        return match hex.len() {
            3 => Ok(Rgba { r: n(0, 1)? * 17.0, g: n(1, 1)? * 17.0, b: n(2, 1)? * 17.0, a: 1.0 }),
            6 => Ok(Rgba { r: n(0, 2)?, g: n(2, 2)?, b: n(4, 2)?, a: 1.0 }),
            8 => Ok(Rgba { r: n(0, 2)?, g: n(2, 2)?, b: n(4, 2)?, a: n(6, 2)? / 255.0 }),
            _ => Err(bad()),
        };
    }
    if let Some(inner) = function(s, "rgb").or_else(|| function(s, "rgba")) {
        let (chans, a) = if inner.contains(',') {
            let parts: Vec<&str> = inner.split(',').collect();
            match parts.as_slice() {
                [r, g, b] => (vec![*r, *g, *b], None),
                [r, g, b, a] => (vec![*r, *g, *b], Some(*a)),
                _ => return Err(bad()),
            }
        } else {
            let (c, a) = match inner.split_once('/') {
                Some((c, a)) => (c, Some(a)),
                None => (inner, None),
            };
            let parts: Vec<&str> = c.split_whitespace().collect();
            if parts.len() != 3 {
                return Err(bad());
            }
            (parts, a)
        };
        let a = a.map(alpha).transpose()?.unwrap_or(1.0);
        return Ok(Rgba { r: channel(chans[0])?, g: channel(chans[1])?, b: channel(chans[2])?, a });
    }
    if let Some(inner) = function(s, "oklch") {
        let (c, a) = match inner.split_once('/') {
            Some((c, a)) => (c, Some(a)),
            None => (inner, None),
        };
        let parts: Vec<&str> = c.split_whitespace().collect();
        let [l, ch, h] = parts.as_slice() else { return Err(bad()) };
        let l = match l.strip_suffix('%') {
            Some(p) => number(p).map(|v| v / 100.0),
            None => number(l),
        }
        .filter(|v| *v <= 1.0)
        .ok_or_else(|| format!("oklch lightness “{l}” must be 0-1 or 0%-100%"))?;
        let ch = number(ch).filter(|v| *v <= 0.5).ok_or_else(|| format!("oklch chroma “{ch}” must be 0-0.5"))?;
        let h = number(h).filter(|v| *v <= 360.0).ok_or_else(|| format!("oklch hue “{h}” must be 0-360"))?;
        let a = a.map(alpha).transpose()?.unwrap_or(1.0);
        let (r, g, b) = oklch_to_rgb(l, ch, h);
        return Ok(Rgba { r, g, b, a });
    }
    Err(bad())
}

fn lin(v: f64) -> f64 {
    let c = v / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn delin(x: f64) -> f64 {
    255.0 * if x <= 0.0031308 { 12.92 * x } else { 1.055 * x.signum() * x.abs().powf(1.0 / 2.4) - 0.055 }
}

/// sRGB (0..255, unclipped) of an OKLCH colour (Björn Ottosson's OKLab).
pub fn oklch_to_rgb(l: f64, c: f64, h: f64) -> (f64, f64, f64) {
    let (a, b) = (c * h.to_radians().cos(), c * h.to_radians().sin());
    let l_ = (l + 0.3963377774 * a + 0.2158037573 * b).powi(3);
    let m_ = (l - 0.1055613458 * a - 0.0638541728 * b).powi(3);
    let s_ = (l - 0.0894841775 * a - 1.291485548 * b).powi(3);
    (
        delin(4.0767416621 * l_ - 3.3077115913 * m_ + 0.2309699292 * s_),
        delin(-1.2684380046 * l_ + 2.6097574011 * m_ - 0.3413193965 * s_),
        delin(-0.0041960863 * l_ - 0.7034186147 * m_ + 1.707612701 * s_),
    )
}

/// OKLab of an sRGB colour.
pub fn oklab(rgb: (u8, u8, u8)) -> (f64, f64, f64) {
    let (r, g, b) = (lin(f64::from(rgb.0)), lin(f64::from(rgb.1)), lin(f64::from(rgb.2)));
    let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
    let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
    let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
    (
        0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s,
        1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s,
        0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s,
    )
}

/// Distance in OKLab (about 0.02 is just noticeable).
pub fn delta_e(a: (u8, u8, u8), b: (u8, u8, u8)) -> f64 {
    let (x, y) = (oklab(a), oklab(b));
    ((x.0 - y.0).powi(2) + (x.1 - y.1).powi(2) + (x.2 - y.2).powi(2)).sqrt()
}

/// A colour as text on `bg`: itself when it reads at 4.6:1, else the
/// nearest OKLCH lightness that does (darker on a light background,
/// lighter on a dark one), its chroma cut only as far as sRGB needs: the
/// token generator's rule for the built-in palettes' accents.
pub fn ink_on(c: (u8, u8, u8), bg: (u8, u8, u8)) -> (u8, u8, u8) {
    const MIN: f64 = 4.6;
    if contrast(c, bg) >= MIN {
        return c;
    }
    let (l0, a, b) = oklab(c);
    let (c0, h) = (a.hypot(b), b.atan2(a).to_degrees().rem_euclid(360.0));
    let light_bg = oklab(bg).0 > 0.6;
    let step = if light_bg { -0.005 } else { 0.005 };
    let mut l = l0;
    while l > 0.0 && l < 1.0 {
        let mut ch = c0;
        let fits = |ch: f64| {
            let (r, g, b) = oklch_to_rgb(l, ch, h);
            [r, g, b].iter().all(|v| (-0.5..=255.5).contains(v))
        };
        while ch > 0.0 && !fits(ch) {
            ch -= 0.002;
        }
        let (r, g, b) = oklch_to_rgb(l, ch.max(0.0), h);
        let out = Rgba { r, g, b, a: 1.0 }.rgb();
        if contrast(out, bg) >= MIN {
            return out;
        }
        l += step;
    }
    if light_bg { (0, 0, 0) } else { (255, 255, 255) }
}

/// WCAG 2.x contrast ratio of two opaque colours.
pub fn contrast(a: (u8, u8, u8), b: (u8, u8, u8)) -> f64 {
    let lum = |c: (u8, u8, u8)| 0.2126 * lin(f64::from(c.0)) + 0.7152 * lin(f64::from(c.1)) + 0.0722 * lin(f64::from(c.2));
    let (x, y) = (lum(a), lum(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// The nearest xterm-256 index (16..255, never the user's own 0..15) by
/// OKLab distance: what the token generator picks for the built-in palettes.
pub fn nearest_xterm(rgb: (u8, u8, u8)) -> u8 {
    const CUBE: [u8; 6] = [0, 95, 135, 175, 215, 255];
    let mut best = (16u8, f64::INFINITY);
    for i in 16u16..256 {
        let c = if i < 232 {
            let n = (i - 16) as usize;
            (CUBE[n / 36], CUBE[(n / 6) % 6], CUBE[n % 6])
        } else {
            let v = (8 + 10 * (i - 232)) as u8;
            (v, v, v)
        };
        let d = delta_e(rgb, c);
        if d < best.1 {
            best = (i as u8, d);
        }
    }
    best.0
}

/// A built-in palette by name.
pub fn palette(name: &str) -> Option<&'static crate::theme_tokens::Palette> {
    PALETTES.iter().find(|p| p.name == name)
}

/// A custom theme, parsed and checked.
#[derive(Debug, Clone)]
pub struct ThemeFile {
    /// The file name without its extension: what `theme = "…"` names.
    pub id: String,
    /// What Settings and the palette show (`name`, else the id).
    pub label: String,
    pub extends: &'static str,
    pub light: SchemeTokens,
    pub dark: SchemeTokens,
    /// Which appearances the file sets (the others are the base palette's).
    pub sets_light: bool,
    pub sets_dark: bool,
}

/// What reading a theme file found: the theme (unless there was an error)
/// and every finding.
#[derive(Debug, Clone)]
pub struct Report {
    pub path: Option<PathBuf>,
    pub id: String,
    pub theme: Option<ThemeFile>,
    pub diags: Vec<Diag>,
}

impl Report {
    pub fn errors(&self) -> impl Iterator<Item = &Diag> {
        self.diags.iter().filter(|d| d.level == Level::Error)
    }
    pub fn warnings(&self) -> impl Iterator<Item = &Diag> {
        self.diags.iter().filter(|d| d.level == Level::Warning)
    }
    pub fn ok(&self) -> bool {
        self.theme.is_some()
    }
}

/// A theme's id from its file name: lowercase letters, digits, `-` and
/// `_`, 1-40 long, not a built-in name.
pub fn id_for(path: &Path) -> Result<String, String> {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let ok = !stem.is_empty()
        && stem.len() <= MAX_NAME_CHARS
        && stem.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
        && stem.bytes().next().is_some_and(|b| b.is_ascii_alphanumeric());
    if !ok {
        return Err(format!(
            "the file name “{stem}” can't name a theme: use lowercase letters, digits, - and _ (at most {MAX_NAME_CHARS})"
        ));
    }
    if RESERVED.contains(&stem) || palette(stem).is_some() {
        return Err(format!("“{stem}” is a built-in theme's name: rename the file"));
    }
    Ok(stem.to_string())
}

/// Read and check a theme file.
pub fn read(path: &Path) -> Report {
    let mut report = Report { path: Some(path.to_path_buf()), id: String::new(), theme: None, diags: vec![] };
    let id = match id_for(path) {
        Ok(id) => id,
        Err(e) => {
            report.diags.push(Diag::error(None, "", e));
            report.id = safe(&path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default());
            return report;
        }
    };
    report.id.clone_from(&id);
    let json = path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("json"));
    let size = match std::fs::metadata(path) {
        // A FIFO or a device (/dev/zero) would hang or never end.
        Ok(m) if !m.is_file() => {
            report.diags.push(Diag::error(None, "", "isn't a regular file"));
            return report;
        }
        Ok(m) => m.len(),
        Err(e) => {
            report.diags.push(Diag::error(None, "", format!("can't read it: {e}")));
            return report;
        }
    };
    if size > MAX_FILE_BYTES {
        report.diags.push(Diag::error(None, "", format!("{size} bytes: a theme file is at most {} KiB", MAX_FILE_BYTES / 1024)));
        return report;
    }
    let bytes = std::fs::File::open(path).and_then(|f| {
        use std::io::Read;
        let mut b = vec![];
        f.take(MAX_FILE_BYTES + 1).read_to_end(&mut b).map(|_| b)
    });
    let text = match bytes {
        Ok(b) if b.len() as u64 <= MAX_FILE_BYTES => match String::from_utf8(b) {
            Ok(t) => t,
            Err(_) => {
                report.diags.push(Diag::error(None, "", "isn't UTF-8 text"));
                return report;
            }
        },
        Ok(_) => {
            report.diags.push(Diag::error(None, "", format!("a theme file is at most {} KiB", MAX_FILE_BYTES / 1024)));
            return report;
        }
        Err(e) => {
            report.diags.push(Diag::error(None, "", format!("can't read it: {e}")));
            return report;
        }
    };
    let mut r = parse(&id, &text, json);
    r.path = report.path;
    r
}

/// A value as the file gave it: a string, or something else (an error).
enum Value {
    Str(String),
    Table(Vec<(String, Option<usize>, Value)>),
    Other(&'static str),
}

fn line_of(text: &str, offset: usize) -> usize {
    text[..offset.min(text.len())].bytes().filter(|b| *b == b'\n').count() + 1
}

fn toml_tree(text: &str) -> Result<Vec<(String, Option<usize>, Value)>, Diag> {
    use toml::de::{DeTable, DeValue};
    fn conv(text: &str, v: &DeValue<'_>) -> Value {
        match v {
            DeValue::String(s) => Value::Str(s.to_string()),
            DeValue::Table(t) => Value::Table(
                t.iter().map(|(k, v)| (k.get_ref().to_string(), Some(line_of(text, k.span().start)), conv(text, v.get_ref()))).collect(),
            ),
            DeValue::Integer(_) | DeValue::Float(_) => Value::Other("a number"),
            DeValue::Boolean(_) => Value::Other("true or false"),
            DeValue::Datetime(_) => Value::Other("a date"),
            DeValue::Array(_) => Value::Other("a list"),
        }
    }
    let table = DeTable::parse(text).map_err(|e| {
        let line = e.span().map(|s| line_of(text, s.start));
        let msg = e.message().lines().next().unwrap_or("isn't valid TOML").to_string();
        Diag::error(line, "", format!("isn't valid TOML: {msg}"))
    })?;
    Ok(table
        .get_ref()
        .iter()
        .map(|(k, v)| (k.get_ref().to_string(), Some(line_of(text, k.span().start)), conv(text, v.get_ref())))
        .collect())
}

fn json_tree(text: &str) -> Result<Vec<(String, Option<usize>, Value)>, Diag> {
    // serde_json keeps the last of duplicate keys; refuse them instead,
    // like TOML does, so the file means one thing.
    struct Seen(serde_json::Value);
    impl<'de> serde::Deserialize<'de> for Seen {
        fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            use serde::de::{Error, MapAccess, SeqAccess, Visitor};
            struct V;
            impl<'de> Visitor<'de> for V {
                type Value = serde_json::Value;
                fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                    f.write_str("JSON")
                }
                fn visit_bool<E>(self, v: bool) -> Result<Self::Value, E> {
                    Ok(v.into())
                }
                fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E> {
                    Ok(v.into())
                }
                fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E> {
                    Ok(v.into())
                }
                fn visit_f64<E>(self, v: f64) -> Result<Self::Value, E> {
                    Ok(serde_json::Value::from(v))
                }
                fn visit_str<E>(self, v: &str) -> Result<Self::Value, E> {
                    Ok(v.into())
                }
                fn visit_unit<E>(self) -> Result<Self::Value, E> {
                    Ok(serde_json::Value::Null)
                }
                fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<Self::Value, A::Error> {
                    let mut out = vec![];
                    while let Some(Seen(v)) = a.next_element()? {
                        out.push(v);
                    }
                    Ok(out.into())
                }
                fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> Result<Self::Value, A::Error> {
                    let mut out = serde_json::Map::new();
                    while let Some(k) = a.next_key::<String>()? {
                        let Seen(v) = a.next_value()?;
                        if out.insert(k.clone(), v).is_some() {
                            return Err(A::Error::custom(format!("duplicate key “{k}”")));
                        }
                    }
                    Ok(out.into())
                }
            }
            d.deserialize_any(V).map(Seen)
        }
    }
    let Seen(root) = serde_json::from_str::<Seen>(text).map_err(|e| Diag::error(Some(e.line()), "", format!("isn't valid JSON: {e}")))?;
    let serde_json::Value::Object(map) = root else {
        return Err(Diag::error(Some(1), "", "isn't a theme: the file must be one JSON object"));
    };
    // serde_json has no spans: find each key's line by searching for it
    // after its table's own key (good enough to point at it).
    fn find(text: &str, key: &str, from: usize) -> (Option<usize>, usize) {
        let Ok(needle) = serde_json::to_string(key) else { return (None, from) };
        match text.get(from..).and_then(|t| t.find(&needle)) {
            Some(i) => (Some(line_of(text, from + i)), from + i),
            None => (None, from),
        }
    }
    fn conv(text: &str, v: &serde_json::Value, from: usize) -> Value {
        match v {
            serde_json::Value::String(s) => Value::Str(s.clone()),
            serde_json::Value::Object(m) => Value::Table(
                m.iter()
                    .map(|(k, v)| {
                        let (line, at) = find(text, k, from);
                        (k.clone(), line, conv(text, v, at))
                    })
                    .collect(),
            ),
            serde_json::Value::Number(_) => Value::Other("a number"),
            serde_json::Value::Bool(_) => Value::Other("true or false"),
            serde_json::Value::Null => Value::Other("null"),
            serde_json::Value::Array(_) => Value::Other("a list"),
        }
    }
    Ok(map
        .iter()
        .map(|(k, v)| {
            let (line, at) = find(text, k, 0);
            (k.clone(), line, conv(text, v, at))
        })
        .collect())
}

/// Which of lt's colours a token sets.
fn slot<'a>(k: &'a mut SchemeTokens, token: &str) -> Option<&'a mut TokenColor> {
    Some(match token {
        "canvas" => &mut k.canvas,
        "ink" => &mut k.text,
        "inkMuted" => &mut k.dim,
        "accentCool" => &mut k.accent,
        "accentWarm" => &mut k.mention,
        "border" => &mut k.border,
        "selection" => &mut k.selection,
        "agent" => &mut k.agent,
        "ok" => &mut k.ok,
        "info" => &mut k.info,
        "warn" => &mut k.warn,
        "danger" => &mut k.danger,
        "neutral" => &mut k.neutral,
        "diffAddInk" => &mut k.diff_add,
        "diffDelInk" => &mut k.diff_del,
        "diffComment" => &mut k.diff_comment,
        _ => return None,
    })
}

fn set(c: &mut TokenColor, rgb: (u8, u8, u8)) {
    // The 16-colour slot stays the role's (ok stays the terminal's green).
    *c = TokenColor { rgb, xterm256: nearest_xterm(rgb), ansi16: c.ansi16 };
}

fn mix(fg: (u8, u8, u8), bg: (u8, u8, u8), a: f64) -> (u8, u8, u8) {
    Rgba { r: f64::from(fg.0), g: f64::from(fg.1), b: f64::from(fg.2), a }.over(bg)
}

/// One scheme from a `[light]` or `[dark]` table over the base palette's.
fn scheme(
    base: &SchemeTokens,
    section: &str,
    entries: &[(String, Option<usize>, Value)],
    diags: &mut Vec<Diag>,
) -> (SchemeTokens, Vec<(&'static str, Option<usize>)>) {
    let mut k = *base;
    let mut given: Vec<(String, Option<usize>, Rgba)> = vec![];
    for (key, line, v) in entries {
        let name = format!("{section}.{key}");
        let known = LT_TOKENS.iter().any(|(t, _)| t == key) || DESKTOP_TOKENS.contains(&key.as_str());
        if !known {
            diags.push(Diag::warn(*line, name, "isn't a theme token; ignored (lt help themes lists them)"));
            continue;
        }
        let Value::Str(s) = v else {
            let what = match v {
                Value::Other(w) => *w,
                _ => "a table",
            };
            diags.push(Diag::error(*line, name, format!("is {what}: a colour is a string, like \"#2b2118\"")));
            continue;
        };
        match parse_color(s) {
            Ok(c) => given.push((key.clone(), *line, c)),
            Err(e) => diags.push(Diag::error(*line, name, e)),
        }
    }
    // The canvas first: translucent colours are painted over it.
    let mut set_keys: Vec<(&'static str, Option<usize>)> = vec![];
    let static_key = |key: &str| LT_TOKENS.iter().find(|(t, _)| *t == key).map(|(t, _)| *t);
    if let Some((_, line, c)) = given.iter().find(|(key, ..)| key == "canvas") {
        let rgb = c.over(base.canvas.rgb);
        set(&mut k.canvas, rgb);
        set_keys.push(("canvas", *line));
    }
    let bg = k.canvas.rgb;
    for (key, line, c) in &given {
        if key == "canvas" {
            continue;
        }
        let Some(t) = static_key(key) else { continue };
        let mut rgb = c.over(bg);
        // Accents and status colours are fills or glyphs on the desktop;
        // lt draws them as text, so it darkens (or lightens) one that
        // wouldn't read, as it does for its own palettes, and says so.
        if INKS.contains(&t) {
            let ink = ink_on(rgb, bg);
            if ink != rgb {
                diags.push(Diag::warn(
                    *line,
                    format!("{section}.{key}"),
                    format!(
                        "{:.1}:1 on the canvas, too faint for text: lt shows it as #{:02x}{:02x}{:02x}",
                        contrast(rgb, bg),
                        ink.0,
                        ink.1,
                        ink.2
                    ),
                ));
                rgb = ink;
            }
        }
        if let Some(dst) = slot(&mut k, key) {
            set(dst, rgb);
            set_keys.push((t, *line));
        }
    }
    let has = |t: &str| set_keys.iter().any(|(s, _)| *s == t);
    // Derived colours follow the ink, the accent and the canvas when the
    // file sets those but not them (the generator's rules).
    if (has("ink") || has("canvas")) && !has("inkMuted") {
        set(&mut k.dim, mix(k.text.rgb, bg, 0.72));
    }
    if (has("ink") || has("canvas")) && !has("border") {
        let mut a = 0.2;
        let mut c = mix(k.text.rgb, bg, 1.0);
        while a <= 1.0 {
            let m = mix(k.text.rgb, bg, a);
            if contrast(m, bg) >= 3.1 {
                c = m;
                break;
            }
            a += 0.01;
        }
        set(&mut k.border, c);
    }
    if (has("accentCool") || has("canvas")) && !has("selection") {
        // A tint of the file's own accent (as given, before any darkening).
        let accent = given.iter().find(|(key, ..)| key == "accentCool").map(|(_, _, c)| c.over(bg)).unwrap_or(base.accent.rgb);
        let mut a = 0.3;
        let mut c = mix(accent, bg, a);
        while a <= 0.6 && delta_e(c, bg) < 0.06 {
            a += 0.02;
            c = mix(accent, bg, a);
        }
        set(&mut k.selection, c);
    }
    (k, set_keys)
}

/// Tokens lt draws as text that a theme file may give as a fill colour.
const INKS: &[&str] = &["accentCool", "accentWarm", "agent", "ok", "info", "warn", "danger", "neutral"];

/// Pairs lt draws: (token, what it is drawn on, minimum contrast).
const PAIRS: &[(&str, f64)] = &[
    ("ink", 4.5),
    ("inkMuted", 4.5),
    ("accentCool", 4.5),
    ("accentWarm", 4.5),
    ("agent", 4.5),
    ("ok", 4.5),
    ("info", 4.5),
    ("warn", 4.5),
    ("danger", 4.5),
    ("neutral", 4.5),
    ("diffAddInk", 4.5),
    ("diffDelInk", 4.5),
    ("border", 3.0),
    ("diffComment", 3.0),
];

fn color_of(k: &SchemeTokens, token: &str) -> (u8, u8, u8) {
    let mut k = *k;
    slot(&mut k, token).map(|c| c.rgb).unwrap_or(k.canvas.rgb)
}

/// Warnings for pairs the file touches that read badly on its canvas, and
/// for text on the selection.
pub fn contrast_warnings(section: &str, k: &SchemeTokens, set_keys: &[(&'static str, Option<usize>)]) -> Vec<Diag> {
    let line = |t: &str| set_keys.iter().find(|(s, _)| *s == t).and_then(|(_, l)| *l);
    let touched = |t: &str| set_keys.iter().any(|(s, _)| *s == t || *s == "canvas");
    let mut out = vec![];
    for (token, min) in PAIRS {
        if !touched(token) {
            continue;
        }
        let c = contrast(color_of(k, token), k.canvas.rgb);
        if c < *min {
            let what = if *min >= 4.5 { "text wants 4.5:1" } else { "a border or glyph wants 3:1" };
            out.push(Diag::warn(line(token).or(line("canvas")), format!("{section}.{token}"), format!("{c:.1}:1 on the canvas; {what}")));
        }
    }
    if touched("selection") || touched("ink") {
        let c = contrast(k.text.rgb, k.selection.rgb);
        if c < 4.5 {
            out.push(Diag::warn(
                line("selection").or(line("ink")),
                format!("{section}.selection"),
                format!("ink on the selection is {c:.1}:1; text wants 4.5:1"),
            ));
        }
    }
    out
}

/// Parse and check a theme file's text (`json` for a .json file).
pub fn parse(id: &str, text: &str, json: bool) -> Report {
    let mut diags = vec![];
    let mut report = Report { path: None, id: id.to_string(), theme: None, diags: vec![] };
    if text.len() as u64 > MAX_FILE_BYTES {
        report.diags.push(Diag::error(None, "", format!("a theme file is at most {} KiB", MAX_FILE_BYTES / 1024)));
        return report;
    }
    let tree = match if json { json_tree(text) } else { toml_tree(text) } {
        Ok(t) => t,
        Err(d) => {
            report.diags.push(d);
            return report;
        }
    };
    let get = |k: &str| tree.iter().find(|(key, ..)| key == k);
    let mut label = id.to_string();
    let mut extends: &'static str = PALETTES[0].name;
    for (key, line, v) in &tree {
        if !TOP_KEYS.contains(&key.as_str()) {
            diags.push(Diag::warn(*line, key.clone(), "isn't a theme file key; ignored"));
            continue;
        }
        let string = |v: &Value| if let Value::Str(s) = v { Some(s.clone()) } else { None };
        match key.as_str() {
            "format" => {
                if string(v).as_deref() != Some("ligus.theme.v1") {
                    diags.push(Diag::error(*line, "format", "must be \"ligus.theme.v1\" (or left out)"));
                }
            }
            "name" => match string(v) {
                Some(n)
                    if !n.trim().is_empty()
                        && n.chars().count() <= MAX_NAME_CHARS
                        && !n.chars().any(|c| c.is_control() || crate::plain::is_bidi(c)) =>
                {
                    label = n.trim().to_string();
                }
                _ => diags.push(Diag::error(
                    *line,
                    "name",
                    format!("must be text, 1-{MAX_NAME_CHARS} characters, no control or bidi characters"),
                )),
            },
            "extends" => match string(v).as_deref().and_then(palette) {
                Some(p) => extends = p.name,
                None => {
                    let names: Vec<&str> = PALETTES.iter().map(|p| p.name).collect();
                    diags.push(Diag::error(*line, "extends", format!("must be a built-in palette: {}", names.join(", "))));
                }
            },
            "$schema" if string(v).is_none() => diags.push(Diag::error(*line, "$schema", "must be a string")),
            "light" | "dark" if !matches!(v, Value::Table(_)) => {
                diags.push(Diag::error(*line, key.clone(), "must be a table of colours ([light] in TOML, an object in JSON)"))
            }
            _ => {}
        }
    }
    let base = palette(extends).unwrap_or(&PALETTES[0]);
    let mut light = base.light;
    let mut dark = base.dark;
    let (mut sets_light, mut sets_dark) = (false, false);
    for (section, out, flag, base_k) in
        [("light", &mut light, &mut sets_light, &base.light), ("dark", &mut dark, &mut sets_dark, &base.dark)]
    {
        if let Some((_, _, Value::Table(entries))) = get(section) {
            let (k, set_keys) = scheme(base_k, section, entries, &mut diags);
            diags.extend(contrast_warnings(section, &k, &set_keys));
            *out = k;
            *flag = true;
        }
    }
    if get("light").is_none() && get("dark").is_none() {
        diags.push(Diag::error(None, "", "sets no colours: add a [light] or [dark] table"));
    }
    if !diags.iter().any(|d| d.level == Level::Error) {
        report.theme = Some(ThemeFile { id: id.to_string(), label, extends, light, dark, sets_light, sets_dark });
    }
    report.diags = diags;
    report
}

/// Where custom themes live: `themes/` next to the config file.
pub fn themes_dir(config_file: Option<&Path>) -> Option<PathBuf> {
    config_file.and_then(Path::parent).map(|d| d.join("themes"))
}

/// Every theme file in `dir` (`.toml` and `.json`, sorted, at most
/// [`MAX_FILES`]), each read and checked. A missing directory is none.
pub fn read_dir(dir: &Path) -> Vec<Report> {
    let Ok(entries) = std::fs::read_dir(dir) else { return vec![] };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("toml") || e.eq_ignore_ascii_case("json"))
        })
        .filter(|p| p.is_file())
        .collect();
    // By name, a .toml before a .json of the same name.
    paths.sort_by_key(|p| (p.file_stem().map(|s| s.to_os_string()), p.extension().is_some_and(|e| e.eq_ignore_ascii_case("json"))));
    let mut out: Vec<Report> = vec![];
    for (i, p) in paths.iter().enumerate() {
        if i >= MAX_FILES {
            let mut r = Report { path: Some(p.clone()), id: String::new(), theme: None, diags: vec![] };
            r.diags.push(Diag::error(None, "", format!("more than {MAX_FILES} theme files: this one isn't read")));
            out.push(r);
            continue;
        }
        let mut r = read(p);
        if out.iter().any(|o| o.id == r.id && o.theme.is_some()) && r.theme.is_some() {
            r.theme = None;
            r.diags.push(Diag::error(None, "", format!("another file is also called “{}”; this one isn't used", r.id)));
        }
        out.push(r);
    }
    out
}

/// One finding as a `ligus.checks.v1` check.
fn check_json(d: &Diag) -> serde_json::Value {
    serde_json::json!({
        "id": if d.key.is_empty() { "theme.file".to_string() } else { format!("theme.{}", d.key) },
        "title": d.key,
        "status": if d.level == Level::Error { "fail" } else { "warn" },
        "detail": d.message,
        "line": d.line,
    })
}

fn report_json(r: &Report) -> serde_json::Value {
    let t = r.theme.as_ref();
    serde_json::json!({
        "id": r.id,
        "name": t.map(|t| t.label.clone()),
        "extends": t.map(|t| t.extends),
        "path": r.path.as_ref().map(|p| p.display().to_string()),
        "ok": r.ok(),
        "appearances": t.map(|t| {
            let mut a = vec![];
            if t.sets_light { a.push("light") }
            if t.sets_dark { a.push("dark") }
            a
        }),
        "checks": r.diags.iter().map(check_json).collect::<Vec<_>>(),
    })
}

/// `lt themes list [--json]` and `lt themes check FILE [--json]`: the
/// output and the exit code (2 is a usage error, printed by the caller).
pub fn command(args: &[String], json: bool, config: Option<&Path>) -> (String, i32) {
    let args: Vec<&str> = args.iter().map(String::as_str).filter(|a| *a != "--json").collect();
    let dir = themes_dir(config);
    match args.as_slice() {
        ["list"] | [] => {
            let reports = dir.as_deref().map(read_dir).unwrap_or_default();
            if json {
                let builtin: Vec<serde_json::Value> = PALETTES
                    .iter()
                    .map(|p| serde_json::json!({"id": p.name, "kind": "palette", "appearances": ["light", "dark"]}))
                    .chain([serde_json::json!({"id": "mono", "kind": "mono", "appearances": []})])
                    .collect();
                let custom: Vec<serde_json::Value> = reports
                    .iter()
                    .map(|r| {
                        let mut v = report_json(r);
                        v["kind"] = "custom".into();
                        v
                    })
                    .collect();
                let out = serde_json::json!({
                    "schema": "lt.themes.v1",
                    "dir": dir.as_ref().map(|d| d.display().to_string()),
                    "themes": builtin.into_iter().chain(custom).collect::<Vec<_>>(),
                });
                return (format!("{out}\n"), 0);
            }
            let mut out = String::from("Palettes (each for light and dark):\n");
            for p in PALETTES {
                out.push_str(&format!("  {}\n", p.name));
            }
            out.push_str("  mono  no colour (what NO_COLOR gives when no theme is set)\n");
            match &dir {
                Some(d) => out.push_str(&format!("\nTheme files in {}:\n", safe_path(d))),
                None => out.push_str("\nTheme files: no config directory\n"),
            }
            if reports.is_empty() {
                out.push_str("  none yet: lt help themes has the format\n");
            }
            for r in &reports {
                let file = safe(&r.path.as_ref().and_then(|p| p.file_name()).map(|f| f.to_string_lossy().to_string()).unwrap_or_default());
                let errors = r.errors().count();
                let warnings = r.warnings().count();
                let state = match (&r.theme, errors, warnings) {
                    (Some(t), _, 0) => format!("{} (extends {})", t.label, t.extends),
                    (Some(t), _, w) => format!("{} (extends {}), {w} warning{}", t.label, t.extends, if w == 1 { "" } else { "s" }),
                    (None, e, _) => format!("not loaded: {e} error{} (lt themes check {file})", if e == 1 { "" } else { "s" }),
                };
                out.push_str(&format!("  {:<16} {state}\n", r.id));
            }
            (out, 0)
        }
        ["check", file] => {
            let mut path = PathBuf::from(file);
            // A bare name is a file in the themes directory.
            if !path.exists()
                && path.components().count() == 1
                && let Some(d) = &dir
            {
                for ext in ["toml", "json"] {
                    let p = d.join(format!("{file}.{ext}"));
                    if p.exists() {
                        path = p;
                        break;
                    }
                }
            }
            if !path.exists() {
                return (format!("themes check: no file {}\n", safe(file)), 5);
            }
            let r = read(&path);
            let code = if r.ok() { 0 } else { 1 };
            if json {
                let out = serde_json::json!({
                    "schema": "ligus.checks.v1",
                    "tool": "lt",
                    "version": env!("CARGO_PKG_VERSION"),
                    "command": "themes check",
                    "ok": r.ok(),
                    "exitCode": code,
                    "theme": report_json(&r),
                    "checks": r.diags.iter().map(check_json).collect::<Vec<_>>(),
                    "next": if r.ok() { vec![format!("theme = \"{}\" in config.toml", r.id)] } else { vec![] },
                });
                return (format!("{out}\n"), code);
            }
            let mut out = format!("{}\n", safe_path(&path));
            for d in &r.diags {
                let word = if d.level == Level::Error { "error" } else { "warn " };
                out.push_str(&format!("  {word} {}\n", d.text()));
            }
            match &r.theme {
                Some(t) => out.push_str(&format!("ok: {} loads (theme = \"{}\" in config.toml)\n", t.label, r.id)),
                None => out.push_str("not loaded: fix the errors above\n"),
            }
            (out, code)
        }
        _ => ("themes: lt themes list, or lt themes check FILE (each takes --json)".into(), 2),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_parse_strictly() {
        let ok = |s: &str| parse_color(s).unwrap().rgb();
        assert_eq!(ok("#fff"), (255, 255, 255));
        assert_eq!(ok("#2B2118"), (0x2b, 0x21, 0x18));
        assert_eq!(parse_color("#00000080").unwrap().a, 128.0 / 255.0);
        assert_eq!(ok("rgb(1, 2, 3)"), (1, 2, 3));
        assert_eq!(ok("RGB(1 2 3)"), (1, 2, 3));
        assert!((parse_color("rgba(1, 2, 3, .5)").unwrap().a - 0.5).abs() < 1e-9);
        assert!((parse_color("rgb(1 2 3 / 0.25)").unwrap().a - 0.25).abs() < 1e-9);
        // OKLCH: the brand's own text colour on light round-trips.
        assert_eq!(ok("oklch(0.2376 0.0267 76.06)"), (38, 29, 16));
        assert_eq!(ok("oklch(23.76% 0.0267 76.06)"), (38, 29, 16));
        assert_eq!(ok("  #ffffff  "), (255, 255, 255));
        for bad in [
            "red",
            "#ff",
            "#fffff",
            "#ggg",
            "rgb(256, 0, 0)",
            "rgb(-1, 0, 0)",
            "rgb(1.5, 0, 0)",
            "rgb(1, 2)",
            "rgba(1, 2, 3, 2)",
            "oklch(1.2 0.1 10)",
            "oklch(0.5 0.9 10)",
            "oklch(0.5 0.1 400)",
            "oklch(0.5 0.1)",
            "oklch(0.5 0.1 1e2)",
            "red; } body { background: url(x) }",
            "#fff;",
            "url(javascript:alert(1))",
            "var(--x)",
            "calc(1px)",
            "rgb(calc(1), 2, 3)",
            "expression(alert(1))",
            "#fff\n}",
            "",
        ] {
            assert!(parse_color(bad).is_err(), "{bad:?} parsed");
        }
        assert!(parse_color(&format!("#{}", "f".repeat(70))).unwrap_err().contains("longer than"));
    }

    #[test]
    fn xterm_matches_the_generator() {
        // The generated table's own indexes, for its own colours.
        for p in PALETTES {
            for k in [p.light, p.dark] {
                for c in [k.text, k.accent, k.warn, k.border, k.selection] {
                    assert_eq!(nearest_xterm(c.rgb), c.xterm256, "{} {:?}", p.name, c.rgb);
                }
            }
        }
    }

    const HEATHER: &str = r##"
name = "Heather"
extends = "slate"
[light]
ink = "#2b2118"
accentCool = "oklch(0.45 0.12 320)"
[dark]
accentCool = "#d7a6e8"
"##;

    #[test]
    fn a_theme_overrides_its_base() {
        let r = parse("heather", HEATHER, false);
        assert!(r.errors().next().is_none(), "{:?}", r.diags);
        let t = r.theme.unwrap();
        assert_eq!(t.label, "Heather");
        assert_eq!(t.extends, "slate");
        let slate = palette("slate").unwrap();
        assert_eq!(t.light.text.rgb, (0x2b, 0x21, 0x18));
        assert_eq!(t.light.text.xterm256, nearest_xterm((0x2b, 0x21, 0x18)));
        assert_eq!(t.light.text.ansi16, slate.light.text.ansi16);
        // Left out: the base's.
        assert_eq!(t.light.ok, slate.light.ok);
        assert_eq!(t.dark.text, slate.dark.text);
        assert_eq!(t.dark.accent.rgb, (0xd7, 0xa6, 0xe8));
        // Derived from the new ink: the muted text, the border.
        assert_ne!(t.light.dim, slate.light.dim);
        assert!(contrast(t.light.border.rgb, t.light.canvas.rgb) >= 3.0);
    }

    #[test]
    fn json_and_toml_are_the_same_theme() {
        let json = r##"{
  "$schema": "https://docs.ligus.dev/schemas/ligus-theme.v1.json",
  "name": "Heather",
  "extends": "slate",
  "light": { "ink": "#2b2118", "accentCool": "oklch(0.45 0.12 320)" },
  "dark": { "accentCool": "#d7a6e8" }
}"##;
        let a = parse("heather", HEATHER, false).theme.unwrap();
        let b = parse("heather", json, true).theme.unwrap();
        assert_eq!((a.light, a.dark, a.label, a.extends), (b.light, b.dark, b.label, b.extends));
    }

    #[test]
    fn errors_name_the_line() {
        let r = parse("x", "name = \"X\"\n[light]\nink = \"#2b2118\"\naccentCool = \"blue\"\nok = 3\n", false);
        assert!(r.theme.is_none());
        let errs: Vec<&Diag> = r.errors().collect();
        assert_eq!(errs.len(), 2, "{errs:?}");
        assert_eq!((errs[0].line, errs[0].key.as_str()), (Some(4), "light.accentCool"));
        assert_eq!((errs[1].line, errs[1].key.as_str()), (Some(5), "light.ok"));
        assert!(errs[1].message.contains("a number"));
        let j = parse("x", "{\n  \"light\": {\n    \"ink\": \"nope\"\n  }\n}", true);
        let e = j.errors().next().unwrap();
        assert_eq!((e.line, e.key.as_str()), (Some(3), "light.ink"));
        // Not TOML at all, and duplicate keys in either format.
        assert!(parse("x", "[light\n", false).errors().next().unwrap().message.contains("TOML"));
        assert!(parse("x", "[light]\nink = \"#000\"\nink = \"#111\"\n", false).theme.is_none());
        assert!(parse("x", r##"{"light": {"ink": "#000", "ink": "#111"}}"##, true).theme.is_none());
        // Prototype-looking keys are just unknown keys.
        let p = parse("x", r##"{"__proto__": {"ink": "#000"}, "light": {"constructor": "#000", "ink": "#000"}}"##, true);
        assert!(p.theme.is_some());
        assert_eq!(p.warnings().filter(|w| w.message.contains("ignored")).count(), 2);
        // Nested tables and arrays are errors, not silently dropped.
        assert!(parse("x", "[light.ink]\nr = \"1\"\n", false).theme.is_none());
        assert!(parse("x", "[light]\nink = [\"#000\"]\n", false).theme.is_none());
        assert!(parse("x", "light = \"#000\"\n", false).theme.is_none());
        assert!(parse("x", "name = \"X\"\n", false).errors().next().unwrap().message.contains("no colours"));
        assert!(parse("x", "extends = \"midnight\"\n[dark]\n", false).theme.is_none());
        assert!(parse("x", &format!("name = \"{}\"\n[dark]\n", "n".repeat(41)), false).theme.is_none());
        assert!(parse("x", &" ".repeat(70 * 1024), false).errors().next().unwrap().message.contains("KiB"));
    }

    #[test]
    fn unknown_keys_and_low_contrast_warn() {
        let r = parse("x", "flavour = \"sweet\"\n[light]\nink = \"#ffffff\"\nsidebarTop = \"#000\"\nsparkle = \"#fff\"\n", false);
        assert!(r.theme.is_some(), "warnings never refuse a theme: {:?}", r.diags);
        let w: Vec<String> = r.warnings().map(|d| d.text()).collect();
        assert!(w.iter().any(|t| t.starts_with("line 1: flavour")), "{w:?}");
        assert!(w.iter().any(|t| t.starts_with("line 5: light.sparkle")), "{w:?}");
        assert!(!w.iter().any(|t| t.contains("sidebarTop")), "a desktop token is known: {w:?}");
        assert!(w.iter().any(|t| t.starts_with("line 3: light.ink") && t.contains("4.5:1")), "white ink on a light canvas: {w:?}");
    }

    /// A file exported from the desktop gives the accents as fill colours;
    /// lt shows them as a readable shade of the same hue, with a warning.
    #[test]
    fn faint_accents_become_inks() {
        let r = parse("x", "[light]\ncanvas = \"#ffffff\"\naccentCool = \"#f0c25a\"\nok = \"#2b9a54\"\n", false);
        let t = r.theme.as_ref().unwrap();
        assert!(contrast(t.light.accent.rgb, (255, 255, 255)) >= 4.5);
        assert!(contrast(t.light.ok.rgb, (255, 255, 255)) >= 4.5);
        let w: Vec<String> = r.warnings().map(|d| d.text()).collect();
        assert!(w.iter().any(|t| t.starts_with("line 3: light.accentCool") && t.contains("lt shows it as #")), "{w:?}");
        // Its hue stays: a honey, not a grey.
        let (_, a, b) = oklab(t.light.accent.rgb);
        assert!(a.hypot(b) > 0.05);
        // A readable one is left alone.
        let r = parse("x", "[light]\ncanvas = \"#ffffff\"\naccentCool = \"#7a4b00\"\n", false);
        assert_eq!(r.theme.unwrap().light.accent.rgb, (0x7a, 0x4b, 0x00));
        assert_eq!(r.diags, vec![]);
    }

    /// Text from the file never reaches a terminal as an escape sequence
    /// or a bidi control: keys, names, quoted values, file names.
    #[test]
    fn file_text_is_made_safe() {
        let r =
            parse("x", "\"\\u001b]0;TITLE\\u0007\\u001b[2J\" = \"x\"\n[light]\n\"\\u001b[41m\\u009b\" = \"#fff\"\nink = \"#000\"\n", false);
        assert!(r.theme.is_some());
        for d in &r.diags {
            assert!(!d.text().chars().any(|c| c.is_control()), "{:?}", d.text());
        }
        assert!(r.diags.iter().any(|d| d.key.contains('\u{FFFD}')), "{:?}", r.diags);
        let j = parse("x", "{\"light\": {\"\\u001b]8;;http://evil\\u0007\": \"#000\"}}", true);
        assert!(j.diags.iter().all(|d| !d.text().contains('\u{1b}')));
        assert!(parse("x", "name = \"\u{202e}gnp.exe\"\n[dark]\n", false).theme.is_none(), "a bidi control in the name");
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("\u{1b}[31mred.toml");
        std::fs::write(&p, "[dark]\n").unwrap();
        let r = read(&p);
        assert!(!r.id.contains('\u{1b}') && !r.diags[0].text().contains('\u{1b}'), "{r:?}");
        assert!(!safe_path(&p).contains('\u{1b}'));
    }

    /// A FIFO or a device in place of a file is refused, never read.
    #[cfg(unix)]
    #[test]
    fn special_files_are_refused() {
        let r = read(Path::new("/dev/zero"));
        assert!(r.errors().next().unwrap().message.contains("regular file"), "{r:?}");
        let dir = tempfile::tempdir().unwrap();
        let fifo = dir.path().join("fifo.toml");
        let c = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
        // SAFETY: a valid NUL-terminated path.
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        assert!(read(&fifo).errors().next().unwrap().message.contains("regular file"));
    }

    #[test]
    fn file_names_are_checked() {
        assert_eq!(id_for(Path::new("/t/heather.toml")).unwrap(), "heather");
        assert_eq!(id_for(Path::new("/t/my-theme_2.json")).unwrap(), "my-theme_2");
        for bad in ["/t/Heather.toml", "/t/pollen.toml", "/t/mono.toml", "/t/a b.toml", "/t/-x.toml", "/t/.toml"] {
            assert!(id_for(Path::new(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn reads_a_directory() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("heather.toml"), HEATHER).unwrap();
        std::fs::write(dir.path().join("broken.toml"), "[light]\nink = \"nope\"\n").unwrap();
        std::fs::write(dir.path().join("heather.json"), "{\"dark\": {}}").unwrap();
        std::fs::write(dir.path().join("big.toml"), " ".repeat(65 * 1024)).unwrap();
        std::fs::write(dir.path().join("notes.txt"), "not a theme").unwrap();
        let rs = read_dir(dir.path());
        let ids: Vec<(&str, bool)> = rs.iter().map(|r| (r.id.as_str(), r.ok())).collect();
        assert_eq!(ids, vec![("big", false), ("broken", false), ("heather", true), ("heather", false)]);
        assert!(read_dir(&dir.path().join("missing")).is_empty());
    }
}
