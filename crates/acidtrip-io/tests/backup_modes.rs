use acidtrip_io::backup::{BackupMode, backup_before_save};

#[test]
fn none_and_missing_file_do_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let f = tmp.path().join("art.ans");
    assert_eq!(backup_before_save(&f, BackupMode::Bak).unwrap(), None);
    std::fs::write(&f, "x").unwrap();
    assert_eq!(backup_before_save(&f, BackupMode::None).unwrap(), None);
    assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 1);
}

#[test]
fn bak_overwrites_previous_bak() {
    let tmp = tempfile::tempdir().unwrap();
    let f = tmp.path().join("art.ans");
    std::fs::write(&f, "v1").unwrap();
    let b = backup_before_save(&f, BackupMode::Bak).unwrap().unwrap();
    assert_eq!(b, tmp.path().join("art.ans.bak"));
    std::fs::write(&f, "v2").unwrap();
    backup_before_save(&f, BackupMode::Bak).unwrap();
    assert_eq!(std::fs::read_to_string(&b).unwrap(), "v2");
}

#[test]
fn numbered_uses_next_free_and_errors_when_full() {
    let tmp = tempfile::tempdir().unwrap();
    let f = tmp.path().join("art.ans");
    std::fs::write(&f, "v1").unwrap();
    assert_eq!(backup_before_save(&f, BackupMode::Numbered).unwrap().unwrap(), tmp.path().join("art.ans.001"));
    std::fs::write(&f, "v2").unwrap();
    assert_eq!(backup_before_save(&f, BackupMode::Numbered).unwrap().unwrap(), tmp.path().join("art.ans.002"));
    assert_eq!(std::fs::read_to_string(tmp.path().join("art.ans.001")).unwrap(), "v1");
    std::fs::remove_file(tmp.path().join("art.ans.001")).unwrap();
    assert_eq!(backup_before_save(&f, BackupMode::Numbered).unwrap().unwrap(), tmp.path().join("art.ans.001"));
    for n in 1..=999 {
        std::fs::write(tmp.path().join(format!("art.ans.{n:03}")), "").unwrap();
    }
    assert!(backup_before_save(&f, BackupMode::Numbered).is_err());
}
