//! The Ligus mark in half-block characters, for the first-run and empty
//! screens. The geometry and colours are the brand kit's mark
//! (`ligus-mark.svg`): seven pointy-top hexagons, the core, right and
//! up-left cells in the warm gradient, the rest in the two creams.
//!
//! Each terminal cell holds two square-ish pixels (`▀` with a foreground
//! and a background). In the mono theme the mark is a silhouette with no
//! colour at all. Callers hide it when it doesn't fit; plain mode never
//! draws the UI, so it never shows there.

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};

use crate::theme::Theme;

/// Rows of the mark (each two pixels tall).
pub const HEIGHT: u16 = 12;
/// Columns of the mark.
pub const WIDTH: u16 = 25;

/// Cell centres in the mark's coordinates (`viewBox -46 -46 92 92`).
const CELLS: [(f32, f32, Fill); 7] = [
    (0.0, 0.0, Fill::Warm),
    (24.25, 0.0, Fill::Warm),
    (12.12, 21.0, Fill::Mid),
    (-12.12, 21.0, Fill::Light),
    (-24.25, 0.0, Fill::Mid),
    (-12.12, -21.0, Fill::Warm),
    (12.12, -21.0, Fill::Mid),
];
/// A cell's vertex radius and half width (the kit's 14 × 0.9 gap scale).
const R: f32 = 12.6;
const HALF_W: f32 = 10.91;
/// The mark's extent: x ±35.16, y ±33.6.
const EXTENT_H: f32 = 67.2;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Fill {
    Warm,
    Mid,
    Light,
}

/// `#f0b878` → `#d98a44` (activeLight → activeDark), the kit's warm
/// gradient, from the top-left of a cell (x 0 → .4, y 0 → 1 of its box).
fn warm(u: f32, v: f32) -> Color {
    // In three bands: at this size they read better than a smooth ramp.
    let t = (((0.4 * u + v) / 1.16).clamp(0.0, 1.0) * 3.0).floor().min(2.0) / 2.0;
    let l = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
    Color::Rgb(l(0xf0, 0xd9), l(0xb8, 0x8a), l(0x78, 0x44))
}

fn cell_color(fill: Fill, u: f32, v: f32) -> Color {
    match fill {
        Fill::Warm => warm(u, v),
        Fill::Mid => Color::Rgb(0xf0, 0xe0, 0xb8),
        Fill::Light => Color::Rgb(0xfb, 0xf1, 0xd8),
    }
}

/// The colour of pixel (`px`, `py`) of the `WIDTH` × `2·HEIGHT` grid, or
/// `None` where the background shows (outside the mark, or in a gap).
fn pixel(px: u16, py: u16) -> Option<Color> {
    const SS: u16 = 4;
    let scale = (2 * HEIGHT) as f32 / EXTENT_H;
    let mut hits = [0u16; 7];
    for sy in 0..SS {
        for sx in 0..SS {
            let x = (px as f32 + (sx as f32 + 0.5) / SS as f32 - WIDTH as f32 / 2.0) / scale;
            let y = (py as f32 + (sy as f32 + 0.5) / SS as f32 - HEIGHT as f32) / scale;
            for (i, (cx, cy, _)) in CELLS.iter().enumerate() {
                let (dx, dy) = ((x - cx).abs(), (y - cy).abs());
                if dx <= HALF_W && dy <= R - dx * (R / 2.0) / HALF_W {
                    hits[i] += 1;
                }
            }
        }
    }
    let (i, n) = hits.iter().enumerate().max_by_key(|(_, n)| **n)?;
    if (*n as f32) < 0.6 * (SS * SS) as f32 {
        return None;
    }
    let (cx, cy, fill) = CELLS[i];
    let x = (px as f32 + 0.5 - WIDTH as f32 / 2.0) / scale;
    let y = (py as f32 + 0.5 - HEIGHT as f32) / scale;
    let (u, v) = ((x - cx + HALF_W) / (2.0 * HALF_W), (y - cy + R) / (2.0 * R));
    Some(cell_color(fill, u, v))
}

/// Write the mark into `buf` with its top-left at `at`, clipped to the
/// buffer. Cells with no pixel are left alone (the terminal's background).
pub fn render(buf: &mut Buffer, at: (u16, u16), t: &Theme) {
    for row in 0..HEIGHT {
        for col in 0..WIDTH {
            let (x, y) = (at.0 + col, at.1 + row);
            if !buf.area.contains((x, y).into()) {
                continue;
            }
            let (top, bottom) = (pixel(col, 2 * row), pixel(col, 2 * row + 1));
            let (sym, style) = match (top, bottom) {
                (None, None) => continue,
                _ if t.mono => {
                    let sym = match (top.is_some(), bottom.is_some()) {
                        (true, true) => "█",
                        (true, false) => "▀",
                        _ => "▄",
                    };
                    (sym, Style::default())
                }
                (Some(a), Some(b)) => ("▀", Style::default().fg(a).bg(b)),
                (Some(a), None) => ("▀", Style::default().fg(a)),
                (None, Some(b)) => ("▄", Style::default().fg(b)),
            };
            buf[(x, y)].set_symbol(sym).set_style(style);
        }
    }
}

/// Draw the mark centred in `area` if it fits; true if it was drawn.
pub fn draw(f: &mut Frame, area: Rect, t: &Theme) -> bool {
    // Block art isn't ASCII: --ascii leaves the mark out.
    if t.ascii || area.width < WIDTH || area.height < HEIGHT {
        return false;
    }
    let at = (area.x + (area.width - WIDTH) / 2, area.y + (area.height - HEIGHT) / 2);
    render(f.buffer_mut(), at, t);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(t: &Theme) -> String {
        let mut buf = Buffer::empty(Rect::new(0, 0, WIDTH, HEIGHT));
        render(&mut buf, (0, 0), t);
        (0..HEIGHT).map(|y| (0..WIDTH).map(|x| buf[(x, y)].symbol()).collect::<String>() + "\n").collect()
    }

    #[test]
    fn the_mark_fills_its_box_and_is_symmetric_in_shape() {
        let s = text(&Theme::DARK);
        let rows: Vec<&str> = s.lines().collect();
        assert_eq!(rows.len(), HEIGHT as usize);
        assert!(rows.iter().all(|r| r.chars().any(|c| c != ' ')), "every row has ink:\n{s}");
        // The silhouette is left-right symmetric (the colours aren't).
        let mono = text(&Theme::MONO);
        for r in mono.lines() {
            let mirrored: String = r.chars().rev().collect();
            assert_eq!(r.replace(['▀', '▄', '█'], "#"), mirrored.replace(['▀', '▄', '█'], "#"), "\n{mono}");
        }
    }

    #[test]
    fn mono_has_no_colour() {
        let mut buf = Buffer::empty(Rect::new(0, 0, WIDTH, HEIGHT));
        render(&mut buf, (0, 0), &Theme::MONO);
        assert!(buf.content().iter().all(|c| c.fg == Color::Reset && c.bg == Color::Reset));
    }

    #[test]
    fn clipped_at_the_buffer_edge() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 5, 3));
        render(&mut buf, (2, 1), &Theme::DARK);
    }
}
