# Drawing flow matrix

One row per flow in `tests/flows/drawing`. **Covers** names the features the
flow crosses; **Found** the bugs it turned up, fixed in the same change
unless marked *open*. Run them with
`ACIDTRIP_FLOWS=drawing/ cargo test -p acidtrip --test flows`.

| Flow | Steps (short) | Covers | Found |
|---|---|---|---|
| 01_logo_layers_undo | font logo → select → flip → gradient → new layer → pattern → move layer below → hide/show → undo ×5 → redo → merge → save .ans → reopen | font, select, flip, gradient, pattern, layers, undo/redo, .ans | redo of a layer move now keeps the moved layer active (was left on the other one) |
| 02_brush_undo_midstroke | brush strokes, glyph keys, eraser, right-drag erase, undo/redo, new edit drops redo, tool key and Esc mid-drag | brush, erase, undo groups | a tool switch mid-drag merged the next edit into the stroke's undo step |
| 03_text_lines_columns | type two lines, insert/delete line and column, insert mode, backspace, undo a typed run, save .ans, reopen | text, insert/delete line/column, .ans | – |
| 04_shapes_looks | rect ×3 looks, ellipse, line, tool switch mid-shape, undo all, redo all | rect, ellipse, line, box styles | Esc mid-shape didn't cancel the shape |
| 05_keyboard_only | arrows, number keys, Shift+arrows trail, Space-Space shapes, Esc drops anchor, Space-Space select, block menu by key, undo | keyboard-only drawing, block menu | stale "anchor set" message; Esc dropping an anchor was silent |
| 06_fill_modes | fill inside/outside a box, match all/char/color, color-only paint, undo | fill | – |
| 07_layer_lock_hide | draw on locked and hidden layers, Layers dialog chips by mouse, merge into locked, undo flags, reference layer not saved, reopen | layers, lock, hide, reference, merge | edits on locked/hidden layers were silent; merge down on bottom layer silent; merge into a locked layer changed it; no clickable lock/reference chips; empty popup hint left a gap in the border; undoing a merge left the layer below active |
| 08_block_menu_chain | justify left/right/center, rotate, outline, fill, delete block, crop, undo each | block menu | crop gave no feedback (and nothing without a selection) |
| 09_clipboard_layers | copy composite, cut active layer, carry float, stamp at the edge, Esc, locked layer refuses cut/paste/block edits | clipboard, float, layers | cut/move copied all layers' composite; move/cut/block edits on a locked layer were silent; Layers key line truncated |
| 10_frames_onion_undo | draw frame 1, add frame (onion), draw frame 2, duplicate, reorder, delete, undo jumps to frame, save .acid, reopen | frames, onion skin, undo, .acid | – |
| 11_pixel_ice_modern | half-block pixels unzoomed and zoomed, iCE on/off, Classic → Modern, undo | pixel tool, zoom, iCE, Classic/Modern | Pixel tool message truncated |
| 12_gradient_pattern | linear, reversed, radial, in-selection gradients; pattern from selection stamped as rect; brush size; undo | gradient, pattern | – |
| 13_color_tools | shade by right-click and one-button chip, colorize, eyedropper, recolor, filters preview + Esc, mirror, undo each | shade, colorize, picker, recolor, filters, mirror | Esc in Filters kept the preview |
| 14_stencil_font | built-in stencil stamped twice, undo one, FIGlet text stamped, save selection as stencil, search and stamp, Esc and empty name | stencils, font tool | font dialog appended typed text to the "ACiD" sample; *open:* '/' can't be typed in the font text |
| 15_mouse_only_select | one button, no keys: close welcome, pick red and brush, draw, Select chips flip/rotate/outline/fill/block menu, dimmed chips | one-button mouse, select panel | dimmed ⋯ chip with no selection jumped to the Text tool |
| 16_art_keyboard | Art mode: left-hand blocks, IJKL, Shift+IJKL, U/O ink, [ ] sets, R erase, Y/H undo/redo, locked layer, save, reopen | Art mode keys | Art-mode R erase on a locked layer was silent |
| 17_small_terminal | 80×24: Ctrl-B, scroll to the bottom row and corner, warnings in the narrow status bar, dialogs fit, grow the terminal | small terminal, resize | Ctrl-B under 100 columns silently flipped a hidden switch (merged with the app area's fix: Ctrl-B lays the sidebar over the canvas); warnings truncated at 80 columns; *open:* info messages over ~58 chars still cut at 80 columns |
| 18_mirror_erase_resize | m cycles mirror, sidebar chips, Erase tool, right-drag, Ctrl/Alt-drag erase, resize mid-stroke, undo, save, reopen | mirror, erase, one-button erase, resize mid-drag | – |
| 19_pen_studio | = / - sizes, Tab presets, Brush studio Marker, Esc mid-stroke, pen on a hidden layer, save, reopen | pen, brush studio, hidden layer | – |
| 20_characters | sidebar set arrows and glyphs by mouse, ⋯ all picker, Alt-K picker, [ ] and number keys, CP437 save/reopen | characters panel, char picker, CP437 | – |
| 21_long_undo | brush, text, layers, rect, line, merge, frame; undo each by label, redo all, draw after undone merge, save, reopen | undo/redo across tools, layers, frames | undoing a merge/removal left the wrong layer active, so drawing went onto the layer below |
| 22_canvas_edges | type past column 79, Enter on last row, insert line/column with art at the edge, undo, deletes at the corner, save, reopen | text and line/column edits at edges | typing past the edge and Enter on the last row silently overwrote; insert/delete line/column gave no feedback and silently pushed art off |
| 23_stamp_modes | bars, "A B" clip with a hole, paste in clear/solid/under, preview checked before each stamp, undo, save, reopen | stamp modes, float preview | the float preview drew blank cells over art in clear/under modes, unlike the stamp |
| 24_keyboard_clipboard_frames | 80×24 keyboard only: Space-Space select, copy, paste, arrows carry the float, resize while carried, Tab mode, stamp on frames 1 and 2, undo on frame 2, paste on hidden layer, save .acid, reopen | keyboard clipboard, frames, resize, hidden layer | with the sidebar hidden nothing showed which frame was up; the status bar now shows F n/m |
| 25_corner_block_ops | selection dragged past the canvas edge, ↻ and ⇆ chips, cut, paste half off the canvas, stamp, undo, redo, save, reopen | select at edges, rotate/flip glyphs, cut, clipped paste | a 180° turn made b into d (not q); cut said "copied" |
| 26_move_cancel | Esc mid-move, click inside a selection, tool switch mid-move, real move, undo/redo, save, reopen | move, Esc/tool switch mid-drag, undo | a cancelled move or a plain click in a selection left an empty "Move" undo step |

## Open

- Number keys stamp at the cursor and step right (ACiDDraw design, kept).
- After a crop, the status bar can show hover coordinates outside the canvas.
- A block-menu Move takes two undo steps.
- Info messages longer than about 58 characters are cut off at 80 columns.
- '/' can't be typed in the font text.
- Insert/delete line or column also shifts locked layers.
- `=` / `-` from any tool switch to the Pen (deliberate).
- The ↻ and ⇆ Select chips change the block without a status message.
- A no-op move still clears the redo stack.
