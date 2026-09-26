# Test corpus

Sample art used by the `format_*` round-trip tests.

- `*.ans`, `*.ANS`, `DR-HEART.ASC`: copied from the icy_tools repository,
  `crates/icy_draw/doc/` (https://github.com/mkrueger/icy_tools), which is
  distributed under MIT OR Apache-2.0 (the `icy_draw` crate declares
  Apache-2.0). The artwork remains the work of its original artists (see the
  SAUCE records inside each file).

The ansilove textmode corpus (https://github.com/ansilove/textmode-corpus,
XB/ADF/IDF/TND/PCB/24-bit ANS) has no license file, so it is not copied here.
The `#[ignore]` test `format_corpus::textmode_corpus` loads it from a local
clone; point `ACIDTRIP_TEXTMODE_CORPUS` at the checkout to run it.
