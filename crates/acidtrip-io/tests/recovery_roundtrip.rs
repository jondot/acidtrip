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
    recovery::write(dir, &a, Some(std::path::Path::new("/art/a.ans")), None).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(5));
    recovery::write(dir, &b, None, None).unwrap();
    let l = recovery::list(dir);
    assert_eq!(l.len(), 2);
    assert_eq!(l[0].doc_id, b.meta.id, "newest first");
    assert_eq!(l[1].file.as_deref(), Some(std::path::Path::new("/art/a.ans")));
    assert!(chrono::DateTime::parse_from_rfc3339(&l[0].saved_at).is_ok());
    assert_eq!(recovery::load(&l[1]).unwrap(), a);

    // Rewriting replaces, not duplicates.
    let mut a2 = a.clone();
    a2.canvas.layers[0].cells[1] = Some(Cell::new('Z', Color::WHITE, Color::BLACK));
    recovery::write(dir, &a2, None, None).unwrap();
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

#[test]
fn keeps_the_edit_log() {
    use acidtrip_core::TxBuilder;
    use acidtrip_core::replay::{EditLog, LogOp};
    let tmp = tempfile::tempdir().unwrap();
    let mut d = doc('A');
    let mut log = EditLog::new();
    for x in 1..4 {
        let mut b = TxBuilder::new(&d, "put");
        b.set(0, x, 0, Some(Cell::new('#', Color::WHITE, Color::BLACK)));
        let tx = b.finish();
        log.record_at(x as u64 * 100, &d, LogOp::Commit, tx.clone());
        d.apply(&tx);
    }
    recovery::write(tmp.path(), &d, None, Some(&log)).unwrap();
    let e = &recovery::list(tmp.path())[0];
    let (back, got) = recovery::load_with_log(e).unwrap();
    assert_eq!(back, d);
    assert_eq!(got.unwrap().entries(), log.entries());
    assert_eq!(recovery::load(e).unwrap(), d);

    // Recovery files from before the log was kept still load.
    std::fs::write(&e.path, acidtrip_io::native::to_bytes(&d).unwrap()).unwrap();
    let (back, got) = recovery::load_with_log(e).unwrap();
    assert_eq!(back, d);
    assert!(got.is_none());
}
