//! The starter stencils compiled into the binary.

use acidtrip_core::{Cell, Clip, Color};

use super::{Stencil, StencilMeta};

const BLACK: Color = Color::BLACK;
const GRAY: Color = Color::LIGHT_GRAY;
const DARK_GRAY: Color = Color::Pal(8);
const WHITE: Color = Color::WHITE;
const BLUE: Color = Color::Pal(1);
const LIGHT_BLUE: Color = Color::Pal(9);
const LIGHT_CYAN: Color = Color::Pal(11);
const RED: Color = Color::Pal(4);
const LIGHT_RED: Color = Color::Pal(12);
const YELLOW: Color = Color::Pal(14);
const MAGENTA: Color = Color::Pal(5);

pub(super) fn starters() -> Vec<Stencil> {
    vec![
        mk(
            "box-single",
            "Single box",
            &["frame", "box", "border", "line"],
            text(&["┌────────┐", "│        │", "│        │", "└────────┘"], GRAY),
        ),
        mk(
            "box-double",
            "Double box",
            &["frame", "box", "border", "line"],
            text(&["╔════════╗", "║        ║", "║        ║", "╚════════╝"], WHITE),
        ),
        mk("shade-bar", "Shaded divider bar", &["divider", "shade", "gradient", "bar"], text(&["░▒▓█▓▒░"], LIGHT_BLUE)),
        mk("scene-gradient", "Scene gradient block", &["gradient", "shade", "fill", "scene"], scene_gradient()),
        mk("divider-ornate", "Ornate divider", &["divider", "line", "ornament"], text(&["·─═≡■≡═─·"], GRAY)),
        mk("arrow-right", "Arrow right", &["arrow", "pointer"], text(&["───►"], WHITE)),
        mk("arrow-down", "Arrow down", &["arrow", "pointer"], text(&["│", "│", "▼"], WHITE)),
        mk("star", "Small star", &["star", "sparkle", "shape"], text(&[" \\│/ ", "──☼──", " /│\\ "], YELLOW)),
        mk("heart", "Half-block heart", &["heart", "love", "shape", "pixel"], heart()),
        mk("checker", "Checker dither patch", &["dither", "pattern", "fill", "checker"], checker()),
        mk(
            "corner-tl",
            "Corner ornament top-left",
            &["corner", "ornament", "frame"],
            text(&["█▀▀▀▀·", "█", "·"], MAGENTA),
        ),
        mk(
            "corner-br",
            "Corner ornament bottom-right",
            &["corner", "ornament", "frame"],
            text(&["     ·", "     █", "·▄▄▄▄█"], MAGENTA),
        ),
    ]
}

fn mk(id: &str, name: &str, tags: &[&str], clip: Clip) -> Stencil {
    Stencil {
        meta: StencilMeta {
            id: format!("builtin-{id}"),
            name: name.into(),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            author: "acidtrip".into(),
            group: String::new(),
            source: "built-in".into(),
            license: "MIT".into(),
            generated: false,
            created: String::new(),
        },
        clip,
    }
}

/// Rows of text; spaces are transparent.
fn text(rows: &[&str], fg: Color) -> Clip {
    let w = rows.iter().map(|r| r.chars().count()).max().unwrap_or(0);
    let mut c = Clip::new(w, rows.len());
    for (y, r) in rows.iter().enumerate() {
        for (x, ch) in r.chars().enumerate() {
            if ch != ' ' {
                c.set(x, y, Some(Cell::new(ch, fg, BLACK)));
            }
        }
    }
    c
}

fn scene_gradient() -> Clip {
    let row = "░░▒▒▓▓████▓▓▒▒░░";
    let mut c = Clip::new(row.chars().count(), 3);
    for (y, fg) in [DARK_GRAY, BLUE, LIGHT_CYAN].into_iter().enumerate() {
        for (x, ch) in row.chars().enumerate() {
            c.set(x, y, Some(Cell::new(ch, fg, BLACK)));
        }
    }
    c
}

/// Two pixel rows per cell with half blocks; '#' is set, anything else empty.
fn pixels(rows: &[&str], fg: Color) -> Clip {
    let w = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    let h = rows.len().div_ceil(2);
    let on = |x: usize, y: usize| rows.get(y).is_some_and(|r| r.as_bytes().get(x) == Some(&b'#'));
    let mut c = Clip::new(w, h);
    for cy in 0..h {
        for x in 0..w {
            let ch = match (on(x, cy * 2), on(x, cy * 2 + 1)) {
                (true, true) => '█',
                (true, false) => '▀',
                (false, true) => '▄',
                (false, false) => continue,
            };
            c.set(x, cy, Some(Cell::new(ch, fg, BLACK)));
        }
    }
    c
}

fn heart() -> Clip {
    let mut c = pixels(&[".##.##.", "#######", "#######", ".#####.", "..###..", "...#..."], LIGHT_RED);
    // A darker shade along the bottom edge.
    for cell in c.cells[c.width * 2..].iter_mut().flatten() {
        cell.fg = RED;
    }
    c
}

fn checker() -> Clip {
    let mut c = Clip::new(5, 3);
    for y in 0..3 {
        for x in 0..5 {
            c.set(x, y, Some(Cell::new(if (x + y) % 2 == 0 { '▀' } else { '▄' }, GRAY, BLACK)));
        }
    }
    c
}
