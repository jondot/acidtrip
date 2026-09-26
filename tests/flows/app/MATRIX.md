# App flow matrix

One row per flow in `tests/flows/app/`. **Covers** names the features the
flow crosses; **Found** names the bugs it turned up. They were fixed in the same
change unless marked open. Step counts include waits and checks. Run the flows
with `ACIDTRIP_FLOWS=app/ cargo test -p acidtrip --test flows`.

`crates/acidtrip/tests/palette_all.rs` goes with these flows. It runs every
palette command on a fresh document and checks that Esc gets back to the
canvas.

| Flow | Steps (short) | Covers | Found |
|---|---|---|---|
| 01_first_launch_restart | 50: welcome, config created, first stroke, quit asks (n, then y), restart without welcome | welcome, first launch, quit prompt, config persistence | - |
| 02_palette_search_run | 65: search, no hits, run by Enter, click and wheel, Esc and click outside, command opening a dialog | palette, dialogs, mouse | A click inside the palette closed it (palette.rs); no empty-state text, and the config name of a command wasn't searchable (palette.rs) |
| 03_help_sheet | 56: ? / palette / Alt-H, scroll by keys and wheel, other key closes, live keymap, small terminal | help sheet, keymap, resize | Help couldn't scroll, lacked PgUp/PgDn, and listed stale keys instead of the live keymap (help.rs) |
| 04_keymap_acid | 51: acid preset from config, typing start, Esc opens colors, Alt commands, palette and help show preset keys | keymap presets, config, palette, help | Preset keys weren't listed first (keymap.rs); an unknown preset was silently ignored (keymap.rs, app.rs, cli.rs); `acidtrip keys` repeated categories and Shift-Tab (actions.rs, keymap.rs); `keys \| head` panicked (cli.rs) |
| 05_custom_bindings | 61: plain-letter binding, override of a tool key, bad entries reported, Reload settings | custom bindings, config, status line | Overrides landed in the wrong context (keymap.rs); only the first config problem was shown, as an overlong line (app.rs); long status text wasn't cut with an ellipsis (ui/mod.rs) |
| 06_config_options | 56: new-doc kind, size and iCE, sidebar and grid start, SAUCE author, settings editor round trip, missing editor | config options, settings editor, forms | No full repaint after the editor (app.rs); form notes ran past the edge and the hint didn't fit (forms.rs) |
| 07_status_chips | 49: size chip, cancel by click, iCE both ways, kind convert then undo, ⇄ panel, hover tips | status chips, undo, Together panel | Form dialogs had no clickable OK/Cancel (forms.rs) |
| 08_dialog_stacking | 91: prompt over Layers, Confirm then form, palette opening dialog opening dialog, Esc peels one | dialog stack, prompts, mouse | A click outside didn't close a dialog (app.rs, dialogs/mod.rs); the prompt couldn't be answered by mouse, and Confirm had extra blank rows (prompt.rs); popup border gap (widgets.rs) |
| 09_replay_scrub_edit | 93: draw with undo, save .acid, replay, pause, step, Home/End, scrub, speed, skip idle, edit leaves replay, restart | replay panel, scrubbing, persistence | - |
| 10_replay_export | 58: GIF and .cast export from paused and playing replay, .ans modem reveal, Esc | replay export | Open: exporting an untitled replay writes into the current directory |
| 11_together_two_sessions | 68: bad ticket, host, join, strokes both ways, local undo, leave, end | draw together (localhost), clipboard, undo | "draw together" in the palette ran Host because multi-word matching was loose (widgets.rs) |
| 12_small_terminal | 83: 80x24 welcome, chips, palette, forms, help to end, Ctrl-B sidebar, Together and Replay panels, draw, save | small terminal, sidebar, panels | Under 100 columns Ctrl-B did nothing and the Together/Replay panels never showed (app.rs, ui/mod.rs, together.rs, replay.rs) |
| 13_resize_mid_action | 63: 220x50, shrink with a dialog up, resize with palette query, resize mid-drag, 80x24, 30x10 help, grow back | resize, dialogs, drag undo | Open: help at 30x10 cuts its hint |
| 14_crowded_sidebar | 41: 120x30, Together plus Replay swap one slot, Esc brings Together back, layers +, height 20, ✕ | crowded sidebar, panels | Open: at 20 rows CHARACTERS and LAYERS drop out |
| 15_one_button_mouse | 62: welcome, tool, fill, char and swatch by click, rect, logo to palette, Undo/Redo rows, New doc, save, quit by click | one-button mouse | The logo didn't open the palette (ui/mod.rs, sidebar.rs); recovery and Colors dialogs needed keys or were clipped (recovery.rs, colors.rs); harness errored when a click quit the app (session.rs). The save is now by the Save / Export dialog's ⏎ save button (added in the documents area) |
| 16_keyboard_only | 73: welcome keys, cursor, Shift-arrow line, digit stamps, text, Colors, add and rename layer, save, restart | keyboard only, layers, persistence | Open: the Layers dialog hint touches its border |
| 17_crash_recover | 55: autosave, kill, Later, Discard, second crash, restore by click, save, clean restart | recovery, autosave, restart | Harness needed `wait-file` to wait for autosave rather than sleep (script.rs). Open: a restored doc loses its edit log |
| 18_text_tool_and_commands | 55: typing, palette while typing, Esc back to typing, Canvas size, undo via palette, resume typing, ? outside text | text tool, palette, undo, help | Undoing typed text left the cursor after the removed text, so new typing left a gap (tab.rs, app.rs) |
| 19_frames_playback | 64: FRAMES panel, duplicate, play, Esc stops, palette and help over playback, canvas click, save, restart, play | frames, playback, persistence | A canvas click during playback painted on whichever frame was showing (app.rs); Esc didn't stop playback (app.rs) |
| 20_together_host_ends | 59: guest with unsaved art asked first (No keeps it), join, host ends session, guest keeps drawing and saves | draw together (localhost), confirm, save | - |
