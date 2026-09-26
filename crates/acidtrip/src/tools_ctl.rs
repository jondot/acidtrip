//! Tool state machine: turns mouse and keyboard input into transactions and
//! live previews. Tools themselves live in `acidtrip_core::tools`.

use acidtrip_core::charsets::{self, Charset};
use acidtrip_core::tools::brush::{BrushSpec, GlyphSet};
use acidtrip_core::tools::pattern::{self, Pattern, PatternCtx};
use acidtrip_core::tools::{self, BoxStyle, Brush, Ctx, FillMatch, PaintMode, Rect, ShapeFill, StampMode, Symmetry};
use acidtrip_core::{Cell, Clip, Color, LayerKind, TxBuilder};
use acidtrip_io::gradient;
use std::rc::Rc;

use crate::actions::Action;
use crate::colorfx::{ColorFx, Kind, Target};
use crate::tab::Tab;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Select,
    Text,
    Brush,
    Pen,
    Pixel,
    Line,
    Rect,
    Ellipse,
    Fill,
    /// Drag a color ramp over an area (the selection, or what Fill would fill).
    Gradient,
    Picker,
    Font,
    Stencil,
    /// Paint with a repeating tile (built-in or from a selection).
    Pattern,
    Shade,
    Colorize,
    Erase,
    /// The keyboard as a glyph board (the mouse paints like the brush).
    Art,
    /// Photo filters (iPhone / Instagram looks) over the layer or selection.
    Filters,
    /// Replace one color everywhere.
    Recolor,
}

impl Tool {
    pub const ALL: [Tool; 20] = [
        Tool::Select,
        Tool::Text,
        Tool::Brush,
        Tool::Pen,
        Tool::Pixel,
        Tool::Line,
        Tool::Rect,
        Tool::Ellipse,
        Tool::Fill,
        Tool::Gradient,
        Tool::Picker,
        Tool::Font,
        Tool::Stencil,
        Tool::Pattern,
        Tool::Shade,
        Tool::Colorize,
        Tool::Erase,
        Tool::Art,
        Tool::Filters,
        Tool::Recolor,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Tool::Select => "Select",
            Tool::Text => "Text",
            Tool::Brush => "Brush",
            Tool::Pen => "Pen",
            Tool::Pixel => "Pixel",
            Tool::Line => "Line",
            Tool::Rect => "Rect",
            Tool::Ellipse => "Ellipse",
            Tool::Fill => "Fill",
            Tool::Gradient => "Gradient",
            Tool::Picker => "Picker",
            Tool::Font => "Font",
            Tool::Stencil => "Stencil",
            Tool::Pattern => "Pattern",
            Tool::Shade => "Shade",
            Tool::Colorize => "Colorize",
            Tool::Erase => "Erase",
            Tool::Art => "Art",
            Tool::Filters => "Filters",
            Tool::Recolor => "Recolor",
        }
    }

    pub fn action(self) -> Action {
        match self {
            Tool::Select => Action::ToolSelect,
            Tool::Text => Action::ToolText,
            Tool::Brush => Action::ToolBrush,
            Tool::Pen => Action::ToolPen,
            Tool::Pixel => Action::ToolPixel,
            Tool::Line => Action::ToolLine,
            Tool::Rect => Action::ToolRect,
            Tool::Ellipse => Action::ToolEllipse,
            Tool::Fill => Action::ToolFill,
            Tool::Gradient => Action::ToolGradient,
            Tool::Picker => Action::ToolPicker,
            Tool::Font => Action::ToolFont,
            Tool::Stencil => Action::ToolStencil,
            Tool::Pattern => Action::ToolPattern,
            Tool::Shade => Action::ToolShade,
            Tool::Colorize => Action::ToolColorize,
            Tool::Erase => Action::ToolErase,
            Tool::Art => Action::ArtMode,
            Tool::Filters => Action::ToolFilters,
            Tool::Recolor => Action::ToolRecolor,
        }
    }

    pub fn from_action(a: Action) -> Option<Tool> {
        Tool::ALL.into_iter().find(|t| t.action() == a)
    }

    pub fn icon(self) -> char {
        match self {
            Tool::Select => '⬚',
            Tool::Text => 'T',
            Tool::Brush => '▓',
            Tool::Pen => '✎',
            Tool::Pixel => '▀',
            Tool::Line => '╱',
            Tool::Rect => '□',
            Tool::Ellipse => '○',
            Tool::Fill => '◘',
            Tool::Gradient => '▒',
            Tool::Picker => '¡',
            Tool::Font => 'Å',
            Tool::Stencil => '♣',
            Tool::Pattern => '▦',
            Tool::Shade => '░',
            Tool::Colorize => '♦',
            Tool::Erase => '×',
            Tool::Art => '▚',
            Tool::Filters => '◐',
            Tool::Recolor => '◈',
        }
    }

    /// One line on what the tool does (sidebar, hover tips).
    pub fn blurb(self) -> &'static str {
        match self {
            Tool::Select => "select, then move or change",
            Tool::Text => "click, then type",
            Tool::Brush => "paint the glyph cell by cell",
            Tool::Pen => "smooth strokes, many brushes",
            Tool::Pixel => "half-block pixels, 2 a cell",
            Tool::Line => "drag a line",
            Tool::Rect => "drag a rectangle",
            Tool::Ellipse => "drag an ellipse",
            Tool::Fill => "flood-fill an area",
            Tool::Gradient => "drag a color ramp over an area",
            Tool::Picker => "click to pick glyph + colors",
            Tool::Font => "big text in TheDraw fonts",
            Tool::Stencil => "stamp a saved stencil",
            Tool::Pattern => "paint with a repeating tile",
            Tool::Shade => "step cells through ░▒▓█",
            Tool::Colorize => "recolor ink, keep glyphs",
            Tool::Erase => "erase to transparent",
            Tool::Art => "the left hand types blocks",
            Tool::Filters => "photo filters: presets + sliders",
            Tool::Recolor => "replace one color everywhere",
        }
    }

    /// Tools that draw a shape between two points.
    pub fn is_shape(self) -> bool {
        matches!(self, Tool::Line | Tool::Rect | Tool::Ellipse)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FillMode {
    All,
    Char,
    Colors,
    Bg,
}

impl FillMode {
    pub fn matcher(self) -> FillMatch {
        match self {
            FillMode::All => FillMatch { ch: true, fg: true, bg: true },
            FillMode::Char => FillMatch { ch: true, fg: false, bg: false },
            FillMode::Colors => FillMatch { ch: false, fg: true, bg: true },
            FillMode::Bg => FillMatch { ch: false, fg: false, bg: true },
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            FillMode::All => "match all",
            FillMode::Char => "match char",
            FillMode::Colors => "match colors",
            FillMode::Bg => "match bg",
        }
    }
}

/// What the pattern tool paints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PatternMode {
    /// Brush strokes (a square dab, `pattern_size` wide).
    Brush,
    /// Drag a filled rectangle.
    Rect,
    /// Flood-fill an area, like the bucket.
    Fill,
}

impl PatternMode {
    pub fn name(self) -> &'static str {
        match self {
            PatternMode::Brush => "brush",
            PatternMode::Rect => "rect",
            PatternMode::Fill => "fill",
        }
    }
}

/// Where pattern tiles line up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PatternAnchor {
    /// The canvas origin: separate strokes join seamlessly.
    Canvas,
    /// Where each stroke, rectangle or fill starts.
    Start,
}

pub const PATTERN_SIZE_MAX: usize = 9;

#[derive(Clone, Debug)]
pub struct ToolOpts {
    pub brush_mode: PaintMode,
    pub rect_fill: ShapeFill,
    pub ellipse_fill: ShapeFill,
    pub box_style: BoxStyle,
    pub fill_mode: FillMode,
    pub symmetry: Symmetry,
    pub stamp_mode: StampMode,
    pub pixel_fill: bool,
    /// Shade brush: a plain click lightens (one-button mice can't right-click).
    pub shade_lighter: bool,
    pub pattern_mode: PatternMode,
    /// Pattern brush width in cells.
    pub pattern_size: usize,
    pub pattern_anchor: PatternAnchor,
    /// Paint patterns in the brush colors, even ones with colors of their own.
    pub pattern_recolor: bool,
    pub gradient: gradient::Options,
}

impl Default for ToolOpts {
    fn default() -> Self {
        ToolOpts {
            brush_mode: PaintMode::Char,
            rect_fill: ShapeFill::Outline,
            ellipse_fill: ShapeFill::Outline,
            box_style: BoxStyle::Single,
            fill_mode: FillMode::All,
            symmetry: Symmetry::None,
            stamp_mode: StampMode::Transparent,
            pixel_fill: false,
            shade_lighter: false,
            pattern_mode: PatternMode::Brush,
            pattern_size: 2,
            pattern_anchor: PatternAnchor::Canvas,
            pattern_recolor: false,
            gradient: gradient::Options::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FloatSource {
    Paste,
    Move,
    Font,
    Stencil,
}

/// A clip following the cursor, stamped with click/Space/Enter.
#[derive(Clone, Debug)]
pub struct Floating {
    pub clip: Clip,
    pub x: usize,
    pub y: usize,
    pub source: FloatSource,
    /// A block carried off the canvas (block menu › Move): it is still in
    /// place until it is put down, and then leaving its spot and landing
    /// are one undo step. Esc leaves it where it was.
    pub lift: Option<Lift>,
}

/// Where a carried block came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lift {
    pub doc: uuid::Uuid,
    pub layer: usize,
    pub rect: Rect,
}

#[derive(Clone, Debug, Default)]
pub enum Drag {
    #[default]
    None,
    Stroke {
        last: (usize, usize),
        button: Button,
    },
    Shape {
        start: (usize, usize),
        cur: (usize, usize),
        button: Button,
    },
    Selecting {
        start: (usize, usize),
        cur: (usize, usize),
    },
    Moving {
        grab: (usize, usize),
        /// Where the block was lifted from (Esc puts it back).
        from: (usize, usize),
    },
    /// A gradient being dragged over `mask` (picked at the press).
    Gradient {
        start: (usize, usize),
        cur: (usize, usize),
        button: Button,
        mask: Rc<Vec<bool>>,
    },
}

pub struct ToolState {
    pub tool: Tool,
    pub prev_tool: Tool,
    pub brush: Brush,
    pub opts: ToolOpts,
    pub drag: Drag,
    /// Keyboard shape drawing: first Space sets the anchor.
    pub anchor: Option<(usize, usize)>,
    pub floating: Option<Floating>,
    pub charsets: Vec<Charset>,
    pub charset: usize,
    /// Pixel tool without sub-cell mouse precision: paint both halves.
    pub pixel_full_cells: bool,
    /// The pen stroke in progress.
    pub pen: Option<tools::pen::PenStroke>,
    /// Pen position in glyph pixels (8x16 per cell), set by the app per event.
    pub pen_point: (f32, f32),
    /// Brush presets (built-in, then the user's), Tab cycles them.
    pub brushes: Vec<BrushSpec>,
    pub brush_idx: usize,
    /// Names of the brushes saved in the user's library.
    pub user_brushes: Vec<String>,
    /// The pen's brush: the selected preset, as tweaked in the studio.
    pub pen_brush: BrushSpec,
    /// Terminal reports only cells: smooth the pen path (stabilizer) so it
    /// follows the intended curve between cell centers.
    pub pen_smooth: bool,
    pen_avg: Option<(f32, f32)>,
    /// Filters and Recolor: their settings and cached previews.
    pub fx: ColorFx,
    /// Patterns to browse: built-in, then the user's.
    pub patterns: Vec<Pattern>,
    /// Names of the patterns saved in the user's library.
    pub user_patterns: Vec<String>,
    /// The pattern the tool paints with, and where it is in `patterns`
    /// (`None`: taken from a selection and not saved).
    pub pattern: Pattern,
    pub pattern_idx: Option<usize>,
    /// Where tile (0, 0) sits for the stroke in progress.
    pattern_origin: (usize, usize),
}

impl Default for ToolState {
    fn default() -> Self {
        ToolState {
            tool: Tool::Brush,
            prev_tool: Tool::Brush,
            brush: Brush { ch: '█', fg: Color::Pal(15), bg: Color::Pal(0) },
            opts: ToolOpts::default(),
            drag: Drag::None,
            anchor: None,
            floating: None,
            charsets: charsets::builtin(),
            charset: charsets::DEFAULT_SET,
            pixel_full_cells: false,
            pen: None,
            pen_point: (0.0, 0.0),
            brushes: tools::brush::presets(),
            brush_idx: 0,
            user_brushes: vec![],
            pen_brush: BrushSpec::default(),
            pen_smooth: false,
            pen_avg: None,
            fx: ColorFx::default(),
            patterns: pattern::builtin(),
            user_patterns: vec![],
            pattern: pattern::builtin().swap_remove(0),
            pattern_idx: Some(0),
            pattern_origin: (0, 0),
        }
    }
}

impl ToolState {
    pub fn set_tool(&mut self, t: Tool) {
        if t != self.tool {
            self.prev_tool = self.tool;
            self.tool = t;
        }
        self.drag = Drag::None;
        self.anchor = None;
    }

    /// Candidates the pen fits strokes with: the brush's glyph set (for the
    /// tile set: the active set, + blocks when it can't draw lines).
    pub fn pen_candidates(&self) -> Vec<char> {
        self.pen_brush.glyphs.candidates(&self.charsets[self.charset % self.charsets.len()].chars)
    }

    /// The brush as it draws: on a box-drawing tile set the size maps to a
    /// thin line (box glyphs are thin), keeping the same relative scale.
    pub fn pen_spec(&self) -> BrushSpec {
        let mut b = self.pen_brush.clone();
        let set = &self.charsets[self.charset % self.charsets.len()].chars;
        if b.glyphs == GlyphSet::Tiles && set.iter().any(|c| ('\u{2500}'..='\u{257F}').contains(c)) {
            b.size *= tools::pen::RADIUS_BOX / tools::pen::RADIUS_BLOCKS;
        }
        b
    }

    /// Select brush preset `i` (wraps).
    pub fn select_brush(&mut self, i: usize) {
        self.brush_idx = i % self.brushes.len().max(1);
        if let Some(b) = self.brushes.get(self.brush_idx) {
            self.pen_brush = b.clone();
        }
    }

    pub fn cycle_brush(&mut self, dir: i32) -> String {
        let n = self.brushes.len().max(1) as i32;
        self.select_brush((self.brush_idx as i32 + dir).rem_euclid(n) as usize);
        self.option_summary()
    }

    pub fn resize_brush(&mut self, dir: i32) -> String {
        // Bigger brushes take bigger steps.
        let steps = if self.pen_brush.size >= 12.0 {
            4
        } else if self.pen_brush.size >= 6.0 {
            2
        } else {
            1
        };
        tools::brush::Param::Size.nudge(&mut self.pen_brush, dir * steps);
        self.option_summary()
    }

    fn pen_paint(&mut self, tab: &mut Tab, button: Button, start: bool) {
        let spec = self.pen_spec();
        if start {
            self.pen = Some(tools::pen::PenStroke::with_brush(&spec));
        }
        let (mut px, mut py) = self.pen_point;
        // Stabilizer: an exponential moving average of the pen position. The
        // brush's streamline sets it; cell-only mice always get some, so a
        // diagonal drag becomes a straight line at sub-cell precision.
        let mut follow = 1.0 - spec.streamline;
        if self.pen_smooth {
            follow = follow.min(0.35);
        }
        if follow < 1.0 {
            let (ax, ay) = match (start, self.pen_avg) {
                (false, Some((ax, ay))) => (ax + (px - ax) * follow, ay + (py - ay) * follow),
                _ => (px, py),
            };
            self.pen_avg = Some((ax, ay));
            (px, py) = (ax, ay);
        }
        let ctx = self.ctx(tab, button);
        let cands = self.pen_candidates();
        let Some(stroke) = self.pen.as_mut() else { return };
        let cells = stroke.add_point(px, py);
        let stroke = &*stroke;
        tab.edit("Pen", |b| tools::pen::apply(b, &ctx, stroke, cells, &cands));
    }

    /// End a pen stroke: catch the stabilized path up with the pen, then let
    /// the brush finish (the end taper redraws the stroke).
    fn pen_release(&mut self, tab: &mut Tab, button: Button, end: (f32, f32)) {
        if self.pen.is_none() {
            return;
        }
        if self.pen_avg.is_some() {
            self.pen_avg = None;
            let ctx = self.ctx(tab, button);
            let cands = self.pen_candidates();
            if let Some(stroke) = self.pen.as_mut() {
                let cells = stroke.add_point(end.0, end.1);
                let stroke = &*stroke;
                tab.edit("Pen", |b| tools::pen::apply(b, &ctx, stroke, cells, &cands));
            }
        }
        let ctx = self.ctx(tab, button);
        let cands = self.pen_candidates();
        if let Some(mut stroke) = self.pen.take() {
            let cells = stroke.finish();
            if !cells.is_empty() {
                tab.edit("Pen", |b| tools::pen::apply(b, &ctx, &stroke, cells, &cands));
            }
        }
    }

    /// Paint with the current pattern, tile (0, 0) at `origin`. Right-drag
    /// erases the same cells.
    fn pattern_ctx(&self, tab: &Tab, button: Button, origin: (usize, usize)) -> PatternCtx<'_> {
        PatternCtx {
            pattern: &self.pattern,
            layer: tab.layer,
            origin,
            fg: self.brush.fg,
            bg: self.brush.bg,
            recolor: self.opts.pattern_recolor,
            erase: button == Button::Right,
        }
    }

    /// Tile origin for a pattern stroke, rectangle or fill starting at `at`.
    fn pattern_anchor(&self, at: (usize, usize)) -> (usize, usize) {
        match self.opts.pattern_anchor {
            PatternAnchor::Canvas => (0, 0),
            PatternAnchor::Start => at,
        }
    }

    fn pattern_rect(&self, tab: &mut Tab, button: Button, a: (usize, usize), c: (usize, usize)) {
        let r = Rect::from_points(a.0, a.1, c.0, c.1);
        let pc = self.pattern_ctx(tab, button, self.pattern_anchor((r.x, r.y)));
        tab.edit("Pattern", |b| pattern::fill_rect(b, &pc, r));
    }

    /// The patterns a document can use: in Classic, built-ins that need
    /// Modern glyphs are left out (saved ones always show).
    pub fn visible_patterns(&self, classic: bool) -> Vec<usize> {
        let builtin = self.patterns.len() - self.user_patterns.len();
        (0..self.patterns.len()).filter(|&i| !classic || i >= builtin || self.patterns[i].is_classic()).collect()
    }

    /// Paint with pattern `i` of the list.
    pub fn select_pattern(&mut self, i: usize) {
        if let Some(p) = self.patterns.get(i) {
            self.pattern = p.clone();
            self.pattern_idx = Some(i);
        }
    }

    /// Step through the patterns (wraps); an unsaved one steps from the start.
    pub fn cycle_pattern(&mut self, dir: i32, classic: bool) -> String {
        let vis = self.visible_patterns(classic);
        if !vis.is_empty() {
            let n = vis.len() as i32;
            let next = match self.pattern_idx.and_then(|i| vis.iter().position(|&v| v == i)) {
                Some(at) => (at as i32 + dir).rem_euclid(n),
                None if dir > 0 => 0,
                None => n - 1,
            };
            self.select_pattern(vis[next as usize]);
        }
        self.option_summary()
    }

    /// True when the current pattern is one the user saved.
    pub fn pattern_saved(&self) -> bool {
        self.pattern_idx.is_some() && self.user_patterns.contains(&self.pattern.name)
    }

    pub fn resize_pattern(&mut self, dir: i32) -> String {
        let s = self.opts.pattern_size as i32 + dir;
        self.opts.pattern_size = s.clamp(1, PATTERN_SIZE_MAX as i32) as usize;
        format!("size {}", self.opts.pattern_size)
    }

    pub fn glyph(&self, i: usize) -> char {
        self.charsets[self.charset % self.charsets.len()].chars[i % 10]
    }

    /// Paint context for a button: right-click swaps fg/bg (paint with the
    /// background color), like Moebius.
    /// Paint context for a button. Right-drag erases (like tile/pixel
    /// editors) so fixing a mistake never needs a tool switch; with the
    /// shade brush it lightens instead.
    pub fn ctx(&self, tab: &Tab, button: Button) -> Ctx {
        let mode = match (self.tool, button) {
            (Tool::Shade, b) => PaintMode::Shade { up: (b == Button::Left) != self.opts.shade_lighter },
            (_, Button::Right) => PaintMode::Erase,
            (Tool::Colorize, _) => PaintMode::Colorize,
            (Tool::Erase, _) => PaintMode::Erase,
            (Tool::Brush | Tool::Fill, _) => self.opts.brush_mode,
            _ => PaintMode::Char,
        };
        Ctx { layer: tab.layer, brush: self.brush, mode, symmetry: self.opts.symmetry }
    }

    /// Cycle the current tool's main option (Tab / pressing the tool key again).
    pub fn cycle_option(&mut self) -> String {
        match self.tool {
            Tool::Brush | Tool::Fill => {
                self.opts.brush_mode = match self.opts.brush_mode {
                    PaintMode::Char => PaintMode::Color,
                    PaintMode::Color => PaintMode::Fg,
                    PaintMode::Fg => PaintMode::Bg,
                    _ => PaintMode::Char,
                };
                if self.tool == Tool::Fill {
                    self.opts.fill_mode = match self.opts.fill_mode {
                        FillMode::All => FillMode::Char,
                        FillMode::Char => FillMode::Colors,
                        FillMode::Colors => FillMode::Bg,
                        FillMode::Bg => FillMode::All,
                    };
                }
            }
            Tool::Rect => self.opts.rect_fill = toggle_fill(self.opts.rect_fill),
            Tool::Ellipse => self.opts.ellipse_fill = toggle_fill(self.opts.ellipse_fill),
            Tool::Select | Tool::Font | Tool::Stencil => {
                self.opts.stamp_mode = match self.opts.stamp_mode {
                    StampMode::Transparent => StampMode::Opaque,
                    StampMode::Opaque => StampMode::Under,
                    StampMode::Under => StampMode::Transparent,
                }
            }
            Tool::Pixel => self.opts.pixel_fill = !self.opts.pixel_fill,
            Tool::Shade => self.opts.shade_lighter = !self.opts.shade_lighter,
            Tool::Pen => return self.cycle_brush(1),
            Tool::Filters => return self.fx.step_preset(1),
            Tool::Recolor => {
                let r = &mut self.fx.recolor;
                r.target = match r.target {
                    Target::Fg => Target::Bg,
                    Target::Bg => Target::Both,
                    Target::Both => Target::Fg,
                }
            }
            Tool::Gradient => {
                let g = &mut self.opts.gradient;
                let i = gradient::Style::ALL.iter().position(|s| *s == g.style).unwrap_or(0);
                g.style = gradient::Style::ALL[(i + 1) % gradient::Style::ALL.len()];
            }
            _ => {}
        }
        self.option_summary()
    }

    /// Shift-Tab: shape tools step through their looks (the brush glyph
    /// family), anything else the box style used by "Outline selection".
    pub fn cycle_style(&mut self, classic: bool) -> String {
        if self.tool == Tool::Filters {
            return self.fx.step_preset(-1);
        }
        if self.tool.is_shape() {
            let looks: Vec<&Look> = LOOKS.iter().filter(|l| !(classic && l.modern_only)).collect();
            let cur = looks.iter().position(|l| l.active(self.brush.ch)).unwrap_or(0);
            let next = looks[(cur + 1) % looks.len()];
            self.brush.ch = next.glyph;
            return self.option_summary();
        }
        self.opts.box_style = match self.opts.box_style {
            BoxStyle::Single => BoxStyle::Double,
            BoxStyle::Double => BoxStyle::DoubleH,
            BoxStyle::DoubleH => BoxStyle::DoubleV,
            BoxStyle::DoubleV => BoxStyle::Block,
            BoxStyle::Block => BoxStyle::Brush,
            BoxStyle::Brush => BoxStyle::Rounded,
            BoxStyle::Rounded => BoxStyle::Single,
        };
        box_style_name(self.opts.box_style).to_string()
    }

    pub fn cycle_mirror(&mut self) -> String {
        self.opts.symmetry = match self.opts.symmetry {
            Symmetry::None => Symmetry::X,
            Symmetry::X => Symmetry::Y,
            Symmetry::Y => Symmetry::Both,
            Symmetry::Both => Symmetry::None,
        };
        format!("mirror: {}", symmetry_name(self.opts.symmetry))
    }

    /// Short human summary of the active tool's options (status bar).
    pub fn option_summary(&self) -> String {
        let mode = paint_mode_name(self.opts.brush_mode);
        match self.tool {
            Tool::Brush => mode.to_string(),
            Tool::Rect => format!("{} · {}", fill_name(self.opts.rect_fill), look_of(self.brush.ch).name),
            Tool::Ellipse => format!("{} · {}", fill_name(self.opts.ellipse_fill), look_of(self.brush.ch).name),
            Tool::Line => look_of(self.brush.ch).name.to_string(),
            Tool::Shade => if self.opts.shade_lighter { "click lightens" } else { "click darkens" }.into(),
            Tool::Fill => format!("{} · {}", self.opts.fill_mode.name(), mode),
            Tool::Select | Tool::Font | Tool::Stencil => stamp_name(self.opts.stamp_mode).to_string(),
            Tool::Pen => format!("{} {:.1}px", self.pen_brush.name, self.pen_brush.size),
            Tool::Filters => self.fx.filter.filter.current().name.to_string(),
            Tool::Recolor => {
                let r = &self.fx.recolor;
                format!("{} · tolerance {}", r.target.name(), r.tolerance)
            }
            Tool::Pattern => {
                let size = match self.opts.pattern_mode {
                    PatternMode::Brush => format!(" {}", self.opts.pattern_size),
                    _ => String::new(),
                };
                format!("{} · {}{size}", self.pattern.name, self.opts.pattern_mode.name())
            }
            Tool::Gradient => {
                let g = &self.opts.gradient;
                let shape = if g.shape == gradient::Shape::Radial { "radial" } else { "linear" };
                format!("{shape} · {} · {}{}", g.style.name(), g.ramp.name(), if g.reverse { " ⇄" } else { "" })
            }
            Tool::Pixel => {
                if self.opts.pixel_fill {
                    "fill".into()
                } else {
                    "pen".into()
                }
            }
            _ => String::new(),
        }
    }

    // ------------------------------------------------------------ mouse

    /// Mouse press at canvas cell (x, y). Pixel tool receives pixel coords.
    pub fn press(&mut self, tab: &mut Tab, x: usize, y: usize, button: Button) -> Option<String> {
        if let Some(f) = &mut self.floating {
            f.x = x;
            f.y = y;
            return self.stamp_floating(tab);
        }
        match self.tool {
            Tool::Brush | Tool::Shade | Tool::Colorize | Tool::Erase | Tool::Art => {
                tab.history.begin_group();
                let ctx = self.ctx(tab, button);
                tab.edit(self.tool.name(), |b| tools::paint(b, &ctx, x, y));
                self.drag = Drag::Stroke { last: (x, y), button };
            }
            Tool::Pen => {
                tab.history.begin_group();
                self.pen_paint(tab, button, true);
                self.drag = Drag::Stroke { last: (x, y), button };
            }
            Tool::Pixel => {
                let color = if button == Button::Left { self.brush.fg } else { Color::BLACK };
                let layer = tab.layer;
                if self.opts.pixel_fill {
                    tab.edit("Pixel fill", |b| tools::pixel::flood_fill(b, layer, x, y, color));
                } else {
                    tab.history.begin_group();
                    let full = self.pixel_full_cells;
                    tab.edit("Pixel", |b| {
                        tools::pixel::set(b, layer, x, y, color);
                        if full {
                            tools::pixel::set(b, layer, x, y | 1, color);
                        }
                    });
                    self.drag = Drag::Stroke { last: (x, y), button };
                }
            }
            Tool::Line | Tool::Rect | Tool::Ellipse => self.drag = Drag::Shape { start: (x, y), cur: (x, y), button },
            Tool::Pattern => {
                self.pattern_origin = self.pattern_anchor((x, y));
                match self.opts.pattern_mode {
                    PatternMode::Brush => {
                        tab.history.begin_group();
                        let pc = self.pattern_ctx(tab, button, self.pattern_origin);
                        let size = self.opts.pattern_size;
                        tab.edit("Pattern", |b| pattern::dab(b, &pc, x, y, size));
                        self.drag = Drag::Stroke { last: (x, y), button };
                    }
                    PatternMode::Rect => self.drag = Drag::Shape { start: (x, y), cur: (x, y), button },
                    PatternMode::Fill => {
                        let pc = self.pattern_ctx(tab, button, self.pattern_origin);
                        tab.edit("Pattern fill", |b| pattern::flood_fill(b, &pc, x, y, FillMatch::default()));
                    }
                }
            }
            Tool::Fill => {
                let ctx = self.ctx(tab, button);
                let m = self.opts.fill_mode.matcher();
                tab.edit("Fill", |b| tools::flood_fill(b, &ctx, x, y, m));
            }
            Tool::Gradient => {
                let mask = Rc::new(self.gradient_mask(tab, x, y));
                if !mask.iter().any(|m| *m) {
                    return Some("nothing to fill here".into());
                }
                self.drag = Drag::Gradient { start: (x, y), cur: (x, y), button, mask };
            }
            Tool::Picker => {
                let c = tab.doc.canvas.composite(x, y);
                self.brush = Brush { ch: c.ch, fg: c.fg, bg: c.bg };
                let back = self.prev_tool;
                self.set_tool(back);
                return Some(format!("picked '{}'", c.ch));
            }
            Tool::Text => {
                tab.cursor = (x, y);
                tab.text_home_x = x;
            }
            Tool::Select => {
                if let Some(sel) = tab.selection.filter(|s| s.contains(x, y)) {
                    // Lift the selection into a floating clip and drag it.
                    let layer = tab.layer;
                    let clip = copy_clip(tab, Some(layer), sel);
                    tab.history.begin_group();
                    tab.edit("Move", |b| tools::erase(b, layer, sel));
                    self.floating = Some(Floating { clip, x: sel.x, y: sel.y, source: FloatSource::Move, lift: None });
                    self.drag = Drag::Moving { grab: (x - sel.x, y - sel.y), from: (sel.x, sel.y) };
                    tab.selection = None;
                } else {
                    tab.selection = None;
                    self.drag = Drag::Selecting { start: (x, y), cur: (x, y) };
                }
            }
            Tool::Filters => {
                // Hold to see the original.
                self.fx.comparing = true;
                return Some("showing the original — let go to see the filter".into());
            }
            Tool::Recolor => return Some(self.pick_from(tab, x, y)),
            Tool::Font | Tool::Stencil => {}
        }
        None
    }

    /// Recolor: take the color to replace from the canvas. With "both", the
    /// glyph pixel under the pointer decides: ink picks the fg, else the bg.
    pub fn pick_from(&mut self, tab: &Tab, x: usize, y: usize) -> String {
        let c = tab.doc.canvas.composite(x, y);
        let (px, py) = self.pen_point;
        let (gx, gy) = ((px - x as f32 * 8.0).clamp(0.0, 7.0) as u32, (py - y as f32 * 16.0).clamp(0.0, 15.0) as u32);
        let fg = match self.fx.recolor.target {
            Target::Fg => true,
            Target::Bg => false,
            Target::Both => tools::has_ink(c.ch) && acidtrip_core::render::glyph_pixel(c.ch, gx, gy),
        };
        let col = if fg { c.fg } else { c.bg };
        self.fx.recolor.from = Some(col);
        format!("replace {} — the brush FG is the new color, Enter applies", crate::ui::sidebar::color_label(col))
    }

    /// Filters / Recolor: which one is active.
    pub fn fx_kind(&self) -> Option<Kind> {
        match self.tool {
            Tool::Filters => Some(Kind::Filter),
            Tool::Recolor => Some(Kind::Recolor),
            _ => None,
        }
    }

    /// Apply the active Filters / Recolor tool as one undo step.
    pub fn apply_fx(&mut self, tab: &mut Tab) -> Option<String> {
        let kind = self.fx_kind()?;
        Some(self.fx.apply(tab, kind, self.brush.fg))
    }

    pub fn drag_to(&mut self, tab: &mut Tab, x: usize, y: usize) {
        match self.drag.clone() {
            Drag::Stroke { last, button } if self.tool == Tool::Pen => {
                self.pen_paint(tab, button, false);
                self.drag = Drag::Stroke { last, button };
            }
            Drag::Stroke { last, button } if self.tool == Tool::Pattern => {
                if (x, y) != last {
                    let pc = self.pattern_ctx(tab, button, self.pattern_origin);
                    let size = self.opts.pattern_size;
                    tab.edit("Pattern", |b| pattern::stroke(b, &pc, last, (x, y), size));
                }
                self.drag = Drag::Stroke { last: (x, y), button };
            }
            Drag::Stroke { last, button } => {
                if self.tool == Tool::Pixel {
                    let color = if button == Button::Left { self.brush.fg } else { Color::BLACK };
                    let layer = tab.layer;
                    let full = self.pixel_full_cells;
                    tab.edit("Pixel", |b| {
                        tools::pixel::line(b, layer, last.0 as i64, last.1 as i64, x as i64, y as i64, color);
                        if full {
                            tools::pixel::line(
                                b,
                                layer,
                                last.0 as i64,
                                (last.1 | 1) as i64,
                                x as i64,
                                (y | 1) as i64,
                                color,
                            );
                        }
                    });
                } else if (x, y) != last {
                    // Skip the segment's first point: it was painted by the previous event
                    // (shading would otherwise step it twice).
                    let ctx = self.ctx(tab, button);
                    let name = self.tool.name();
                    let pts: Vec<(i64, i64)> =
                        line_drawing::Midpoint::<f64, i64>::new((last.0 as f64, last.1 as f64), (x as f64, y as f64))
                            .skip(1)
                            .collect();
                    tab.edit(name, |b| {
                        for (px, py) in pts {
                            tools::paint(b, &ctx, px.max(0) as usize, py.max(0) as usize);
                        }
                    });
                }
                self.drag = Drag::Stroke { last: (x, y), button };
            }
            Drag::Shape { start, button, .. } => self.drag = Drag::Shape { start, cur: (x, y), button },
            Drag::Selecting { start, .. } => {
                self.drag = Drag::Selecting { start, cur: (x, y) };
                tab.selection = Some(Rect::from_points(start.0, start.1, x, y));
            }
            Drag::Moving { grab, .. } => {
                if let Some(f) = &mut self.floating {
                    f.x = x.saturating_sub(grab.0);
                    f.y = y.saturating_sub(grab.1);
                }
            }
            Drag::Gradient { start, button, mask, .. } => {
                self.drag = Drag::Gradient { start, cur: (x, y), button, mask }
            }
            Drag::None => {}
        }
    }

    pub fn release(&mut self, tab: &mut Tab, x: usize, y: usize) -> Option<String> {
        self.fx.comparing = false;
        let drag = std::mem::take(&mut self.drag);
        match drag {
            Drag::Stroke { button, .. } => {
                if self.tool == Tool::Pen {
                    // Cell-only mice end at the cell center, pixel mice where they are.
                    let end = if self.pen_smooth { tools::pen::cell_center(x, y) } else { self.pen_point };
                    self.pen_release(tab, button, end);
                }
                self.pen = None;
                self.pen_avg = None;
                tab.history.end_group()
            }
            Drag::Shape { start, button, .. } if self.tool == Tool::Pattern => {
                self.pattern_rect(tab, button, start, (x, y));
            }
            Drag::Shape { start, button, .. } => {
                let ctx = self.ctx(tab, button);
                let tool = self.tool;
                let opts = self.opts.clone();
                tab.edit(tool.name(), |b| draw_shape(b, tool, &opts, &ctx, start, (x, y)));
            }
            Drag::Selecting { start, .. } => {
                let r = Rect::from_points(start.0, start.1, x, y);
                tab.selection = Some(r);
                return Some(format!("selected {}x{} — Enter: block menu, Ctrl-C copy, drag to move", r.w, r.h));
            }
            Drag::Moving { grab, .. } => {
                if let Some(f) = &mut self.floating {
                    f.x = x.saturating_sub(grab.0);
                    f.y = y.saturating_sub(grab.1);
                }
                let msg = self.stamp_floating(tab);
                tab.history.end_group();
                return msg;
            }
            Drag::Gradient { start, button, mask, .. } => {
                let x = x.min(tab.doc.width().saturating_sub(1));
                let y = y.min(tab.doc.height().saturating_sub(1));
                let (layer, opts, stops) = (tab.layer, self.opts.gradient, self.gradient_stops(button));
                tab.edit("Gradient", |b| gradient::fill(b, layer, &mask, (start, (x, y)), &opts, &stops));
            }
            Drag::None => {}
        }
        None
    }

    /// A key came in while the mouse button is still down (a tool key, undo,
    /// Esc…): end the drag there, so its undo step closes and the next edit
    /// can't merge into it. Strokes keep what they painted and shapes,
    /// gradients and marquees are dropped. A moved block is put down where
    /// it is, or back where it came from when `cancel` (Esc).
    pub fn end_drag(&mut self, tab: &mut Tab, cancel: bool) -> Option<String> {
        self.fx.comparing = false;
        match std::mem::take(&mut self.drag) {
            Drag::Stroke { button, .. } => {
                if self.tool == Tool::Pen {
                    self.pen_release(tab, button, self.pen_point);
                }
                self.pen = None;
                self.pen_avg = None;
                tab.history.end_group();
                None
            }
            Drag::Moving { from, .. } => {
                if cancel && let Some(f) = &mut self.floating {
                    (f.x, f.y) = from;
                }
                let msg = self.stamp_floating(tab);
                tab.history.end_group();
                msg
            }
            Drag::Shape { .. } | Drag::Gradient { .. } => {
                if cancel {
                    return Some(format!("{} cancelled", self.tool.name()));
                }
                None
            }
            Drag::Selecting { .. } | Drag::None => None,
        }
    }

    /// Hover (no button): floating clips follow the mouse.
    pub fn hover(&mut self, x: usize, y: usize) {
        if let Some(f) = &mut self.floating {
            f.x = x;
            f.y = y;
        }
    }

    // --------------------------------------------------------- keyboard

    /// Space: apply the tool at the keyboard cursor.
    pub fn apply_at_cursor(&mut self, tab: &mut Tab) -> Option<String> {
        let (x, y) = tab.cursor;
        if self.floating.is_some() {
            if let Some(f) = &mut self.floating {
                f.x = x;
                f.y = y;
            }
            return self.stamp_floating(tab);
        }
        if self.two_point() || matches!(self.tool, Tool::Select | Tool::Gradient) {
            match self.anchor.take() {
                None => {
                    self.anchor = Some((x, y));
                    return Some("anchor set — move the cursor and press Space again".into());
                }
                Some(a) => {
                    if self.tool == Tool::Select {
                        let r = Rect::from_points(a.0, a.1, x, y);
                        tab.selection = Some(r);
                        return Some(format!("selected {}x{} — Enter: block menu, Ctrl-C copy", r.w, r.h));
                    }
                    if self.tool == Tool::Pattern {
                        self.pattern_rect(tab, Button::Left, a, (x, y));
                        return None;
                    }
                    if self.tool == Tool::Gradient {
                        let r = self.press(tab, a.0, a.1, Button::Left);
                        return r.or_else(|| self.release(tab, x, y));
                    }
                    let ctx = self.ctx(tab, Button::Left);
                    let tool = self.tool;
                    let opts = self.opts.clone();
                    tab.edit(tool.name(), |b| draw_shape(b, tool, &opts, &ctx, a, (x, y)));
                    return Some(format!("{} drawn — Space starts another", tool.name()));
                }
            }
        }
        match self.tool {
            Tool::Filters => return self.apply_fx(tab),
            Tool::Recolor => {
                self.pen_point = tools::pen::cell_center(x, y);
                return Some(self.pick_from(tab, x, y));
            }
            _ => {}
        }
        if self.tool == Tool::Pixel {
            let color = self.brush.fg;
            let layer = tab.layer;
            tab.edit("Pixel", |b| {
                tools::pixel::set(b, layer, x, y * 2, color);
                tools::pixel::set(b, layer, x, y * 2 + 1, color);
            });
            return None;
        }
        let r = self.press(tab, x, y, Button::Left);
        let r2 = self.release(tab, x, y);
        r.or(r2)
    }

    /// Put a char into the brush (tool mode) — charset keys, char picker.
    pub fn set_brush_char(&mut self, ch: char) {
        self.brush.ch = ch;
        if self.opts.brush_mode != PaintMode::Char && matches!(self.tool, Tool::Brush | Tool::Fill) {
            self.opts.brush_mode = PaintMode::Char;
        }
    }

    // ------------------------------------------------------- floating

    pub fn stamp_floating(&mut self, tab: &mut Tab) -> Option<String> {
        let f = self.floating.clone()?;
        if let Some(lift) = f.lift {
            // A carried block: leaving its spot and landing are one step (none
            // at all when it lands where it was). Another document only gets a copy.
            let from = Some(lift).filter(|l| l.doc == tab.doc.meta.id && l.layer < tab.doc.canvas.layers.len());
            let layer = from.map_or(tab.layer, |l| l.layer);
            tab.edit("Move", |b| {
                if let Some(l) = from {
                    tools::erase(b, l.layer, l.rect);
                }
                tools::stamp(b, layer, &f.clip, f.x, f.y, StampMode::Opaque);
            });
            tab.selection = Some(Rect::new(f.x, f.y, f.clip.width, f.clip.height));
            self.floating = None;
            let (w, h) = (f.clip.width, f.clip.height);
            if from.is_some_and(|l| (l.rect.x, l.rect.y) == (f.x, f.y)) {
                return Some(format!("the {w}x{h} block is back where it was"));
            }
            return Some(format!("moved the {w}x{h} block (Ctrl-Z to undo)"));
        }
        let layer = tab.layer;
        let mode = if f.source == FloatSource::Move { StampMode::Opaque } else { self.opts.stamp_mode };
        tab.edit("Stamp", |b| tools::stamp(b, layer, &f.clip, f.x, f.y, mode));
        if f.source == FloatSource::Move {
            tab.selection = Some(Rect::new(f.x, f.y, f.clip.width, f.clip.height));
            self.floating = None;
            None
        } else {
            Some("stamped — click/Space to stamp again, Esc when done".into())
        }
    }

    // --------------------------------------------------------- preview

    /// Cells to overlay on the canvas (shape preview, floating clip).
    pub fn preview(&self, tab: &Tab) -> Vec<(usize, usize, Cell)> {
        if let Some(kind) = self.fx_kind() {
            return self.fx.preview(tab, kind, self.brush.fg);
        }
        let mut out = vec![];
        if let Some(f) = &self.floating {
            // A carried block's spot shows as it will be once the block is gone.
            if let Some(l) = f.lift.filter(|l| l.doc == tab.doc.meta.id) {
                let c = &tab.doc.canvas;
                for y in l.rect.y..(l.rect.y + l.rect.h).min(c.height) {
                    for x in l.rect.x..(l.rect.x + l.rect.w).min(c.width) {
                        let i = y * c.width + x;
                        let under = c
                            .layers
                            .iter()
                            .enumerate()
                            .rev()
                            .filter(|(li, layer)| *li != l.layer && layer.visible && layer.kind == LayerKind::Normal)
                            .find_map(|(_, layer)| layer.cells[i])
                            .unwrap_or(Cell::BLANK);
                        out.push((x, y, under));
                    }
                }
            }
            // Show what a stamp would do: in clear and under modes blank
            // cells are see-through, and under skips cells with art.
            let mode = if f.source == FloatSource::Move { StampMode::Opaque } else { self.opts.stamp_mode };
            for cy in 0..f.clip.height {
                for cx in 0..f.clip.width {
                    let (x, y) = (f.x + cx, f.y + cy);
                    let Some(c) = f.clip.get(cx, cy) else { continue };
                    let hidden = match mode {
                        StampMode::Opaque => false,
                        StampMode::Transparent => c.is_blank(),
                        StampMode::Under => c.is_blank() || !tab.doc.canvas.composite(x, y).is_blank(),
                    };
                    if !hidden {
                        out.push((x, y, c));
                    }
                }
            }
            return out;
        }
        if let Some((start, cur, button, mask)) = match (&self.drag, self.anchor) {
            (Drag::Gradient { start, cur, button, mask }, _) => Some((*start, *cur, *button, mask.clone())),
            (_, Some(a)) if self.tool == Tool::Gradient => {
                Some((a, tab.cursor, Button::Left, Rc::new(self.gradient_mask(tab, a.0, a.1))))
            }
            _ => None,
        } {
            let mut b = TxBuilder::new(&tab.doc, "preview");
            let cur = (cur.0.min(tab.doc.width().saturating_sub(1)), cur.1.min(tab.doc.height().saturating_sub(1)));
            let stops = self.gradient_stops(button);
            gradient::fill(&mut b, tab.layer, &mask, (start, cur), &self.opts.gradient, &stops);
            return b.finish().cells.into_iter().filter_map(|c| c.after.map(|cell| (c.x, c.y, cell))).collect();
        }
        let (start, cur, button) = match (&self.drag, self.anchor) {
            (Drag::Shape { start, cur, button }, _) => (*start, *cur, *button),
            (_, Some(a)) if self.two_point() => (a, tab.cursor, Button::Left),
            _ => return out,
        };
        let mut b = TxBuilder::new(&tab.doc, "preview");
        if self.tool == Tool::Pattern {
            let r = Rect::from_points(start.0, start.1, cur.0, cur.1);
            let pc = self.pattern_ctx(tab, button, self.pattern_anchor((r.x, r.y)));
            pattern::fill_rect(&mut b, &pc, r);
        } else {
            let ctx = self.ctx(tab, button);
            draw_shape(&mut b, self.tool, &self.opts, &ctx, start, cur);
        }
        let tx = b.finish();
        for c in tx.cells {
            if let Some(cell) = c.after {
                out.push((c.x, c.y, cell));
            }
        }
        out
    }

    /// Tools the keyboard draws with two Spaces (anchor, then the far corner).
    fn two_point(&self) -> bool {
        self.tool.is_shape() || (self.tool == Tool::Pattern && self.opts.pattern_mode == PatternMode::Rect)
    }

    /// The cells a gradient from (x, y) covers: the selection when there is
    /// one, else the area the Fill tool would fill (by its match setting).
    pub fn gradient_mask(&self, tab: &Tab, x: usize, y: usize) -> Vec<bool> {
        let (w, h) = (tab.doc.width(), tab.doc.height());
        match tab.selection {
            Some(r) => (0..w * h).map(|i| r.contains(i % w, i / w)).collect(),
            None => tools::fill_region(&TxBuilder::new(&tab.doc, "region"), x, y, self.opts.fill_mode.matcher()),
        }
    }

    /// The gradient's colors; a right-drag runs them the other way.
    pub fn gradient_stops(&self, button: Button) -> Vec<Color> {
        let mut s = self.opts.gradient.stops(self.brush.fg, self.brush.bg);
        if button == Button::Right {
            s.reverse();
        }
        s
    }

    /// Selection currently being dragged or keyboard-anchored (for the marquee).
    pub fn pending_selection(&self, tab: &Tab) -> Option<Rect> {
        match (&self.drag, self.anchor) {
            (Drag::Selecting { start, cur }, _) => Some(Rect::from_points(start.0, start.1, cur.0, cur.1)),
            (_, Some(a)) if self.tool == Tool::Select => Some(Rect::from_points(a.0, a.1, tab.cursor.0, tab.cursor.1)),
            _ => None,
        }
    }
}

fn draw_shape(b: &mut TxBuilder, tool: Tool, opts: &ToolOpts, ctx: &Ctx, a: (usize, usize), c: (usize, usize)) {
    let r = Rect::from_points(a.0, a.1, c.0, c.1);
    match tool {
        // Shapes take their look from the brush glyph: solid blocks draw in
        // half-block pixels, box-drawing chars pick the matching frame style.
        Tool::Line => tools::smart_line(b, ctx, a.0, a.1, c.0, c.1),
        Tool::Rect => tools::smart_rect(b, ctx, r, opts.rect_fill),
        Tool::Ellipse => tools::smart_ellipse(b, ctx, r, opts.ellipse_fill),
        _ => {}
    }
}

pub fn copy_clip(tab: &Tab, layer: Option<usize>, r: Rect) -> Clip {
    let b = TxBuilder::new(&tab.doc, "copy");
    tools::copy(&b, layer, r)
}

fn toggle_fill(f: ShapeFill) -> ShapeFill {
    match f {
        ShapeFill::Outline => ShapeFill::Filled,
        ShapeFill::Filled => ShapeFill::Outline,
    }
}

pub fn paint_mode_name(m: PaintMode) -> &'static str {
    match m {
        PaintMode::Char => "char+colors",
        PaintMode::Color => "colors only",
        PaintMode::Fg => "fg only",
        PaintMode::Bg => "bg only",
        PaintMode::Shade { .. } => "shade",
        PaintMode::Colorize => "colorize",
        PaintMode::Erase => "erase",
    }
}

/// How shape tools render, chosen by the brush glyph (see
/// [`tools::shape_style`]). The sidebar shows these as chips.
pub struct Look {
    pub glyph: char,
    pub name: &'static str,
    /// Not in CP437: hidden for Classic documents.
    pub modern_only: bool,
}

impl Look {
    pub fn active(&self, brush: char) -> bool {
        tools::shape_style(brush) == tools::shape_style(self.glyph)
    }
}

pub const LOOKS: [Look; 7] = [
    Look { glyph: '█', name: "pixels", modern_only: false },
    Look { glyph: '┌', name: "single frame", modern_only: false },
    Look { glyph: '╔', name: "double frame", modern_only: false },
    Look { glyph: '╒', name: "double-h frame", modern_only: false },
    Look { glyph: '╓', name: "double-v frame", modern_only: false },
    Look { glyph: '╭', name: "rounded frame", modern_only: true },
    Look { glyph: '▒', name: "brush glyph", modern_only: false },
];

pub fn look_of(brush: char) -> &'static Look {
    LOOKS.iter().find(|l| l.active(brush)).unwrap_or(&LOOKS[6])
}

pub fn fill_name(f: ShapeFill) -> &'static str {
    match f {
        ShapeFill::Outline => "outline",
        ShapeFill::Filled => "filled",
    }
}

pub fn box_style_name(s: BoxStyle) -> &'static str {
    match s {
        BoxStyle::Single => "┌─┐ single",
        BoxStyle::Double => "╔═╗ double",
        BoxStyle::DoubleH => "╒═╕ double-h",
        BoxStyle::DoubleV => "╓─╖ double-v",
        BoxStyle::Block => "███ block",
        BoxStyle::Brush => "brush char",
        BoxStyle::Rounded => "╭─╮ rounded",
    }
}

pub fn stamp_name(s: StampMode) -> &'static str {
    match s {
        StampMode::Transparent => "transparent",
        StampMode::Opaque => "opaque",
        StampMode::Under => "under",
    }
}

pub fn symmetry_name(s: Symmetry) -> &'static str {
    match s {
        Symmetry::None => "off",
        Symmetry::X => "left↔right",
        Symmetry::Y => "top↕bottom",
        Symmetry::Both => "both",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acidtrip_core::{DocKind, Document};

    fn setup(rows: &[&str]) -> (ToolState, Tab) {
        let mut ts = ToolState::default();
        ts.set_tool(Tool::Pattern);
        ts.pattern = Pattern::from_rows("t", rows);
        ts.pattern_idx = None;
        (ts, Tab::new(Document::new(DocKind::Classic, 20, 10), None))
    }

    fn ch(tab: &Tab, x: usize, y: usize) -> char {
        tab.doc.canvas.composite(x, y).ch
    }

    #[test]
    fn a_pattern_stroke_is_one_undo_step() {
        let (mut ts, mut tab) = setup(&["ab", "cd"]);
        ts.opts.pattern_size = 1;
        ts.press(&mut tab, 2, 2, Button::Left);
        ts.drag_to(&mut tab, 6, 2);
        ts.release(&mut tab, 6, 2);
        assert_eq!((2..=6).map(|x| ch(&tab, x, 2)).collect::<String>(), "ababa");
        assert_eq!(tab.history.len(), 1);
        tab.undo();
        assert!(tab.doc.canvas.composite(4, 2).is_blank());
    }

    #[test]
    fn pattern_rect_and_fill() {
        let (mut ts, mut tab) = setup(&["xy"]);
        ts.opts.pattern_mode = PatternMode::Rect;
        ts.press(&mut tab, 1, 1, Button::Left);
        ts.drag_to(&mut tab, 4, 3);
        ts.release(&mut tab, 4, 3);
        assert_eq!((1..=4).map(|x| ch(&tab, x, 3)).collect::<String>(), "yxyx", "lines up with the canvas");
        assert!(tab.doc.canvas.composite(5, 3).is_blank());
        ts.opts.pattern_anchor = PatternAnchor::Start;
        ts.opts.pattern_mode = PatternMode::Fill;
        ts.press(&mut tab, 9, 8, Button::Left);
        assert_eq!(ch(&tab, 9, 8), 'x', "the fill starts its tile at the press");
        assert_eq!(ch(&tab, 10, 8), 'y');
        assert_eq!(ch(&tab, 2, 2), 'x', "the rect is left alone");
        assert_eq!(tab.history.len(), 2);
    }

    #[test]
    fn browsing_patterns() {
        let mut ts = ToolState::default();
        assert!(ts.visible_patterns(true).len() < ts.patterns.len());
        for _ in 0..ts.patterns.len() {
            ts.cycle_pattern(1, true);
            assert!(ts.pattern.is_classic(), "{}", ts.pattern.name);
        }
        ts.pattern_idx = None;
        ts.cycle_pattern(1, false);
        assert_eq!(ts.pattern_idx, Some(0), "an unsaved pattern steps from the start");
        assert_eq!(ts.resize_pattern(99), format!("size {PATTERN_SIZE_MAX}"));
    }

    #[test]
    fn the_float_preview_matches_the_stamp_mode() {
        use acidtrip_core::Color;
        let (mut ts, mut tab) = setup(&["x"]);
        let art = |c| Some(Cell::new(c, Color::WHITE, Color::BLACK));
        tab.doc.canvas.layers[0].cells[2 * 20 + 1] = art('#');
        let mut clip = Clip::new(3, 1);
        clip.set(0, 0, art('A'));
        clip.set(1, 0, art(' '));
        clip.set(2, 0, art('B'));
        ts.floating = Some(Floating { clip, x: 0, y: 2, source: FloatSource::Paste, lift: None });
        let shown = |ts: &ToolState, tab: &Tab| ts.preview(tab).iter().map(|&(x, _, c)| (x, c.ch)).collect::<Vec<_>>();
        ts.opts.stamp_mode = StampMode::Transparent;
        assert_eq!(shown(&ts, &tab), [(0, 'A'), (2, 'B')], "a blank lets the art show through");
        ts.opts.stamp_mode = StampMode::Opaque;
        assert_eq!(shown(&ts, &tab), [(0, 'A'), (1, ' '), (2, 'B')]);
        tab.doc.canvas.layers[0].cells[2 * 20 + 2] = art('#');
        ts.opts.stamp_mode = StampMode::Under;
        assert_eq!(shown(&ts, &tab), [(0, 'A')], "under skips cells that have art");
        ts.floating.as_mut().unwrap().source = FloatSource::Move;
        assert_eq!(shown(&ts, &tab).len(), 3, "a move always lands opaque");
    }

    /// A tab with "AB" at (1,1) and one undone edit waiting to be redone.
    fn art_with_redo() -> (ToolState, Tab) {
        let (mut ts, mut tab) = setup(&["x"]);
        ts.set_tool(Tool::Select);
        let art = |c| Some(Cell::new(c, Color::WHITE, Color::BLACK));
        tab.edit("Paint", |b| {
            b.set(0, 1, 1, art('A'));
            b.set(0, 2, 1, art('B'));
        });
        tab.edit("Paint", |b| b.set(0, 9, 9, art('Z')));
        tab.undo();
        assert!(tab.history.can_redo());
        (ts, tab)
    }

    fn carry(ts: &mut ToolState, tab: &Tab, rect: Rect) {
        let clip = copy_clip(tab, Some(0), rect);
        let lift = Some(Lift { doc: tab.doc.meta.id, layer: 0, rect });
        ts.floating = Some(Floating { clip, x: rect.x, y: rect.y, source: FloatSource::Move, lift });
    }

    #[test]
    fn a_carried_block_moves_in_one_step() {
        let (mut ts, mut tab) = art_with_redo();
        let steps = tab.history.len();
        carry(&mut ts, &tab, Rect::new(1, 1, 2, 1));
        assert_eq!(tab.history.len(), steps, "picking it up changes nothing");
        assert_eq!(ch(&tab, 1, 1), 'A');
        let msg = ts.press(&mut tab, 5, 3, Button::Left);
        assert_eq!(msg.as_deref(), Some("moved the 2x1 block (Ctrl-Z to undo)"));
        assert_eq!((ch(&tab, 5, 3), ch(&tab, 6, 3)), ('A', 'B'));
        assert!(tab.doc.canvas.composite(1, 1).is_blank());
        assert_eq!(tab.history.len(), steps + 1);
        assert_eq!(tab.undo().as_deref(), Some("Move"));
        assert_eq!((ch(&tab, 1, 1), ch(&tab, 2, 1)), ('A', 'B'), "one undo puts it back");
        assert!(tab.doc.canvas.composite(5, 3).is_blank());
    }

    #[test]
    fn moves_that_change_nothing_keep_redo() {
        // A carried block put down where it was.
        let (mut ts, mut tab) = art_with_redo();
        let steps = tab.history.len();
        carry(&mut ts, &tab, Rect::new(1, 1, 2, 1));
        let msg = ts.press(&mut tab, 1, 1, Button::Left);
        assert_eq!(msg.as_deref(), Some("the 2x1 block is back where it was"));
        assert_eq!(tab.history.len(), steps);
        assert!(tab.history.can_redo(), "redo survives");
        // A dragged selection let go where it started.
        let (mut ts, mut tab) = art_with_redo();
        tab.selection = Some(Rect::new(1, 1, 2, 1));
        ts.press(&mut tab, 1, 1, Button::Left);
        ts.drag_to(&mut tab, 3, 2);
        ts.drag_to(&mut tab, 1, 1);
        ts.release(&mut tab, 1, 1);
        assert_eq!((ch(&tab, 1, 1), ch(&tab, 2, 1)), ('A', 'B'));
        assert_eq!(tab.history.len(), steps);
        assert_eq!(tab.history.redo_label(), Some("Paint"), "redo survives");
        // A real move still drops redo.
        tab.selection = Some(Rect::new(1, 1, 2, 1));
        ts.press(&mut tab, 1, 1, Button::Left);
        ts.release(&mut tab, 4, 4);
        assert_eq!(tab.history.len(), steps + 1);
        assert!(!tab.history.can_redo());
    }
}
