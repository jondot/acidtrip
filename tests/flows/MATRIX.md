# Flow matrix

One row per flow. **Covers** names the features the flow crosses; **Found** the
bugs it turned up (fixed in the same change unless marked open).

The rows live in each area's matrix; this page sums them up.

| Area | Flows | Bugs found and fixed | Matrix |
|---|---|---|---|
| documents | 25 | 20 | [documents/MATRIX.md](documents/MATRIX.md) |
| drawing | 26 | 28 | [drawing/MATRIX.md](drawing/MATRIX.md) |
| library | 20 | 34 | [library/MATRIX.md](library/MATRIX.md) |
| app | 20 | 24 | [app/MATRIX.md](app/MATRIX.md) |
| **total** | **91** | **106** | |

Bugs are counted from each matrix's **Found** column, one per separate fault
named, and each fault only once. The "popup with an empty hint leaves a gap in
its border" bug was found in all four areas and is one fix (`ui/widgets.rs`),
counted under documents. Drawing flow 21 found the same wrong-layer-after-
undoing-a-merge bug as flow 07, so it counts once. Merging the four areas turned up two more. The Font dialog
said which letters a font lacks from what was typed, not from the "ACiD"
sample it shows before you type, so the warning never showed for the sample
(`dialogs/fonts.rs`, library flow 16). And a view scrolled to fit beside the
narrow sidebar stayed scrolled after the sidebar closed or the terminal grew,
leaving the left of the canvas off screen with empty room to the right
(`Tab::clamp_scroll`, drawing flow 17).

`crates/acidtrip/tests/palette_all.rs` goes with the flows: it runs every
palette command on a fresh document and checks that Esc gets back to the
canvas.

Run everything with `cargo test -p acidtrip --test flows`;
`ACIDTRIP_FLOWS=drawing/` runs one area, `ACIDTRIP_FLOW_JOBS=1` one flow at a
time, and `ACIDTRIP_FLOW_BIN=path/to/acidtrip` runs them against another build.

## Still open

Checked against the merged code; items another area fixed are left out
(the Save dialog's missing Save button, the mouse path to Open, New and Quit,
undo by mouse, and the dirty flag after undoing back to the saved state are
all fixed now).

Files
- Export rows live in the document's metadata, so saving to a non-.acid
  format drops them.
- .ans and .xb trim to the used rows, so a canvas with empty rows at the
  bottom reopens shorter.
- Exporting from an untitled piece (Alt-Shift-E, or a replay GIF / .cast)
  writes into the working directory without asking.
- A restored document loses its edit log.

Drawing
- After a crop, the status bar can show hover coordinates outside the canvas.
- A block-menu Move takes two undo steps.
- Insert/delete line or column also shifts locked layers.
- The ↻ and ⇆ Select chips change the block without a status message.
- A move that changes nothing still clears the redo stack.
- '/' can't be typed in the Font tool's text (it opens the filter).

Small terminals and layout
- Info messages longer than the room left in the status bar (about 58
  characters at 80 columns) are cut with "…"; paths shrink to their names
  first, and warnings push the read-outs away so they show whole.
- Help at 30x10 cuts its hint.
- At 20 rows the sidebar's CHARACTERS and LAYERS panels drop out.
- The Layers dialog's hint touches its border.

By design, noted so they aren't reported again: number keys stamp at the
cursor and step right (as in ACiDDraw), and `=` / `-` switch any tool to the
Pen.
