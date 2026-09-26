# Documents flows

Whole tasks around files, run by `cargo test -p acidtrip --test flows`
(`ACIDTRIP_FLOWS=documents/` for just these). Files go under
`/tmp/acidtrip-flows/documents-NN/`. "Found" names what the flow turned up
when it was written; "—" means it passed against the app as it was.

| Flow | Steps (short) | Covers | Found |
|---|---|---|---|
| 01 ans_roundtrip_export | draw, save .ans, quit, reopen from the command line, edit, undo past the reopen, save .xb, 2x PNG from the panel | save as, CLI open, undo at the reopen, export panel | undo back to the saved state stayed dirty (history.rs) |
| 02 new_document_discard | Ctrl-N over dirty work: no, then a 100x30 modern iCE doc by mouse; undo can't return; quit doesn't ask | new, discard confirm, kind/iCE | — |
| 03 quit_unsaved_cancel | Ctrl-Q: Esc, N, click "no"; still takes keys; save; quit without asking | quit guard | — |
| 04 quit_anyway_recover | quit anyway, restart, restore, quit again, restore, save, third start clean | recovery after quit | recovery kept/cleared at the wrong times (recovery, app.rs) |
| 05 crash_recovery_mouse | SIGKILL after autosave, discard all by mouse; undo to saved, crash again, nothing stale | crash recovery, autosave | stale recovery offered after undo to the saved state |
| 06 recover_later_then_resave | crash on a CLI file, "later" opens the disk copy, next start restores, Ctrl-S writes back with .bak | recovery of a named file | recovery lost after a restore then a second crash |
| 07 versions_restore_copy | two saves, restore older, undo restore, open copy by mouse | versions | — |
| 08 versions_after_reopen | .ans saved twice, restart, Alt-V lists both, restore and save | versions of non-.acid | .ans/.xb versions vanished on restart (versions.rs) |
| 09 export_rows_persist | panel rows by mouse, SVG in the row editor, Esc cancel, export 2, undo/redo, save, reopen, remove row | export panel, row editor | — |
| 10 overwrite_decline | export PNG, export over it (no, Esc), save over another .ans (no/yes) | overwrite confirm | "no" closed the Save dialog and lost the path (export.rs) |
| 11 roundtrip_classic_formats | colored art saved as .ans .xb .bin .adf .idf, each reopened | classic formats | — |
| 12 roundtrip_other_formats | same for .tnd .pcb .avt UTF-8 ANSI .acid | other formats | — |
| 13 lossy_resave_warning | two layers to .ans: the loss is shown, Ctrl-S asks, no/yes; then ASCII | loss warnings | — |
| 14 open_dialog_paths | missing path, folder path, filter, Tab completion, row click, discard confirm, already open, Esc | files dialog | typed paths opened some other file; rows not clickable (files.rs) |
| 15 command_line_files | corrupt .acid, two files, a missing name then Ctrl-S, a missing folder | CLI open | error hidden by the welcome line; zstd jargon; clipped messages (app.rs, native.rs) |
| 16 sauce_roundtrip | SAUCE: Esc throws typing away, fill in, OK by mouse, undo/redo, save, reopen, attach off | SAUCE | typed SAUCE on a file without one was dropped on save (forms.rs) |
| 17 canvas_size_modes | crop warning, fit art, +25 rows, undo/redo, width confirm, MODERN/iCE chips, reopen size, crop count | canvas size, classic/modern, iCE | — |
| 18 mouse_save_png_open | hover and click the title to save, export PNG, untitled title opens Save/Export, cancel, PNG from CLI | one-button save | no mouse path to Save (ui/mod.rs) |
| 19 small_terminal | 80x24: save .xb, export PNG, SAUCE, canvas, versions, quit | small terminals | status clipped the file name; form notes drawn under buttons (ui/mod.rs, forms.rs) |
| 20 resize_mid_dialog | Save dialog through 80x24/40x12/30x8 and back, every dialog at 40x12, panel gives way at 90, quit at 50x14 | resizing | Confirm border had a gap with no hint (widgets.rs) |
| 21 unwritable_locations | /dev/null/x, a file in the way, read-only folder (and below it), a folder with no name, then a good path; folder goes read-only under a saved piece | unwritable paths | long errors cut to two lines (export.rs); failed save named tempfile's ".tmpXXXX" (library.rs) |
| 22 backups | bak: two saves; export makes none; numbered: .001/.002; save as over another file backs it up; none | backups | — |
| 23 open_bad_files | damaged .acid, cut .xb/.adf, fake .png via Open while dirty; empty .ans; unknown extension as ANSI | corrupt files, files dialog | a failed open closed the dialog and lost the path (files.rs, app.rs) |
| 24 export_panel_folder | Alt-Shift-E, scale click, add row, folder to a subfolder, read-only folder, selection export, whole, "as…", reopen, undo folder | export panel, export folder | export into a read-only folder named the temp file (library.rs, same fix as 21) |
| 25 escape_every_dialog | New, Save/Export, export mode, Open, SAUCE, properties, versions, canvas size — each with changes, then Esc | cancelling | — |

## Open

- Export rows live in document metadata, so saving to a non-.acid format
  drops them.
- .ans and .xb trim to the used rows by default, so a canvas with empty
  rows at the bottom reopens shorter.
- Alt-Shift-E on an untitled piece writes into the working directory
  without asking.
