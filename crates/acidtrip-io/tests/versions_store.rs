use acidtrip_core::{Cell, Color, DocKind, Document};
use acidtrip_io::versions::{VersionStore, preview};

fn doc_with(text: &str) -> Document {
    let mut d = Document::new(DocKind::Classic, 90, 10);
    for (y, line) in text.lines().enumerate() {
        for (x, ch) in line.chars().enumerate() {
            d.canvas.layers[0].cells[y * 90 + x] = Some(Cell::new(ch, Color::WHITE, Color::BLACK));
        }
    }
    d
}

#[test]
fn snapshot_dedupe_order_rename_load() {
    let tmp = tempfile::tempdir().unwrap();
    let base = doc_with("one");
    let id = base.meta.id;
    let mut s = VersionStore::open(tmp.path(), id).unwrap();
    assert!(s.list().is_empty());

    let v1 = s.snapshot(&base, "save", Some(std::path::Path::new("/x/a.ans"))).unwrap();
    let mut d2 = base.clone();
    d2.canvas.layers[0].cells[0] = Some(Cell::new('T', Color::WHITE, Color::BLACK));
    let v2 = s.snapshot(&d2, "auto", None).unwrap();
    assert_ne!(v1.hash, v2.hash);
    assert_eq!(v1.hash.len(), 64);
    assert_eq!(s.list().iter().map(|v| v.hash.clone()).collect::<Vec<_>>(), vec![v2.hash.clone(), v1.hash.clone()]);

    // Identical content: same entry, label appended, moved to newest.
    let v1b = s.snapshot(&base, "auto", None).unwrap();
    assert_eq!(v1b.hash, v1.hash);
    assert_eq!(v1b.label, "save, auto");
    assert_eq!(v1b.file.as_deref(), Some("/x/a.ans"));
    let l = s.list();
    assert_eq!(l.len(), 2);
    assert_eq!(l[0].hash, v1.hash);
    // Repeating a label doesn't grow it.
    assert_eq!(s.snapshot(&base, "auto", None).unwrap().label, "save, auto");
    assert_eq!(std::fs::read_dir(tmp.path().join(id.to_string())).unwrap().count(), 3); // 2 docs + index

    s.rename(&v2.hash, "before the logo").unwrap();
    assert!(s.rename("deadbeef", "x").is_err());

    // Persisted across reopen.
    let s2 = VersionStore::open(tmp.path(), id).unwrap();
    assert_eq!(s2.list(), s.list());
    assert_eq!(s2.list()[1].label, "before the logo");
    assert_eq!(s2.load(&v2.hash).unwrap(), d2);
    assert_eq!(s2.load(&v1.hash).unwrap(), base);
    assert!(s2.load("../../etc").is_err());
    assert!(s2.load(&"0".repeat(64)).is_err());
}

#[test]
fn file_doc_id_is_stable_per_file() {
    use acidtrip_io::versions::file_doc_id;
    let tmp = tempfile::tempdir().unwrap();
    let a = tmp.path().join("a.ans");
    // Same id before the file exists and after it's written.
    let before = file_doc_id(&a);
    std::fs::write(&a, b"x").unwrap();
    assert_eq!(file_doc_id(&a), before);
    // The folder may be reached through a symlink (/tmp on macOS).
    let canon = tmp.path().canonicalize().unwrap().join("a.ans");
    assert_eq!(file_doc_id(&canon), before);
    assert_ne!(file_doc_id(&tmp.path().join("b.ans")), before);
}

#[test]
fn preview_takes_first_non_blank_rows_trimmed() {
    let long = "x".repeat(88);
    let text = format!("\nhello\n\n  world  \n{long}\n3\n4\n5\n6");
    let p = preview(&doc_with(&text));
    let rows: Vec<&str> = p.lines().collect();
    assert_eq!(rows.len(), 6);
    assert_eq!(rows[0], "hello");
    assert_eq!(rows[1], "  world");
    assert_eq!(rows[2].chars().count(), 80);
    assert_eq!(rows[5], "5");
    assert_eq!(preview(&Document::new(DocKind::Classic, 4, 4)), "");
}
