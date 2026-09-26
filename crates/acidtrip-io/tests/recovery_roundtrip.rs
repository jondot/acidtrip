use acidtrip_core::{Cell, Color, DocKind, Document};
use acidtrip_io::recovery;

fn doc(ch: char) -> Document {
    let mut d = Document::new(DocKind::Classic, 8, 3);
    d.canvas.layers[0].cells[0] = Some(Cell::new(ch, Color::Pal(12), Color::BLACK));
    d
}

#[test]
fn write_list_load_clear() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    assert!(recovery::list(dir).is_empty());
    let a = doc('A');
    let b = doc('B');
    recovery::write(dir, &a, Some(std::path::Path::new("/art/a.ans"))).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(5));
    recovery::write(dir, &b, None).unwrap();
    let l = recovery::list(dir);
    assert_eq!(l.len(), 2);
    assert_eq!(l[0].doc_id, b.meta.id, "newest first");
    assert_eq!(l[1].file.as_deref(), Some(std::path::Path::new("/art/a.ans")));
    assert!(chrono::DateTime::parse_from_rfc3339(&l[0].saved_at).is_ok());
    assert_eq!(recovery::load(&l[1]).unwrap(), a);

    // Rewriting replaces, not duplicates.
    let mut a2 = a.clone();
    a2.canvas.layers[0].cells[1] = Some(Cell::new('Z', Color::WHITE, Color::BLACK));
    recovery::write(dir, &a2, None).unwrap();
    let l = recovery::list(dir);
    assert_eq!(l.len(), 2);
    assert_eq!(recovery::load(l.iter().find(|e| e.doc_id == a.meta.id).unwrap()).unwrap(), a2);

    recovery::clear(dir, a.meta.id);
    let l = recovery::list(dir);
    assert_eq!(l.len(), 1);
    assert_eq!(l[0].doc_id, b.meta.id);
    recovery::clear(dir, a.meta.id); // idempotent
    // No temp files left behind.
    assert_eq!(std::fs::read_dir(dir).unwrap().count(), 2);
}

#[test]
fn list_skips_garbage() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("junk.json"), "{nope").unwrap();
    std::fs::write(tmp.path().join("x.txt"), "hi").unwrap();
    assert!(recovery::list(tmp.path()).is_empty());
    assert!(recovery::list(&tmp.path().join("missing")).is_empty());
}
