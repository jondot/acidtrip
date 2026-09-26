//! The document model: cells, layers, canvas, metadata.
//!
//! Every mutation goes through [`crate::tx::Transaction`]; nothing here is
//! mutated directly by UI code except via `Document::apply`/`revert`.

use serde::{Deserialize, Serialize};

use crate::color::{Color, Palette};
use crate::cp437;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Cell {
    pub ch: char,
    pub fg: Color,
    pub bg: Color,
}

impl Default for Cell {
    fn default() -> Self {
        Cell::BLANK
    }
}

impl Cell {
    pub const BLANK: Cell = Cell { ch: ' ', fg: Color::LIGHT_GRAY, bg: Color::BLACK };

    pub fn new(ch: char, fg: Color, bg: Color) -> Self {
        Cell { ch, fg, bg }
    }

    /// True when the cell shows nothing but the default black background.
    pub fn is_blank(&self) -> bool {
        matches!(self.ch, ' ' | '\u{0}' | '\u{A0}') && self.bg == Color::BLACK
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum DocKind {
    /// CP437 glyphs, 16-color palette, lossless to classic formats.
    #[default]
    Classic,
    /// Any Unicode char, RGB or palette colors.
    Modern,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum LayerKind {
    #[default]
    Normal,
    /// Shown dimmed in the editor, never exported (e.g. an imported reference image).
    Reference,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    pub name: String,
    pub visible: bool,
    pub locked: bool,
    pub kind: LayerKind,
    /// Row-major, `width * height` entries. `None` is transparent.
    pub cells: Vec<Option<Cell>>,
}

impl Layer {
    pub fn new(name: impl Into<String>, width: usize, height: usize) -> Self {
        Layer {
            name: name.into(),
            visible: true,
            locked: false,
            kind: LayerKind::Normal,
            cells: vec![None; width * height],
        }
    }

    pub fn is_empty(&self) -> bool {
        self.cells.iter().all(Option::is_none)
    }
}

/// The resizable part of a document: dimensions and layer stack.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Canvas {
    pub width: usize,
    pub height: usize,
    /// Bottom to top.
    pub layers: Vec<Layer>,
}

impl Canvas {
    pub fn new(width: usize, height: usize) -> Self {
        Canvas { width, height, layers: vec![Layer::new("Background", width, height)] }
    }

    #[inline]
    pub fn idx(&self, x: usize, y: usize) -> Option<usize> {
        (x < self.width && y < self.height).then_some(y * self.width + x)
    }

    /// Cell on a given layer (`None` if transparent or out of bounds).
    pub fn get(&self, layer: usize, x: usize, y: usize) -> Option<Cell> {
        let i = self.idx(x, y)?;
        self.layers.get(layer)?.cells[i]
    }

    /// Composite of all visible Normal layers at (x, y).
    pub fn composite(&self, x: usize, y: usize) -> Cell {
        let Some(i) = self.idx(x, y) else {
            return Cell::BLANK;
        };
        for l in self.layers.iter().rev() {
            if l.visible
                && l.kind == LayerKind::Normal
                && let Some(c) = l.cells[i]
            {
                return c;
            }
        }
        Cell::BLANK
    }

    /// Composite including Reference layers; the bool marks cells that come
    /// from a reference layer (the UI dims those).
    pub fn composite_with_reference(&self, x: usize, y: usize) -> (Cell, bool) {
        let Some(i) = self.idx(x, y) else {
            return (Cell::BLANK, false);
        };
        for l in self.layers.iter().rev() {
            if l.visible
                && let Some(c) = l.cells[i]
            {
                return (c, l.kind == LayerKind::Reference);
            }
        }
        (Cell::BLANK, false)
    }

    /// Flatten visible Normal layers into a single grid.
    pub fn flatten(&self) -> Grid {
        let mut g = Grid::new(self.width, self.height);
        for y in 0..self.height {
            for x in 0..self.width {
                g.set(x, y, self.composite(x, y));
            }
        }
        g
    }

    /// Resize all layers, anchoring at the top-left. New area is transparent
    /// (blank on the background layer).
    pub fn resized(&self, width: usize, height: usize) -> Canvas {
        let mut out = Canvas { width, height, layers: Vec::with_capacity(self.layers.len()) };
        for l in &self.layers {
            let mut nl = Layer { cells: vec![None; width * height], ..l.clone() };
            for y in 0..height.min(self.height) {
                for x in 0..width.min(self.width) {
                    nl.cells[y * width + x] = l.cells[y * self.width + x];
                }
            }
            out.layers.push(nl);
        }
        out
    }

    /// Rows that contain anything (last non-blank row + 1) in the composite.
    pub fn used_height(&self) -> usize {
        (0..self.height).rev().find(|&y| (0..self.width).any(|x| !self.composite(x, y).is_blank())).map_or(0, |y| y + 1)
    }
}

/// A flat grid of opaque cells (export/render input, clipboard, stencils).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Grid {
    pub width: usize,
    pub height: usize,
    pub cells: Vec<Cell>,
}

impl Grid {
    pub fn new(width: usize, height: usize) -> Self {
        Grid { width, height, cells: vec![Cell::BLANK; width * height] }
    }

    pub fn get(&self, x: usize, y: usize) -> Cell {
        if x < self.width && y < self.height { self.cells[y * self.width + x] } else { Cell::BLANK }
    }

    pub fn set(&mut self, x: usize, y: usize, c: Cell) {
        if x < self.width && y < self.height {
            self.cells[y * self.width + x] = c;
        }
    }

    pub fn row(&self, y: usize) -> &[Cell] {
        &self.cells[y * self.width..(y + 1) * self.width]
    }

    /// Index of the last row with any non-blank cell, +1.
    pub fn used_height(&self) -> usize {
        (0..self.height).rev().find(|&y| self.row(y).iter().any(|c| !c.is_blank())).map_or(0, |y| y + 1)
    }
}

/// A rectangular clip of cells with transparency (clipboard, stencils, AI
/// patches). `None` cells are transparent when stamped.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    pub width: usize,
    pub height: usize,
    pub cells: Vec<Option<Cell>>,
}

impl Clip {
    pub fn new(width: usize, height: usize) -> Self {
        Clip { width, height, cells: vec![None; width * height] }
    }

    pub fn get(&self, x: usize, y: usize) -> Option<Cell> {
        if x < self.width && y < self.height { self.cells[y * self.width + x] } else { None }
    }

    pub fn set(&mut self, x: usize, y: usize, c: Option<Cell>) {
        if x < self.width && y < self.height {
            self.cells[y * self.width + x] = c;
        }
    }

    pub fn from_grid(g: &Grid) -> Clip {
        Clip { width: g.width, height: g.height, cells: g.cells.iter().map(|&c| Some(c)).collect() }
    }

    pub fn to_grid(&self) -> Grid {
        Grid {
            width: self.width,
            height: self.height,
            cells: self.cells.iter().map(|c| c.unwrap_or(Cell::BLANK)).collect(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SauceMeta {
    pub title: String,
    pub author: String,
    pub group: String,
    /// CCYYMMDD; filled on save when empty.
    pub date: String,
    pub comments: Vec<String>,
    /// Attach a SAUCE record when saving classic formats.
    pub attach: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AspectRatio {
    #[default]
    Square,
    /// Legacy DOS 4:3 stretch.
    Legacy,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DocMeta {
    pub id: uuid::Uuid,
    pub kind: DocKind,
    pub palette: Palette,
    /// iCE colors: bright backgrounds instead of blink.
    pub ice: bool,
    /// SAUCE TInfoS font name, e.g. "IBM VGA".
    pub font_name: String,
    pub letter_spacing_9px: bool,
    pub aspect: AspectRatio,
    pub sauce: SauceMeta,
    pub tab_stops: Vec<u16>,
    /// The EXPORT panel's rows and folder, kept with the piece.
    #[serde(default, skip_serializing_if = "ExportSettings::is_empty")]
    pub exports: ExportSettings,
}

/// What the EXPORT panel writes when you press Export: one file (or one per
/// frame) for every row, into one folder.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ExportSettings {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub presets: Vec<ExportPreset>,
    /// Where the files go: relative to the piece's folder, or absolute.
    /// None = next to the piece.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
}

impl ExportSettings {
    pub fn is_empty(&self) -> bool {
        self.presets.is_empty() && self.folder.is_none()
    }
}

/// One export row: a format, a pixel scale and a file name pattern.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExportPreset {
    /// The format's default extension ("png", "svg", "ans"...).
    pub format: String,
    /// Pixel scale for bitmap formats (1..=8).
    #[serde(default = "one")]
    pub scale: u32,
    /// File name without extension; `{name}`, `{scale}`, `{frame}`, `{w}`
    /// and `{h}` are filled in.
    pub name: String,
    /// Format options (the io crate's save options, fields may be missing).
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub options: serde_json::Value,
}

impl Default for ExportPreset {
    fn default() -> Self {
        ExportPreset { format: "png".into(), scale: 1, name: "{name}".into(), options: serde_json::Value::Null }
    }
}

impl Default for DocMeta {
    fn default() -> Self {
        DocMeta {
            id: uuid::Uuid::new_v4(),
            kind: DocKind::Classic,
            palette: Palette::default(),
            ice: true,
            font_name: "IBM VGA".into(),
            letter_spacing_9px: false,
            aspect: AspectRatio::Square,
            sauce: SauceMeta::default(),
            tab_stops: (1..20).map(|i| i * 8).collect(),
            exports: ExportSettings::default(),
        }
    }
}

/// Frames per second a new animation plays at.
pub const DEFAULT_FPS: u32 = 8;
/// Id of the frame every document starts with (the one all pre-animation
/// files and edit logs refer to).
pub const FIRST_FRAME: u64 = 0;

fn one() -> u32 {
    1
}

fn default_fps() -> u32 {
    DEFAULT_FPS
}

/// One animation frame. The frame being edited keeps its canvas in
/// [`Document::canvas`] (its `canvas` here is `None`); the others hold
/// their own, all the same size.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Frame {
    /// Stable across moves, so edits find their frame wherever it is.
    pub id: u64,
    /// How many ticks (1/fps) the frame stays up.
    #[serde(default = "one")]
    pub hold: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canvas: Option<Canvas>,
}

/// The document's frames, in play order.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Frames {
    pub list: Vec<Frame>,
    #[serde(default = "default_fps")]
    pub fps: u32,
}

impl Default for Frames {
    fn default() -> Self {
        Frames { list: vec![Frame { id: FIRST_FRAME, hold: 1, canvas: None }], fps: DEFAULT_FPS }
    }
}

impl Frames {
    /// A plain, unanimated document: stored exactly as before frames existed.
    pub fn is_single(&self) -> bool {
        self.fps == DEFAULT_FPS && matches!(self.list.as_slice(), [f] if f.id == FIRST_FRAME && f.hold == 1)
    }
}

/// Every frame with its canvas, for replacing them all at once (version restore).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FrameSet {
    pub frames: Vec<Frame>,
    pub current: usize,
    pub fps: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(from = "DocRepr")]
pub struct Document {
    pub meta: DocMeta,
    /// The canvas of the frame being edited (the only one unless animated).
    pub canvas: Canvas,
    #[serde(default, skip_serializing_if = "Frames::is_single")]
    pub frames: Frames,
}

#[derive(Deserialize)]
struct DocRepr {
    meta: DocMeta,
    canvas: Canvas,
    #[serde(default)]
    frames: Frames,
}

impl From<DocRepr> for Document {
    fn from(r: DocRepr) -> Self {
        let mut d = Document { meta: r.meta, canvas: r.canvas, frames: r.frames };
        d.normalize_frames();
        d
    }
}

/// Which frame is current is view state: two documents with the same frames
/// are equal whichever one each is showing.
impl PartialEq for Document {
    fn eq(&self, o: &Self) -> bool {
        self.meta == o.meta
            && self.frames.fps == o.frames.fps
            && self.frames.list.len() == o.frames.list.len()
            && self.frames.list.iter().zip(&o.frames.list).enumerate().all(|(i, (a, b))| {
                a.id == b.id && a.hold == b.hold && self.frame_canvas(i) == o.frame_canvas(i)
            })
    }
}

impl Document {
    pub fn new(kind: DocKind, width: usize, height: usize) -> Self {
        let mut d = Document::with_canvas(DocMeta { kind, ..DocMeta::default() }, Canvas::new(width, height));
        // The background layer starts fully opaque-blank so it can be flood filled.
        d.canvas.layers[0].cells.iter_mut().for_each(|c| *c = Some(Cell::BLANK));
        d
    }

    /// A single-frame document around `canvas`.
    pub fn with_canvas(meta: DocMeta, canvas: Canvas) -> Self {
        Document { meta, canvas, frames: Frames::default() }
    }

    pub fn from_grid(kind: DocKind, g: &Grid) -> Self {
        let mut d = Document::new(kind, g.width, g.height);
        d.canvas.layers[0].cells = g.cells.iter().map(|&c| Some(c)).collect();
        d
    }

    pub fn width(&self) -> usize {
        self.canvas.width
    }

    pub fn height(&self) -> usize {
        self.canvas.height
    }

    pub fn is_classic(&self) -> bool {
        self.meta.kind == DocKind::Classic
    }

    /// Normalize a cell to what this document can hold (Classic: CP437 glyph,
    /// palette color <16, bg <8 unless iCE).
    pub fn conform(&self, c: Cell) -> Cell {
        conform_cell(&self.meta, c)
    }

    pub fn flatten(&self) -> Grid {
        self.canvas.flatten()
    }

    // ------------------------------------------------------------ frames

    pub fn frame_count(&self) -> usize {
        self.frames.list.len()
    }

    pub fn is_animated(&self) -> bool {
        self.frames.list.len() > 1
    }

    /// Index of the frame being edited.
    pub fn current_frame(&self) -> usize {
        self.frames.list.iter().position(|f| f.canvas.is_none()).unwrap_or(0)
    }

    /// Id of the frame being edited.
    pub fn frame_id(&self) -> u64 {
        self.frames.list.get(self.current_frame()).map_or(FIRST_FRAME, |f| f.id)
    }

    pub fn frame_index(&self, id: u64) -> Option<usize> {
        self.frames.list.iter().position(|f| f.id == id)
    }

    pub fn fps(&self) -> u32 {
        self.frames.fps.max(1)
    }

    /// Ticks frame `i` stays up.
    pub fn hold(&self, i: usize) -> u32 {
        self.frames.list.get(i).map_or(1, |f| f.hold.max(1))
    }

    /// Canvas of frame `i` (the current one's lives in `self.canvas`).
    pub fn frame_canvas(&self, i: usize) -> &Canvas {
        match self.frames.list.get(i).and_then(|f| f.canvas.as_ref()) {
            Some(c) => c,
            None => &self.canvas,
        }
    }

    pub fn canvas_of(&self, id: u64) -> Option<&Canvas> {
        let i = self.frame_index(id)?;
        Some(self.frame_canvas(i))
    }

    pub fn canvas_of_mut(&mut self, id: u64) -> Option<&mut Canvas> {
        let f = self.frames.list.iter_mut().find(|f| f.id == id)?;
        Some(match &mut f.canvas {
            Some(c) => c,
            None => &mut self.canvas,
        })
    }

    /// Edit frame `i` from now on. Which frame is shown is view state, like
    /// the scroll position: not an edit, not undoable, not shared. Returns
    /// whether anything changed.
    pub fn show_frame(&mut self, i: usize) -> bool {
        let cur = self.current_frame();
        if i == cur || i >= self.frames.list.len() {
            return false;
        }
        let next = self.frames.list[i].canvas.take().unwrap_or_else(|| self.blank_frame_canvas());
        let prev = std::mem::replace(&mut self.canvas, next);
        self.frames.list[cur].canvas = Some(prev);
        true
    }

    /// A fresh id no frame has.
    pub fn new_frame_id(&self) -> u64 {
        loop {
            let id = uuid::Uuid::new_v4().as_u64_pair().0 | 1;
            if self.frame_index(id).is_none() {
                return id;
            }
        }
    }

    /// An empty canvas with the current frame's layers (names and flags;
    /// reference layers keep their image, so tracing works on every frame).
    pub fn blank_frame_canvas(&self) -> Canvas {
        let c = &self.canvas;
        Canvas {
            width: c.width,
            height: c.height,
            layers: c
                .layers
                .iter()
                .enumerate()
                .map(|(i, l)| Layer {
                    cells: match l.kind {
                        LayerKind::Reference => l.cells.clone(),
                        LayerKind::Normal => vec![(i == 0).then_some(Cell::BLANK); c.width * c.height],
                    },
                    ..l.clone()
                })
                .collect(),
        }
    }

    /// All frames with their canvases.
    pub fn frame_set(&self) -> FrameSet {
        FrameSet {
            frames: (0..self.frame_count())
                .map(|i| Frame { canvas: Some(self.frame_canvas(i).clone()), ..self.frames.list[i].clone() })
                .collect(),
            current: self.current_frame(),
            fps: self.frames.fps,
        }
    }

    /// Replace every frame (and the canvas) with `set`.
    pub fn set_frame_set(&mut self, set: &FrameSet) {
        let mut list = set.frames.clone();
        if list.is_empty() {
            return;
        }
        let cur = set.current.min(list.len() - 1);
        if let Some(c) = list[cur].canvas.take() {
            self.canvas = c;
        }
        self.frames = Frames { list, fps: set.fps };
        self.normalize_frames();
    }

    /// Keep the frame invariants: at least one frame, exactly one without a
    /// canvas of its own (the current one), every canvas the document's size.
    pub fn normalize_frames(&mut self) {
        if self.frames.list.is_empty() {
            self.frames.list.push(Frame { id: FIRST_FRAME, hold: 1, canvas: None });
        }
        if !self.frames.list.iter().any(|f| f.canvas.is_none()) {
            self.frames.list[0].canvas = None;
        }
        let cur = self.current_frame();
        let (w, h) = (self.canvas.width, self.canvas.height);
        let blank = self.blank_frame_canvas();
        for (i, f) in self.frames.list.iter_mut().enumerate() {
            f.hold = f.hold.max(1);
            match &mut f.canvas {
                None if i != cur => f.canvas = Some(blank.clone()),
                Some(c) if c.width != w || c.height != h => *c = c.resized(w, h),
                _ => {}
            }
        }
        self.frames.fps = self.frames.fps.clamp(1, 60);
    }
}

/// Normalize a cell for a document with `meta` (see [`Document::conform`]).
pub fn conform_cell(meta: &DocMeta, c: Cell) -> Cell {
    if meta.kind == DocKind::Modern {
        return c;
    }
    let pal = &meta.palette;
    let fix = |col: Color, limit: usize| match col {
        Color::Pal(i) if (i as usize) < limit => Color::Pal(i),
        other => Color::Pal(pal.nearest(other.rgb(pal), limit)),
    };
    let ch = if cp437::is_cp437(c.ch) { c.ch } else { cp437::to_char(cp437::from_char_lossy(c.ch)) };
    Cell { ch, fg: fix(c.fg, 16), bg: fix(c.bg, if meta.ice { 16 } else { 8 }) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composite_top_layer_wins_and_transparency_shows_through() {
        let mut d = Document::new(DocKind::Classic, 4, 2);
        d.canvas.layers.push(Layer::new("top", 4, 2));
        let red = Cell::new('X', Color::Pal(4), Color::BLACK);
        d.canvas.layers[1].cells[1] = Some(red);
        assert_eq!(d.canvas.composite(1, 0), red);
        assert_eq!(d.canvas.composite(0, 0), Cell::BLANK);
    }

    #[test]
    fn resize_keeps_content() {
        let mut d = Document::new(DocKind::Classic, 3, 3);
        d.canvas.layers[0].cells[4] = Some(Cell::new('A', Color::WHITE, Color::BLACK));
        let c = d.canvas.resized(5, 2);
        assert_eq!(c.get(0, 1, 1).unwrap().ch, 'A');
        assert_eq!(c.layers[0].cells.len(), 10);
    }

    #[test]
    fn conform_classic_downsamples() {
        let d = Document::new(DocKind::Classic, 1, 1);
        let c = d.conform(Cell::new('╭', Color::Rgb(250, 250, 90), Color::Rgb(0, 0, 170)));
        assert_eq!(c.ch, '┌');
        assert_eq!(c.fg, Color::Pal(14));
        assert_eq!(c.bg, Color::Pal(1));
    }
}
