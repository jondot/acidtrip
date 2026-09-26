//! Tool definitions (name, description, JSON input schema), shared by MCP
//! and the Messages API.

use serde_json::{Value, json};

/// Shared description of the color argument format.
pub const COLOR_HELP: &str = "Color: a palette index 0-15 in VGA/DOS order - 0 black, 1 blue, 2 green, 3 cyan, \
4 red, 5 magenta, 6 brown, 7 light gray, 8 dark gray, 9 light blue, 10 light green, 11 light cyan, \
12 light red, 13 light magenta, 14 yellow, 15 white - or one of those names, or \"#rrggbb\". \
This is NOT ANSI SGR order (in SGR 1 is red; here 1 is blue). Classic docs snap #rrggbb to the \
nearest palette color; backgrounds 8-15 need iCE (on by default).";

const CH_HELP: &str = "A single character (CP437 art glyphs: █ ▀ ▄ ▌ ▐ ░ ▒ ▓ ■ · ─│┌┐└┘├┤┬┴┼ ═║╔╗╚╝╠╣╦╩╬) \
or an integer CP437 code 0-255.";

fn color(desc: &str) -> Value {
    json!({ "type": ["integer", "string"], "description": format!("{desc}. {COLOR_HELP}") })
}

fn int(desc: &str) -> Value {
    json!({ "type": "integer", "description": desc })
}

fn boolean(desc: &str) -> Value {
    json!({ "type": "boolean", "description": desc })
}

fn string(desc: &str) -> Value {
    json!({ "type": "string", "description": desc })
}

fn ch(desc: &str) -> Value {
    json!({ "type": ["string", "integer"], "description": format!("{desc}. {CH_HELP}") })
}

fn enumeration(values: &[&str], desc: &str) -> Value {
    json!({ "type": "string", "enum": values, "description": desc })
}

fn layer() -> Value {
    int("Layer index to edit (default: the AI layer; see get_info)")
}

fn tool(name: &str, description: &str, props: Value, required: &[&str]) -> Value {
    json!({
        "name": name,
        "description": description,
        "input_schema": { "type": "object", "properties": props, "required": required },
    })
}

fn rect_props(extra: Value) -> Value {
    let mut p = json!({
        "x": int("Left column (0-based)"),
        "y": int("Top row (0-based)"),
        "w": int("Width in cells"),
        "h": int("Height in cells"),
    });
    if let (Some(p), Value::Object(e)) = (p.as_object_mut(), extra) {
        p.extend(e);
    }
    p
}

const PIXEL_HELP: &str = "PIXEL SPACE: every text cell holds two square half-block pixels (top and bottom), \
so pixel x = column and pixel y = 2*row (+1 for the bottom half); a 80x25 canvas is 80x50 pixels. \
The tool packs pixels into ▀ ▄ █ cells and keeps the other half of each cell, so plot in any order. \
Classic docs: a cell holds at most one bright color (8-15) unless iCE is on; when two bright colors \
share a cell the lower one is dimmed, so align bright color boundaries to even pixel rows.";

/// `[{"name","description","input_schema"}]`
pub fn tool_definitions() -> Vec<Value> {
    vec![
        tool(
            "get_info",
            "Describe the document: kind (Classic = CP437 + 16 colors, Modern = Unicode + RGB), size, iCE, \
             layers (index, name, visibility, lock) and which one AI edits go to, the palette with names, \
             SAUCE credits, the file path and undo depth. Call this first if you don't know the canvas.",
            json!({}),
            &[],
        ),
        tool(
            "get_canvas",
            "Read the canvas as text rows with a column ruler, so you can count positions exactly. \
             Defaults to the used area of the composited image. format \"text\" = glyphs only; \"colors\" = \
             glyphs plus per-row fg/bg runs; \"ansi\" = 24-bit ANSI escape text. Use render_png to judge \
             how it actually looks.",
            rect_props(json!({
                "format": enumeration(&["text", "colors", "ansi"], "Output format (default text)"),
                "layer": int("Read a single layer instead of the composite"),
            })),
            &[],
        ),
        tool(
            "render_png",
            "Render the canvas (or a region) to a PNG with the real VGA font and palette and return it, so \
             you can SEE the art. Each cell is 8x16 px at scale 1. Use it after drawing to check your work \
             and fix mistakes. Optionally also writes the PNG to `path`.",
            rect_props(json!({
                "scale": int("Pixel scale 1-4 (default 1)"),
                "path": string("Optional .png file to also write"),
            })),
            &[],
        ),
        tool(
            "new_canvas",
            "Replace the document with an empty canvas (undoable). Standard ANSI art is 80 columns wide; \
             height grows as needed. Layers are kept (cleared).",
            json!({
                "width": int("Columns (default 80)"),
                "height": int("Rows (default 25)"),
                "kind": enumeration(&["classic", "modern"], "classic = CP437 + 16-color VGA palette (default, exports to .ans); modern = any Unicode + RGB"),
                "ice": boolean("iCE colors: allow bright backgrounds 8-15 (default true)"),
            }),
            &[],
        ),
        tool(
            "resize",
            "Resize the canvas, anchored at the top-left. New area is empty.",
            json!({ "width": int("Columns"), "height": int("Rows") }),
            &["width", "height"],
        ),
        tool(
            "set_cells",
            "Set individual cells. Omitted ch/fg/bg keep the cell's current value. Best for small precise \
             touch-ups; use the shape tools for larger areas.",
            json!({
                "cells": {
                    "type": "array",
                    "description": "Cells to set",
                    "items": {
                        "type": "object",
                        "properties": { "x": int("Column"), "y": int("Row"), "ch": ch("Glyph"), "fg": color("Foreground"), "bg": color("Background") },
                        "required": ["x", "y"],
                    },
                },
                "layer": layer(),
            }),
            &["cells"],
        ),
        tool(
            "put_text",
            "Write text at column x, row y. '\\n' starts a new row at the same x. Text is typed verbatim with \
             one color pair; use CP437 glyphs for art (░▒▓█ ▀▄▌▐ box drawing). Nothing wraps; text past the \
             right edge is clipped.",
            json!({
                "x": int("Column"),
                "y": int("Row"),
                "text": string("Text, may contain newlines"),
                "fg": color("Foreground (default 7 light gray)"),
                "bg": color("Background (default 0 black)"),
                "transparent_spaces": boolean("Spaces leave existing cells untouched (default false)"),
                "layer": layer(),
            }),
            &["x", "y", "text"],
        ),
        tool(
            "fill_rect",
            "Fill a rectangle of cells. `what` picks which parts to write: all (glyph + colors, default), \
             char (glyph only), fg, bg, or colors (fg + bg, keep glyphs - recolor existing art).",
            rect_props(json!({
                "ch": ch("Glyph (default █)"),
                "fg": color("Foreground (default 7)"),
                "bg": color("Background (default 0)"),
                "what": enumeration(&["all", "char", "fg", "bg", "colors"], "What to write (default all)"),
                "layer": layer(),
            })),
            &["x", "y", "w", "h"],
        ),
        tool(
            "draw_box",
            "Draw a rectangular frame with matching corner/edge glyphs. Styles: single ┌─┐, double ╔═╗, \
             double_h ╒═╕, double_v ╓─╖, block (solid █), rounded ╭─╮ (Modern only; Classic falls back to \
             single), brush (every edge cell uses `ch`). filled=true also fills the inside.",
            rect_props(json!({
                "style": enumeration(&["single", "double", "double_h", "double_v", "block", "rounded", "brush"], "Frame style (default single)"),
                "filled": boolean("Fill the interior too (default false)"),
                "ch": ch("Glyph for style=brush and filled interiors (default █)"),
                "fg": color("Foreground (default 7)"),
                "bg": color("Background (default 0)"),
                "layer": layer(),
            })),
            &["x", "y", "w", "h"],
        ),
        tool(
            "draw_line",
            "Draw a straight line of glyphs between two cells (Bresenham, endpoints inclusive).",
            json!({
                "x0": int("Start column"), "y0": int("Start row"), "x1": int("End column"), "y1": int("End row"),
                "ch": ch("Glyph (default █)"),
                "fg": color("Foreground (default 7)"),
                "bg": color("Background (default 0)"),
                "layer": layer(),
            }),
            &["x0", "y0", "x1", "y1"],
        ),
        tool(
            "draw_ellipse",
            "Draw an ellipse of glyphs inscribed in the rectangle x,y,w,h (cell space; cells are twice as \
             tall as wide, so a round circle needs w = 2*h). For smooth round shapes prefer pixel_ellipse.",
            rect_props(json!({
                "filled": boolean("Fill it (default false)"),
                "ch": ch("Glyph (default █)"),
                "fg": color("Foreground (default 7)"),
                "bg": color("Background (default 0)"),
                "layer": layer(),
            })),
            &["x", "y", "w", "h"],
        ),
        tool(
            "flood_fill",
            "Flood fill the connected area around (x, y) that matches the start cell (by glyph, fg and bg \
             unless `match` says otherwise). `mode`: all (write glyph + colors, default), colors, fg or bg.",
            json!({
                "x": int("Column"), "y": int("Row"),
                "ch": ch("Glyph (default █)"),
                "fg": color("Foreground (default 7)"),
                "bg": color("Background (default 0)"),
                "mode": enumeration(&["all", "colors", "fg", "bg"], "What to write (default all)"),
                "match": {
                    "type": "object",
                    "description": "Which attributes must equal the start cell (default all true)",
                    "properties": { "ch": boolean("Match glyph"), "fg": boolean("Match foreground"), "bg": boolean("Match background") },
                },
                "layer": layer(),
            }),
            &["x", "y"],
        ),
        tool(
            "erase_rect",
            "Erase a rectangle to transparent (blank on the background layer).",
            rect_props(json!({ "layer": layer() })),
            &["x", "y", "w", "h"],
        ),
        tool(
            "pixel_set",
            &format!(
                "Plot individual pixels. {PIXEL_HELP} Prefer pixel tools for picture-like art; use text tools for lettering and ░▒▓ textures."
            ),
            json!({
                "pixels": {
                    "type": "array",
                    "description": "Pixels as [x, y, color] triples or {x, y, color} objects (y in pixel rows)",
                    "items": { "type": ["array", "object"] },
                },
                "layer": layer(),
            }),
            &["pixels"],
        ),
        tool(
            "pixel_line",
            &format!("Draw a 1-pixel line. {PIXEL_HELP}"),
            json!({
                "x0": int("Start pixel x"), "y0": int("Start pixel y"), "x1": int("End pixel x"), "y1": int("End pixel y"),
                "color": color("Pixel color"),
                "layer": layer(),
            }),
            &["x0", "y0", "x1", "y1", "color"],
        ),
        tool(
            "pixel_rect",
            &format!(
                "Draw a pixel rectangle (filled by default). With color2 the fill becomes a Bayer ordered \
                 dither of color and color2 (`mix` = share of color2, 0-1): fake gradients by stepping mix \
                 across adjacent rects. {PIXEL_HELP}"
            ),
            json!({
                "x": int("Left pixel x"), "y": int("Top pixel y"), "w": int("Width in pixels"), "h": int("Height in pixels"),
                "color": color("Pixel color"),
                "color2": color("Optional second color for a dithered fill"),
                "mix": { "type": "number", "description": "Fraction of color2, 0.0-1.0 (default 0.5)" },
                "filled": boolean("Filled (default true); false draws a 1px outline"),
                "layer": layer(),
            }),
            &["x", "y", "w", "h", "color"],
        ),
        tool(
            "pixel_ellipse",
            &format!(
                "Draw a pixel ellipse centered at (cx, cy) with radii rx, ry - filled by default. Layer offset \
                 filled ellipses (dark base, lighter inner, white speck) for cartoon shading. {PIXEL_HELP}"
            ),
            json!({
                "cx": int("Center pixel x"), "cy": int("Center pixel y"), "rx": int("Horizontal radius"), "ry": int("Vertical radius"),
                "color": color("Pixel color"),
                "filled": boolean("Filled (default true)"),
                "layer": layer(),
            }),
            &["cx", "cy", "rx", "ry", "color"],
        ),
        tool(
            "brush_stroke",
            "Paint a freehand stroke with a brush, like the app's smart pen: the path is inked in 8x16 \
             sub-cell pixels and every touched cell becomes the glyph that best reproduces the ink. Brushes: \
             ink (crisp half-block lines), brush pen (tapered ends), marker (bold square tip), calligraphy \
             (45° flat nib: thick/thin), airbrush (soft ░▒▓ glow, great for shading and highlights), soft \
             shade (never solid, 60% tone), chalk (textured), spray (scattered dots), ascii (text-art \
             characters), line art (─│┌┐ box lines). Points are in CELL coordinates with fractions allowed: \
             (10.5, 3.5) is the center of column 10, row 3; pass several points for curves (they are joined \
             by straight segments).",
            json!({
                "points": {
                    "type": "array",
                    "items": { "type": "array", "items": { "type": "number" }, "minItems": 2, "maxItems": 2 },
                    "minItems": 1,
                    "description": "Path as [[x, y], ...] in cell units (fractions allowed)",
                },
                "brush": enumeration(
                    &["ink", "brush pen", "marker", "calligraphy", "airbrush", "soft shade", "chalk", "spray", "ascii", "line art"],
                    "Brush preset (default ink)",
                ),
                "size": json!({ "type": "number", "description": "Tip radius in sub-cell pixels (a cell is 8 wide, 16 tall); default per brush" }),
                "fg": color("Ink color (default 15 white)"),
                "layer": layer(),
            }),
            &["points"],
        ),
        tool(
            "pixel_fill",
            &format!("Flood fill connected same-colored pixels starting at pixel (x, y). {PIXEL_HELP}"),
            json!({ "x": int("Pixel x"), "y": int("Pixel y"), "color": color("Fill color"), "layer": layer() }),
            &["x", "y", "color"],
        ),
        tool(
            "banner",
            "Stamp big lettering with a TheDraw (.tdf) or FIGlet font - the classic scene logo look. Color \
             TDF fonts carry their own colors; Block/Outline/FIGlet fonts use fg/bg. Glyph spaces are \
             transparent. Many TDF fonts are CAPS only. Call list_fonts first to pick a font id and see how \
             wide the text will be (80 columns is the usual limit).",
            json!({
                "text": string("Text to render (\\n for multiple lines)"),
                "font": string("Font id or name from list_fonts"),
                "x": int("Left column (default 0; ignored when center=true)"),
                "y": int("Top row (default 0)"),
                "center": boolean("Center horizontally on the canvas"),
                "outline_style": int("TDF outline style 0-18 for Outline fonts (default 0)"),
                "spacing": int("Extra columns between letters, may be negative (default 0)"),
                "fg": color("Foreground for non-color fonts (default 15)"),
                "bg": color("Background for non-color fonts (default 0)"),
                "layer": layer(),
            }),
            &["text", "font"],
        ),
        tool(
            "list_fonts",
            "List available text fonts (id, name, kind, charset). `filter` matches id/name/kind. With `text`, \
             also reports the rendered width x height of that text for each listed font (max 30 fonts).",
            json!({ "filter": string("Substring filter"), "text": string("Optional text to measure") }),
            &[],
        ),
        tool(
            "list_stencils",
            "Search the stencil library (reusable clips: logos, frames, ornaments) by name, tags or author.",
            json!({ "query": string("Search text (empty lists everything)") }),
            &[],
        ),
        tool(
            "stamp_stencil",
            "Stamp a stencil at (x, y). mode: transparent (empty cells show what's below, default), opaque, \
             or under (only fills empty cells).",
            json!({
                "id": string("Stencil id from list_stencils"),
                "x": int("Left column"), "y": int("Top row"),
                "mode": enumeration(&["transparent", "opaque", "under"], "Stamp mode (default transparent)"),
                "layer": layer(),
            }),
            &["id", "x", "y"],
        ),
        tool(
            "save_stencil",
            "Save a region of the canvas (composite, or one layer) to the stencil library for reuse.",
            rect_props(json!({
                "name": string("Stencil name"),
                "tags": { "type": "array", "items": { "type": "string" }, "description": "Tags" },
                "layer": int("Copy one layer (with transparency) instead of the composite"),
            })),
            &["x", "y", "w", "h", "name"],
        ),
        tool(
            "transform_region",
            "Transform a rectangular region in place: flip_x / flip_y (mirrors glyphs too: ▌↔▐, ┌↔┐, ▀↔▄), \
             rotate_180, justify_left / justify_center / justify_right (per row), or outline (draw a box of \
             `style` around the region's inside edge).",
            rect_props(json!({
                "op": enumeration(&["flip_x", "flip_y", "rotate_180", "justify_left", "justify_center", "justify_right", "outline"], "Operation"),
                "style": enumeration(&["single", "double", "double_h", "double_v", "block", "rounded"], "Box style for outline (default single)"),
                "fg": color("Outline foreground (default 7)"),
                "bg": color("Outline background (default 0)"),
                "layer": layer(),
            })),
            &["x", "y", "w", "h", "op"],
        ),
        tool(
            "move_region",
            "Move (or with copy=true, duplicate) a rectangular region of a layer to (to_x, to_y).",
            rect_props(json!({
                "to_x": int("Destination column"),
                "to_y": int("Destination row"),
                "copy": boolean("Keep the original (default false = move)"),
                "mode": enumeration(&["opaque", "transparent", "under"], "How the region lands (default opaque)"),
                "layer": layer(),
            })),
            &["x", "y", "w", "h", "to_x", "to_y"],
        ),
        tool(
            "layer",
            "Layer operations. action: list; add (name, index = position, default top; becomes the target); \
             select (index: later edits go there); set_props (index + any of name/visible/locked/reference); \
             merge_down (index into the layer below); remove (index).",
            json!({
                "action": enumeration(&["list", "add", "select", "set_props", "merge_down", "remove"], "What to do"),
                "index": int("Layer index"),
                "name": string("Layer name"),
                "visible": boolean("Visibility"),
                "locked": boolean("Lock against edits"),
                "reference": boolean("Reference layer: shown dimmed, never exported"),
            }),
            &["action"],
        ),
        tool(
            "import_image",
            "Convert an image (PNG/GIF/JPEG/WebP/BMP path or base64) into ANSI cells and stamp it at (x, y), \
             in the document's colors. A preset picks good settings: photo (best fit), scene (CP437 blocks, \
             shades, ordered dither), pixel_art (sharp half blocks), cel (anime/cartoons: outlines kept, flat fills), \
             comic (ink drawings and comics: every line kept), line_art (keeps dark lines), ascii. \
             Or set style: halfblock = 2 square pixels per cell; blocks = best-fit block/shade glyphs; \
             ascii = density ramp. The canvas grows downwards if needed. Inspect with render_png after.",
            json!({
                "path": string("Image file path"),
                "png_base64": string("Base64-encoded image instead of path"),
                "x": int("Left column (default 0)"),
                "y": int("Top row (default 0)"),
                "width": int("Target width in cells (default: canvas width - x)"),
                "preset": enumeration(&["photo", "scene", "pixel_art", "cel", "comic", "line_art", "ascii"], "Starting settings (overrides style/dither)"),
                "style": enumeration(&["halfblock", "blocks", "ascii"], "Conversion style (default halfblock)"),
                "dither": boolean("Error-diffusion dithering for photos/gradients (default false; not for flat cel art)"),
                "ink": boolean("Keep thin dark outlines (line art/cartoons) (default false)"),
                "layer": layer(),
            }),
            &[],
        ),
        tool(
            "undo",
            "Undo the last edit(s) - yours or the user's; each tool call is one step.",
            json!({ "steps": int("How many (default 1)") }),
            &[],
        ),
        tool("redo", "Redo edit(s) that were just undone.", json!({ "steps": int("How many (default 1)") }), &[]),
        tool(
            "save",
            "Save the document. Format comes from the extension: .acid (native, all layers), .ans (CP437 \
             ANSI), .utf8ans, .bin, .xb, .adf, .idf, .tnd, .pcb, .avt, .asc, .png, .gif, .svg, .html, .tsx \
             and more. Optional SAUCE credits are stored in the doc (undoable) and attached to the file.",
            json!({
                "path": string("File path (default: the document's current file)"),
                "title": string("SAUCE title (max 35 chars)"),
                "author": string("SAUCE author (max 20 chars)"),
                "group": string("SAUCE group (max 20 chars)"),
            }),
            &[],
        ),
        tool(
            "load",
            "Open a file (any supported format; images are converted) replacing the document (undoable).",
            json!({ "path": string("File path") }),
            &["path"],
        ),
    ]
}

/// Tools only the MCP server offers (they don't edit the document; the
/// client model does the letter reading so no API key is needed).
pub fn mcp_tool_definitions() -> Vec<Value> {
    vec![
        tool(
            "harvest_candidates",
            "Font/stencil harvester step 1. Fetch scene art (local .ans/.xb/.bin file or dir, .zip pack, \
             http(s) URL, or \"16colo.rs:<pack>\" / a 16colo.rs pack URL), find logo-like blobs and return \
             each as an image with an id and its size in cells. Then read each logo's letters yourself and \
             call harvest_commit.",
            json!({
                "source": string("Where to fetch art from"),
                "limit": int("Max candidates to return (default 8)"),
            }),
            &["source"],
        ),
        tool(
            "harvest_commit",
            "Font/stencil harvester step 2. For each candidate id: optionally give a reading - the text, the \
             column span of every letter (x0..x1 inclusive, in cells, 0 = the candidate's left edge; each \
             cell is 8 px wide in the image) and a short style tag. Letters become glyphs of a partial TDF \
             color font grouped by artist + style (written to the font library); every candidate also \
             becomes a stencil. Everything keeps the artist's attribution.",
            json!({
                "ids": { "type": "array", "items": { "type": "string" }, "description": "Candidate ids to keep as stencils only" },
                "readings": {
                    "type": "array",
                    "description": "Letter readings",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": string("Candidate id"),
                            "text": string("The text the logo spells"),
                            "style": string("Short style tag, e.g. \"chrome\", \"fire\", \"blocky\""),
                            "letters": {
                                "type": "array",
                                "items": {
                                    "type": "object",
                                    "properties": { "char": string("Letter"), "x0": int("First column"), "x1": int("Last column (inclusive)") },
                                    "required": ["char", "x0", "x1"],
                                },
                            },
                        },
                        "required": ["id", "letters"],
                    },
                },
            }),
            &[],
        ),
    ]
}

/// Look up a definition by name (editing tools and MCP-only tools).
pub fn find(name: &str) -> Option<Value> {
    tool_definitions().into_iter().chain(mcp_tool_definitions()).find(|t| t["name"] == name)
}

/// One-line summary of a tool's arguments: `x*, y*, text*, fg, bg` (* = required).
pub fn args_summary(name: &str) -> String {
    let Some(t) = find(name) else {
        return String::new();
    };
    let req: Vec<&str> = t["input_schema"]["required"]
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let props = t["input_schema"]["properties"].as_object().cloned().unwrap_or_default();
    props
        .keys()
        .map(|k| if req.contains(&k.as_str()) { format!("{k}*") } else { k.clone() })
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn every_tool_is_well_formed_and_unique() {
        let mut names = HashSet::new();
        for t in tool_definitions().into_iter().chain(mcp_tool_definitions()) {
            let name = t["name"].as_str().unwrap();
            assert!(names.insert(name.to_string()), "duplicate {name}");
            assert!(name.chars().all(|c| c.is_ascii_lowercase() || c == '_' || c.is_ascii_digit()));
            assert!(t["description"].as_str().unwrap().len() > 20, "{name} description");
            let s = &t["input_schema"];
            assert_eq!(s["type"], "object", "{name}");
            let props = s["properties"].as_object().unwrap();
            for r in s["required"].as_array().unwrap() {
                assert!(props.contains_key(r.as_str().unwrap()), "{name} requires unknown {r}");
            }
            for (k, v) in props {
                assert!(v.get("type").is_some(), "{name}.{k} has no type");
                assert!(v.get("description").is_some(), "{name}.{k} has no description");
            }
        }
        for n in ["get_info", "render_png", "banner", "pixel_rect", "layer", "undo", "save", "load"] {
            assert!(names.contains(n), "{n}");
        }
    }

    #[test]
    fn summary_marks_required() {
        let s = args_summary("put_text");
        assert!(s.contains("x*") && s.contains("text*") && s.contains("fg"));
    }
}
