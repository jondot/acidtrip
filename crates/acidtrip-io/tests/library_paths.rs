use acidtrip_io::library::{Paths, write_atomic};

#[test]
fn acidtrip_home_redirects_and_creates_dirs() {
    let tmp = tempfile::tempdir().unwrap();
    // Only test in this binary that touches ACIDTRIP_HOME.
    unsafe { std::env::set_var("ACIDTRIP_HOME", tmp.path()) };
    let p = Paths::resolve().unwrap();
    unsafe { std::env::remove_var("ACIDTRIP_HOME") };
    assert_eq!(p.config_dir, tmp.path().join("config"));
    assert_eq!(p.data_dir, tmp.path().join("data"));
    assert_eq!(p.state_dir, tmp.path().join("state"));
    for d in [
        p.config_dir.clone(),
        p.fonts_dir(),
        p.stencils_dir(),
        p.palettes_dir(),
        p.charsets_dir(),
        p.versions_dir(),
        p.recovery_dir(),
        p.sockets_dir(),
    ] {
        assert!(d.is_dir(), "{} missing", d.display());
    }
    assert_eq!(p.config_file(), tmp.path().join("config/config.toml"));
    assert_eq!(p.fonts_dir(), tmp.path().join("data/library/fonts"));
    assert_eq!(p.recovery_dir(), tmp.path().join("state/recovery"));
}

#[test]
fn write_atomic_replaces_contents() {
    let tmp = tempfile::tempdir().unwrap();
    let f = tmp.path().join("a/b.txt");
    write_atomic(&f, b"one").unwrap();
    write_atomic(&f, b"two").unwrap();
    assert_eq!(std::fs::read(&f).unwrap(), b"two");
    assert_eq!(std::fs::read_dir(tmp.path().join("a")).unwrap().count(), 1);
}
