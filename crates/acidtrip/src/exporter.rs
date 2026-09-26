//! The EXPORT panel, Figma-style: rows of format, scale and file name kept
//! with the piece, one folder, and one button that writes them all.
//! Drawing lives in the sidebar; this is what the panel shows and does.

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use acidtrip_core::tools::Rect;
use acidtrip_core::{Document, ExportPreset};
use acidtrip_io::exports::{self, Job};

use crate::actions::Action;
use crate::app::{App, Level};
use crate::dialogs;
use crate::tab::Tab;
use crate::ui::sidebar::ExportHit;

/// The piece's own folder: where its file is, else the working directory.
pub fn base_dir(tab: &Tab) -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    match tab.file.as_deref().and_then(Path::parent) {
        Some(p) if p.as_os_str().is_empty() => cwd,
        Some(p) if p.is_absolute() => p.to_path_buf(),
        Some(p) => cwd.join(p),
        None => cwd,
    }
}

/// Where the files go.
pub fn folder(tab: &Tab) -> PathBuf {
    exports::folder(&tab.doc.meta.exports, &base_dir(tab))
}

/// The part being exported: the selection, unless "whole" is picked.
pub fn scope(tab: &Tab) -> Option<Rect> {
    tab.selection.filter(|r| !tab.export_whole && r.w > 0 && r.h > 0)
}

/// Cells wide and high of what gets exported.
pub fn size(tab: &Tab) -> (usize, usize) {
    let (w, h) = (tab.doc.width(), tab.doc.height());
    match scope(tab) {
        Some(r) => (r.w.min(w.saturating_sub(r.x)).max(1), r.h.min(h.saturating_sub(r.y)).max(1)),
        None => (w, h),
    }
}

/// What `{name}` means: the piece's name, "-selection" for a part of it.
pub fn name(tab: &Tab) -> String {
    let n = exports::doc_name(tab.file.as_deref(), &tab.doc);
    if scope(tab).is_some() { format!("{n}-selection") } else { n }
}

pub fn rows(tab: &Tab) -> Vec<ExportPreset> {
    exports::rows(&tab.doc.meta.exports)
}

/// Every file Export writes, or why it can't.
pub fn plan(tab: &Tab) -> Result<Vec<Job>, String> {
    exports::plan_sized(&tab.doc, size(tab), &rows(tab), &folder(tab), &name(tab))
}

/// The files row `i` writes (several for a per-frame row).
pub fn row_files(tab: &Tab, i: usize) -> Vec<String> {
    let Some(row) = rows(tab).get(i).cloned() else { return vec![] };
    exports::plan_sized(&tab.doc, size(tab), &[row], Path::new(""), &name(tab))
        .map(|jobs| jobs.iter().map(|j| j.path.display().to_string()).collect())
        .unwrap_or_default()
}

/// A path for showing: the home folder as "~".
pub fn pretty(p: &Path) -> String {
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from)
        && let Ok(rest) = p.strip_prefix(&home)
    {
        return if rest.as_os_str().is_empty() { "~".into() } else { format!("~/{}", rest.display()) };
    }
    p.display().to_string()
}

/// File names for a message: all of them when few, else first … last.
pub fn file_list(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [a, .., b] if names.len() > 4 => format!("{a} … {b} ({} files)", names.len()),
        _ => names.join(", "),
    }
}

impl App {
    /// Ctrl-E: open or close the EXPORT panel. Without room for the sidebar,
    /// the Export as… dialog instead.
    pub fn toggle_export_panel(&mut self) {
        let wide = crossterm::terminal::size().map_or(true, |(w, _)| w >= 100);
        if !wide {
            self.run(Action::ExportAs);
            return;
        }
        if !self.show_sidebar {
            self.show_sidebar = true;
            self.tab_mut().export_panel = true;
            return;
        }
        let t = self.tab_mut();
        t.export_panel = !t.export_panel;
    }

    /// Write every row. Existing files are replaced without asking.
    pub fn export_now(&mut self) {
        let tab = self.tab();
        let jobs = match plan(tab) {
            Ok(jobs) => jobs,
            Err(e) => return self.flash(format!("export: {e}"), Level::Error),
        };
        let doc: Cow<Document> = match scope(tab) {
            Some(r) => Cow::Owned(exports::scoped(&tab.doc, r)),
            None => Cow::Borrowed(&tab.doc),
        };
        let dir = folder(tab);
        match exports::write(&doc, &jobs) {
            Ok(()) => {
                let names: Vec<String> =
                    jobs.iter().filter_map(|j| j.path.file_name()).map(|n| n.to_string_lossy().into_owned()).collect();
                self.flash(format!("exported {} → {}", file_list(&names), pretty(&dir)), Level::Ok);
            }
            Err(e) => self.flash(format!("export failed: {e:#}"), Level::Error),
        }
    }

    /// Change the rows as one undo step.
    pub fn edit_export_rows(&mut self, label: &str, f: impl FnOnce(&mut Vec<ExportPreset>)) {
        let mut rows = rows(self.tab());
        let before = rows.clone();
        f(&mut rows);
        if rows != before {
            self.tab_mut().edit(label, move |b| b.replace_meta(|m| m.exports.presets = rows));
        }
    }

    /// A click in the EXPORT panel; `alt` is the right button (or a modifier).
    pub fn export_click(&mut self, h: ExportHit, alt: bool) {
        match h {
            ExportHit::Whole(whole) => {
                if !whole && self.tab().selection.is_none() {
                    self.flash("select an area first (V, drag) to export just that", Level::Info);
                }
                self.tab_mut().export_whole = whole;
            }
            ExportHit::Scale(i) => {
                let has = rows(self.tab()).get(i).and_then(exports::format_of).is_some_and(exports::has_scale);
                if has {
                    self.edit_export_rows("Export scale", |rows| {
                        let r = &mut rows[i];
                        exports::set_scale(r, exports::step_scale(r.scale, if alt { -1 } else { 1 }));
                    });
                } else {
                    self.flash("this format has no pixel scale", Level::Info);
                }
            }
            ExportHit::Name(i) => {
                self.dialogs.push(Box::new(dialogs::export_row::ExportRowDialog::new(self, i, false)))
            }
            ExportHit::Format(i) => {
                self.dialogs.push(Box::new(dialogs::export_row::ExportRowDialog::new(self, i, true)))
            }
            ExportHit::Remove(i) => self.edit_export_rows("Remove export", |rows| {
                if rows.len() > 1 && i < rows.len() {
                    rows.remove(i);
                }
            }),
            ExportHit::Add => self.edit_export_rows("Add export", |rows| {
                let r = exports::next_row(rows);
                rows.push(r);
            }),
            ExportHit::Folder if alt => self.set_export_folder(None),
            ExportHit::Folder => {
                let cur = pretty(&folder(self.tab()));
                self.dialogs.push(Box::new(dialogs::prompt::PromptDialog::new(
                    "Export to folder (empty: next to the piece)",
                    &cur,
                    Box::new(|app: &mut App, text: String| {
                        let text = text.trim();
                        let chosen = if text.is_empty() {
                            None
                        } else {
                            let p = dialogs::forms::expand_tilde(text);
                            Some(if p.is_absolute() { p } else { base_dir(app.tab()).join(p) })
                        };
                        app.set_export_folder(chosen);
                    }),
                )));
            }
            ExportHit::Run => self.export_now(),
        }
    }

    /// Remember the export folder with the piece (None: next to it).
    fn set_export_folder(&mut self, chosen: Option<PathBuf>) {
        let base = base_dir(self.tab());
        let stored = chosen.and_then(|p| exports::store_folder(&p, &base));
        if stored != self.tab().doc.meta.exports.folder {
            self.tab_mut().edit("Export folder", |b| b.replace_meta(|m| m.exports.folder = stored));
        }
        let shown = pretty(&folder(self.tab()));
        self.flash(format!("exports go to {shown}"), Level::Info);
    }
}

/// Hover tip for an EXPORT panel element.
pub fn tip(app: &App, h: ExportHit) -> String {
    let tab = app.tab();
    let row = |i: usize| rows(tab).get(i).cloned().unwrap_or_default();
    match h {
        ExportHit::Whole(true) => {
            format!("export the whole piece ({}×{})", tab.doc.width(), tab.doc.height())
        }
        ExportHit::Whole(false) => match tab.selection {
            Some(r) => format!("export only the selection ({}×{})", r.w, r.h),
            None => "select an area first (V, drag) to export just that".into(),
        },
        ExportHit::Scale(i) => match exports::format_of(&row(i)) {
            Some(f) if exports::has_scale(f) => {
                format!("{}x pixels — click for bigger, right-click for smaller", row(i).scale)
            }
            _ => "this format has no pixel scale".into(),
        },
        ExportHit::Name(i) => format!(
            "writes {} — click to rename ({{name}} {{scale}} {{frame}} {{w}} {{h}})",
            file_list(&row_files(tab, i))
        ),
        ExportHit::Format(i) => match exports::format_of(&row(i)) {
            Some(f) => format!("{} — click to change the format and its options", f.name()),
            None => "unknown format — click to pick one".into(),
        },
        ExportHit::Remove(_) => "remove this export".into(),
        ExportHit::Add => "add an export: another format or size".into(),
        ExportHit::Folder => {
            format!("files go to {} — click to change, right-click: next to the piece", pretty(&folder(tab)))
        }
        ExportHit::Run => {
            let key = app.keymap.key_for(Action::ExportNow).map(|k| format!("  [{k}]")).unwrap_or_default();
            match plan(tab) {
                Ok(jobs) => {
                    let names: Vec<String> = jobs
                        .iter()
                        .filter_map(|j| j.path.file_name())
                        .map(|n| n.to_string_lossy().into_owned())
                        .collect();
                    format!("write {} (replacing old ones){key}", file_list(&names))
                }
                Err(e) => e,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_lists_stay_short() {
        let n = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(file_list(&n(&["a.png", "b.svg"])), "a.png, b.svg");
        assert_eq!(file_list(&n(&["1", "2", "3", "4", "5"])), "1 … 5 (5 files)");
    }

    #[test]
    fn selection_scope_names_and_sizes() {
        let mut tab = Tab::new(Document::new(acidtrip_core::DocKind::Classic, 80, 25), Some("/x/art.ans".into()));
        assert_eq!((name(&tab), size(&tab)), ("art".to_string(), (80, 25)));
        assert_eq!(base_dir(&tab), Path::new("/x"));
        tab.selection = Some(Rect { x: 70, y: 0, w: 20, h: 5 });
        assert_eq!((name(&tab), size(&tab)), ("art-selection".to_string(), (10, 5)));
        tab.export_whole = true;
        assert_eq!(name(&tab), "art");
    }
}
