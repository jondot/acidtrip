//! Every user-invokable command. Keymaps, the command palette, the sidebar
//! and the help screen are all driven from this one table.

macro_rules! actions {
    ($( $v:ident => $id:literal, $title:literal, $cat:literal; )*) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Action { $($v),* }

        impl Action {
            pub const ALL: &'static [Action] = &[$(Action::$v),*];

            /// Stable id used in config.toml keymap overrides.
            pub fn id(self) -> &'static str {
                match self { $(Action::$v => $id),* }
            }

            pub fn title(self) -> &'static str {
                match self { $(Action::$v => $title),* }
            }

            pub fn category(self) -> &'static str {
                match self { $(Action::$v => $cat),* }
            }

            pub fn from_id(id: &str) -> Option<Action> {
                match id { $($id => Some(Action::$v),)* _ => None }
            }
        }
    };
}

actions! {
    // File
    New => "new", "New document…", "File";
    Open => "open", "Open…", "File";
    Save => "save", "Save", "File";
    SaveAs => "save_as", "Save as / Export…", "File";
    Export => "export", "Export panel (formats, sizes, names)", "File";
    ExportNow => "export_now", "Export: write every row now", "File";
    ExportAs => "export_as", "Export as… (one file)", "File";
    Share => "share", "Share…", "File";
    Quit => "quit", "Quit", "File";
    Versions => "versions", "Version history…", "File";
    SnapshotVersion => "snapshot", "Save a named version…", "File";
    ImportImage => "import_image", "Import image…", "File";
    ReferenceImage => "reference_image", "Add reference image layer…", "File";
    Gallery => "gallery", "Gallery: browse ANSI art…", "File";
    // Edit
    Undo => "undo", "Undo", "Edit";
    Redo => "redo", "Redo", "Edit";
    Copy => "copy", "Copy", "Edit";
    Cut => "cut", "Cut", "Edit";
    Paste => "paste", "Paste", "Edit";
    CopyAnsi => "copy_ansi", "Copy selection as ANSI text", "Edit";
    SelectAll => "select_all", "Select all", "Edit";
    Deselect => "deselect", "Deselect / cancel", "Edit";
    DeleteSelection => "delete", "Erase selection", "Edit";
    BlockMenu => "block_menu", "Block menu…", "Edit";
    FlipX => "flip_x", "Flip selection horizontally", "Edit";
    FlipY => "flip_y", "Flip selection vertically", "Edit";
    Rotate180 => "rotate_180", "Rotate selection 180°", "Edit";
    FillSelection => "fill_selection", "Fill selection with brush", "Edit";
    OutlineSelection => "outline_selection", "Outline selection with box", "Edit";
    JustifyLeft => "justify_left", "Justify left", "Edit";
    JustifyCenter => "justify_center", "Justify center", "Edit";
    JustifyRight => "justify_right", "Justify right", "Edit";
    DeleteBlock => "delete_block", "Delete block (shift left)", "Edit";
    CropToSelection => "crop", "Crop canvas to selection", "Edit";
    SaveStencil => "save_stencil", "Save selection as stencil…", "Edit";
    InsertLine => "insert_line", "Insert line", "Edit";
    DeleteLine => "delete_line", "Delete line", "Edit";
    InsertColumn => "insert_column", "Insert column", "Edit";
    DeleteColumn => "delete_column", "Delete column", "Edit";
    ClearCanvas => "clear", "Clear layer", "Edit";
    // Tools
    ToolSelect => "tool_select", "Select tool", "Tools";
    ToolText => "tool_text", "Text tool (type)", "Tools";
    ToolBrush => "tool_brush", "Brush", "Tools";
    ToolPen => "tool_pen", "Smart pen / brushes (fits the tile set)", "Tools";
    ToolPixel => "tool_pixel", "Half-block pixels", "Tools";
    ToolLine => "tool_line", "Line", "Tools";
    ToolRect => "tool_rect", "Rectangle", "Tools";
    ToolEllipse => "tool_ellipse", "Ellipse", "Tools";
    ToolFill => "tool_fill", "Fill bucket", "Tools";
    ToolGradient => "tool_gradient", "Gradient fill", "Tools";
    ToolPicker => "tool_picker", "Eyedropper", "Tools";
    ToolFont => "tool_font", "Font text stamp…", "Tools";
    GetFonts => "get_fonts", "Download more TheDraw fonts", "Tools";
    ToolStencil => "tool_stencil", "Stencils…", "Tools";
    ToolPattern => "tool_pattern", "Pattern brush", "Tools";
    PatternFromSelection => "pattern_from_selection", "Use selection as pattern", "Tools";
    PatternBrowse => "pattern_browse", "Patterns…", "Tools";
    PatternNext => "pattern_next", "Next pattern", "Tools";
    PatternPrev => "pattern_prev", "Previous pattern", "Tools";
    PatternSave => "pattern_save", "Save pattern…", "Tools";
    PatternDelete => "pattern_delete", "Delete saved pattern…", "Tools";
    ToolShade => "tool_shade", "Shade brush", "Tools";
    ToolColorize => "tool_colorize", "Colorize brush", "Tools";
    ToolErase => "tool_erase", "Eraser", "Tools";
    ToolFilters => "tool_filters", "Filters: photo looks (iPhone / Instagram)", "Tools";
    ToolRecolor => "tool_recolor", "Recolor: replace one color everywhere", "Tools";
    ApplyFx => "apply_fx", "Apply the filter / recolor", "Tools";
    ResetFilter => "reset_filter", "Reset the filter (Original, sliders to 0)", "Tools";
    ToolOption => "tool_option", "Cycle tool option", "Tools";
    ToolStyle => "tool_style", "Cycle shape look / box style", "Tools";
    Mirror => "mirror", "Cycle mirror mode", "Tools";
    BrushStudio => "brushes", "Brush studio…", "Tools";
    BrushBigger => "brush_bigger", "Bigger brush", "Tools";
    BrushSmaller => "brush_smaller", "Smaller brush", "Tools";
    ArtMode => "art_mode", "Art tool: the keyboard types blocks", "Tools";
    // Color & chars
    FgNext => "fg_next", "Next foreground color", "Color";
    FgPrev => "fg_prev", "Previous foreground color", "Color";
    BgNext => "bg_next", "Next background color", "Color";
    BgPrev => "bg_prev", "Previous background color", "Color";
    SwapColors => "swap_colors", "Swap foreground/background", "Color";
    PickUnderCursor => "pick", "Pick up color under cursor", "Color";
    ColorDialog => "color_dialog", "Colors…", "Color";
    CharPicker => "char_picker", "Character picker…", "Color";
    CharsetNext => "charset_next", "Next character set", "Color";
    CharsetPrev => "charset_prev", "Previous character set", "Color";
    Glyph1 => "glyph_1", "Charset glyph 1", "Color";
    Glyph2 => "glyph_2", "Charset glyph 2", "Color";
    Glyph3 => "glyph_3", "Charset glyph 3", "Color";
    Glyph4 => "glyph_4", "Charset glyph 4", "Color";
    Glyph5 => "glyph_5", "Charset glyph 5", "Color";
    Glyph6 => "glyph_6", "Charset glyph 6", "Color";
    Glyph7 => "glyph_7", "Charset glyph 7", "Color";
    Glyph8 => "glyph_8", "Charset glyph 8", "Color";
    Glyph9 => "glyph_9", "Charset glyph 9", "Color";
    Glyph10 => "glyph_10", "Charset glyph 10", "Color";
    // Document
    DocProperties => "doc_properties", "Document properties…", "Document";
    CanvasSize => "canvas_size", "Canvas size…", "Document";
    Sauce => "sauce", "SAUCE info…", "Document";
    ToggleIce => "toggle_ice", "Toggle iCE colors", "Document";
    ConvertModern => "convert_modern", "Convert to Modern (Unicode + RGB)", "Document";
    ConvertClassic => "convert_classic", "Convert to Classic (CP437 + 16 colors)", "Document";
    LayerAdd => "layer_add", "Add layer", "Layers";
    LayerDuplicate => "layer_duplicate", "Duplicate layer", "Layers";
    LayersPanel => "layers", "Layers…", "Layers";
    LayerRemove => "layer_remove", "Remove layer", "Layers";
    LayerUp => "layer_up", "Select layer above", "Layers";
    LayerDown => "layer_down", "Select layer below", "Layers";
    LayerMoveUp => "layer_move_up", "Move layer up", "Layers";
    LayerMoveDown => "layer_move_down", "Move layer down", "Layers";
    LayerToggle => "layer_toggle", "Toggle layer visibility", "Layers";
    LayerMerge => "layer_merge", "Merge layer down", "Layers";
    LayerRename => "layer_rename", "Rename layer…", "Layers";
    // Animation
    FramesPanel => "frames", "Frames: animation panel", "Frames";
    FrameAdd => "frame_add", "Add a blank frame", "Frames";
    FrameDuplicate => "frame_duplicate", "Duplicate frame", "Frames";
    FrameRemove => "frame_remove", "Delete frame", "Frames";
    FrameNext => "frame_next", "Next frame", "Frames";
    FramePrev => "frame_prev", "Previous frame", "Frames";
    FrameMoveLeft => "frame_move_left", "Move frame earlier", "Frames";
    FrameMoveRight => "frame_move_right", "Move frame later", "Frames";
    FramePlay => "frame_play", "Play / stop the animation", "Frames";
    OnionPrev => "onion_prev", "Onion skin: previous frame", "Frames";
    OnionNext => "onion_next", "Onion skin: next frame", "Frames";
    FpsUp => "fps_up", "Faster: one more frame per second", "Frames";
    FpsDown => "fps_down", "Slower: one less frame per second", "Frames";
    HoldMore => "hold_more", "Hold this frame a tick longer", "Frames";
    HoldLess => "hold_less", "Hold this frame a tick shorter", "Frames";
    // View
    Zoom => "zoom", "Toggle zoom (pixel view)", "View";
    Sidebar => "sidebar", "Toggle sidebar", "View";
    Minimap => "minimap", "Toggle minimap", "View";
    Grid => "grid", "Toggle grid guides", "View";
    Preview => "preview", "Pixel-exact preview", "View";
    PlayBaud => "play_baud", "Play at modem speed", "View";
    Replay => "replay", "Replay: watch how the piece was drawn", "View";
    // Cursor (keyboard drawing)
    Up => "up", "Cursor up", "Cursor";
    Down => "down", "Cursor down", "Cursor";
    Left => "left", "Cursor left", "Cursor";
    Right => "right", "Cursor right", "Cursor";
    PageUp => "page_up", "Page up", "Cursor";
    PageDown => "page_down", "Page down", "Cursor";
    LineStart => "line_start", "Start of line", "Cursor";
    LineEnd => "line_end", "End of line", "Cursor";
    FirstChar => "first_char", "First non-blank char", "Cursor";
    LastChar => "last_char", "Last non-blank char", "Cursor";
    TabStop => "tab_stop", "Next tab stop", "Cursor";
    Apply => "apply", "Apply tool at cursor", "Cursor";
    DrawUp => "draw_up", "Draw up with the brush", "Cursor";
    DrawDown => "draw_down", "Draw down with the brush", "Cursor";
    DrawLeft => "draw_left", "Draw left with the brush", "Cursor";
    DrawRight => "draw_right", "Draw right with the brush", "Cursor";
    // AI & library
    AiPrompt => "ai", "Ask AI…", "AI";
    AiSetup => "ai_setup", "AI & MCP setup…", "AI";
    AiStop => "ai_stop", "Stop the AI run", "AI";
    Harvest => "harvest", "Harvest fonts & stencils from art…", "AI";
    HarvestedFonts => "harvested_fonts", "My harvested fonts…", "AI";
    // Draw together
    TogetherPanel => "together", "Draw together: live drawing with others…", "Together";
    TogetherHost => "together_host", "Host: share this drawing live", "Together";
    TogetherJoin => "together_join", "Join a shared drawing (paste its ticket)", "Together";
    TogetherPasteTicket => "together_paste", "Paste a session ticket to join", "Together";
    TogetherCopyTicket => "together_copy", "Copy the session ticket", "Together";
    TogetherLeave => "together_leave", "Leave the shared drawing", "Together";
    // App
    CommandPalette => "palette", "Command palette", "App";
    Help => "help", "Help & keys", "App";
    Messages => "messages", "Message history…", "App";
    Settings => "settings", "Open settings file", "App";
    ReloadConfig => "reload_config", "Reload settings", "App";
}

impl Action {
    /// Keys that change only the brush can come in mid-drag without ending
    /// it: the stroke goes on in the new colors.
    pub fn keeps_drag(self) -> bool {
        use Action::*;
        matches!(self, FgNext | FgPrev | BgNext | BgPrev | SwapColors | CharsetNext | CharsetPrev)
    }
}

#[cfg(test)]
mod tests {
    use super::Action;

    /// The palette and `acidtrip keys` list actions in this order, one
    /// heading per category: each category must be in one run.
    #[test]
    fn categories_are_contiguous() {
        let mut seen: Vec<&str> = vec![];
        for a in Action::ALL {
            if seen.last() != Some(&a.category()) {
                assert!(!seen.contains(&a.category()), "{} splits category {}", a.id(), a.category());
                seen.push(a.category());
            }
        }
    }
}
