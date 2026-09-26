# User flows

Each `.at` file here walks one whole task the way a person would: many steps,
across tools, panels and dialogs, checking the state after each step and at
the end (files written, what the screen shows, undo). They catch what
one-feature scenarios in `tests/e2e/` miss: features that work alone but
break in combination.

- `documents/` new, open, save, export, tabs, recovery, versions, canvas size
- `drawing/` tools, selections and blocks, layers, frames, undo across all of it
- `library/` gallery, studio, fonts, stencils, patterns, brushes, image import, AI
- `app/` together, replay, command palette, keys, config, small terminals, one-button mouse

Run them with `cargo test -p acidtrip --test flows`. `ACIDTRIP_FLOWS=drawing/`
runs one area, `ACIDTRIP_FLOW_JOBS=1` runs them one at a time. Files a flow
writes go under `/tmp/acidtrip-flows/<flow>/`. Screenshots land in
`target/shots/flows/`.
