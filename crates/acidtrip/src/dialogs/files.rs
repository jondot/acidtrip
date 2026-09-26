//! Open dialog: fuzzy finder over art files under the current directory,
//! plus a typed path (Tab completes, ~ expands).

use std::path::{Path, PathBuf};

use acidtrip_io::format::{self, Format};
use crossterm::event::{KeyCode, KeyEvent, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;

use super::{Dialog, Outcome};
use crate::app::{App, open_error};
use crate::dialogs::forms::expand_tilde;
use crate::ui::widgets::{LineInput, ListState, centered, fuzzy, list_line, popup, theme};

pub struct OpenDialog {
    input: LineInput,
    list: ListState,
    all: Vec<PathBuf>,
    filtered: Vec<usize>,
    list_area: Rect,
    root: PathBuf,
    /// Why the last Enter opened nothing.
    error: Option<String>,
}

const MAX_FILES: usize = 5000;

pub fn scan(root: &Path) -> Vec<PathBuf> {
    let mut out = vec![];
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut entries: Vec<_> = rd.flatten().map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with('.') || name == "target" || name == "node_modules" {
                continue;
            }
            if p.is_dir() {
                if depth < 4 {
                    stack.push((p, depth + 1));
                }
            } else if Format::from_path(&p).is_some_and(|f| f.can_load()) {
                out.push(p);
                if out.len() >= MAX_FILES {
                    return out;
                }
            }
        }
    }
    out
}

impl OpenDialog {
    pub fn new() -> Self {
        let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let all = scan(&root);
        let mut d = OpenDialog {
            input: LineInput::default(),
            list: ListState::default(),
            filtered: vec![],
            all,
            list_area: Rect::default(),
            root,
            error: None,
        };
        d.refilter();
        d
    }

    fn rel(&self, p: &Path) -> String {
        p.strip_prefix(&self.root).unwrap_or(p).display().to_string()
    }

    /// The filter text: a typed path inside the listed folder filters by
    /// what follows the folder ("~/art/" lists everything in ~/art).
    fn query(&self) -> String {
        let text = self.input.text.trim();
        if looks_like_path(text)
            && let Ok(rest) = expand_tilde(text).strip_prefix(&self.root)
        {
            return rest.display().to_string();
        }
        text.to_string()
    }

    fn inside_root(&self) -> bool {
        let text = self.input.text.trim();
        !looks_like_path(text) || expand_tilde(text).starts_with(&self.root)
    }

    fn refilter(&mut self) {
        let q = self.query();
        let mut scored: Vec<(i32, usize)> =
            self.all.iter().enumerate().filter_map(|(i, p)| fuzzy(&q, &self.rel(p)).map(|s| (s, i))).collect();
        if !q.is_empty() {
            scored.sort_by_key(|x| std::cmp::Reverse(x.0));
        }
        self.filtered = scored.into_iter().map(|(_, i)| i).collect();
        self.list.selected = 0;
    }

    fn open(&mut self) -> Outcome {
        let text = self.input.text.trim().to_string();
        let typed = expand_tilde(&text);
        let path = if !text.is_empty() && typed.is_file() {
            Some(typed)
        } else if looks_like_path(&text) && (typed.is_dir() || !self.inside_root() || self.filtered.is_empty()) {
            // A path, not a filter: never open some other file instead.
            if typed.is_dir() {
                self.enter(&typed);
            } else {
                self.error = Some(format!("no file at {}", typed.display()));
            }
            None
        } else {
            let hit = self.filtered.get(self.list.selected).map(|&i| self.all[i].clone());
            if hit.is_none() {
                self.error = Some(format!("no art file matches \"{text}\" — type a path"));
            }
            hit
        };
        let Some(p) = path else { return Outcome::Keep };
        // Read it here, so a file that won't open says why with the path
        // still in the box to fix, rather than after the dialog has gone.
        match format::load_with_log(&p) {
            Ok((doc, log)) => Outcome::Then(Box::new(move |app: &mut App| app.open_loaded(&p, doc, log))),
            Err(e) => {
                self.error = Some(open_error(&p, &e));
                Outcome::Keep
            }
        }
    }

    /// List the art files under `dir` and keep typing from there.
    fn enter(&mut self, dir: &Path) {
        let mut s = dir.display().to_string();
        if !s.ends_with('/') {
            s.push('/');
        }
        self.all = scan(dir);
        self.root = dir.to_path_buf();
        self.input = LineInput::new(&s);
        self.refilter();
    }

    fn complete(&mut self) {
        let typed = expand_tilde(self.input.text.trim());
        let (dir, prefix) = if self.input.text.ends_with('/') {
            (typed.clone(), String::new())
        } else {
            (
                typed.parent().map(Path::to_path_buf).unwrap_or_default(),
                typed.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            )
        };
        let dir = if dir.as_os_str().is_empty() { PathBuf::from(".") } else { dir };
        let Ok(rd) = std::fs::read_dir(&dir) else {
            return;
        };
        let mut matches: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with(&prefix)))
            .collect();
        matches.sort();
        if let Some(m) = matches.first() {
            if m.is_dir() {
                self.enter(&m.clone());
            } else {
                self.input = LineInput::new(&m.display().to_string());
                self.refilter();
            }
        }
    }
}

/// Typed text that names a path rather than filters the list.
fn looks_like_path(s: &str) -> bool {
    s.starts_with('/') || s.starts_with('~') || s.starts_with("./") || s.starts_with("../")
}

impl Dialog for OpenDialog {
    fn draw(&mut self, f: &mut Frame, area: Rect, _app: &App) {
        let r = centered(area, 80, 26);
        let inner = popup(f, r, "Open", "type to filter or a path · Tab complete · Enter open · Esc");
        self.input.render(f, Rect::new(inner.x, inner.y, inner.width, 1), "› ", true);
        let under = match &self.error {
            Some(e) => Span::styled(e.clone(), Style::new().fg(theme::ERR)),
            None => Span::styled(format!("in {}", self.root.display()), Style::new().fg(theme::DIM)),
        };
        f.render_widget(Paragraph::new(under), Rect::new(inner.x, inner.y + 1, inner.width, 1));
        let lr = Rect::new(inner.x, inner.y + 2, inner.width, inner.height.saturating_sub(2));
        self.list_area = lr;
        if self.filtered.is_empty() {
            f.render_widget(
                Paragraph::new(Span::styled("  no art files here — type a path", Style::new().fg(theme::DIM))),
                lr,
            );
            return;
        }
        let range = self.list.visible(self.filtered.len(), lr.height as usize);
        let lines: Vec<_> = range
            .map(|i| {
                let p = &self.all[self.filtered[i]];
                let fmt = Format::from_path(p).map(|f| f.name()).unwrap_or("");
                list_line(self.rel(p), fmt, i == self.list.selected, lr.width)
            })
            .collect();
        f.render_widget(Paragraph::new(lines), lr);
    }

    fn key(&mut self, k: KeyEvent, _app: &App) -> Outcome {
        match k.code {
            KeyCode::Esc => Outcome::Close,
            KeyCode::Enter => self.open(),
            KeyCode::Tab => {
                self.complete();
                Outcome::Keep
            }
            _ => {
                if !self.list.key(&k, self.filtered.len(), 10) && self.input.key(&k) {
                    self.error = None;
                    self.refilter();
                }
                Outcome::Keep
            }
        }
    }

    fn mouse(&mut self, m: MouseEvent, _app: &App) -> Outcome {
        let r = self.list_area;
        match m.kind {
            MouseEventKind::Down(_)
                if m.row >= r.y && m.row < r.bottom() && m.column >= r.x && m.column < r.right() =>
            {
                let i = self.list.offset + (m.row - r.y) as usize;
                if i < self.filtered.len() {
                    self.list.selected = i;
                    self.input = LineInput::default();
                    return self.open();
                }
                Outcome::Keep
            }
            MouseEventKind::ScrollDown => {
                self.list.selected = (self.list.selected + 1).min(self.filtered.len().saturating_sub(1));
                Outcome::Keep
            }
            MouseEventKind::ScrollUp => {
                self.list.selected = self.list.selected.saturating_sub(1);
                Outcome::Keep
            }
            _ => Outcome::Keep,
        }
    }

    fn paste(&mut self, s: &str) {
        self.input.paste(s.trim());
        self.error = None;
        self.refilter();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dialog(root: &Path) -> OpenDialog {
        let mut d = OpenDialog {
            input: LineInput::default(),
            list: ListState::default(),
            all: scan(root),
            filtered: vec![],
            list_area: Rect::default(),
            root: root.to_path_buf(),
            error: None,
        };
        d.refilter();
        d
    }

    #[test]
    fn a_missing_path_opens_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("other.ans"), b"hi").unwrap();
        let mut d = dialog(dir.path());
        let missing = dir.path().join("gone.ans");
        d.input = LineInput::new(&missing.display().to_string());
        d.refilter();
        assert!(matches!(d.open(), Outcome::Keep));
        assert!(d.error.as_deref().is_some_and(|e| e.starts_with("no file at")));
    }

    #[test]
    fn a_file_that_wont_open_says_why_in_the_dialog() {
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join("bad.adf");
        std::fs::write(&bad, b"ADF").unwrap();
        let mut d = dialog(dir.path());
        d.input = LineInput::new(&bad.display().to_string());
        d.refilter();
        assert!(matches!(d.open(), Outcome::Keep));
        assert!(d.error.as_deref().is_some_and(|e| e.starts_with("can't open bad.adf: ")), "{:?}", d.error);
        // A good one goes on to open.
        std::fs::write(dir.path().join("ok.ans"), b"hi").unwrap();
        d.input = LineInput::new(&dir.path().join("ok.ans").display().to_string());
        assert!(matches!(d.open(), Outcome::Then(_)));
    }

    #[test]
    fn a_folder_path_lists_its_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/in.ans"), b"hi").unwrap();
        let mut d = dialog(dir.path());
        d.input = LineInput::new(&dir.path().join("sub").display().to_string());
        assert!(matches!(d.open(), Outcome::Keep));
        assert_eq!(d.root, dir.path().join("sub"));
        assert_eq!(d.filtered.len(), 1);
        assert!(d.error.is_none());
    }

    #[test]
    fn a_filter_with_no_match_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let mut d = dialog(dir.path());
        d.input = LineInput::new("zzz");
        d.refilter();
        assert!(matches!(d.open(), Outcome::Keep));
        assert!(d.error.is_some());
    }
}
