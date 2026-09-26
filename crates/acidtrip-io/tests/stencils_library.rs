use acidtrip_core::{Cell, Clip, Color};
use acidtrip_io::stencils::{Stencil, StencilLibrary, StencilMeta, slug};

fn stencil(name: &str, tags: &[&str], author: &str) -> Stencil {
    let mut clip = Clip::new(3, 1);
    clip.set(1, 0, Some(Cell::new('▓', Color::Pal(9), Color::BLACK)));
    Stencil {
        meta: StencilMeta {
            name: name.into(),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            author: author.into(),
            source: "drawn".into(),
            ..Default::default()
        },
        clip,
    }
}

#[test]
fn builtins_present_and_undeletable() {
    let tmp = tempfile::tempdir().unwrap();
    let mut lib = StencilLibrary::load(tmp.path());
    let list = lib.list();
    assert!(list.len() >= 12);
    for m in &list {
        assert_eq!(m.source, "built-in");
        assert_eq!(m.license, "MIT");
        let s = lib.get(&m.id).unwrap();
        assert!(s.clip.width > 0 && s.clip.height > 0);
        assert_eq!(s.clip.cells.len(), s.clip.width * s.clip.height);
        assert!(s.clip.cells.iter().any(Option::is_some), "{} is empty", m.id);
    }
    let heart = lib.get("builtin-heart").unwrap();
    assert!(heart.clip.cells.iter().flatten().all(|c| "▀▄█".contains(c.ch)));
    let frame = lib.get("builtin-box-single").unwrap();
    assert_eq!(frame.clip.get(1, 1), None, "frame interior is transparent");
    assert!(lib.delete(tmp.path(), "builtin-heart").is_err());
    assert!(lib.get("builtin-heart").is_some());
}

#[test]
fn save_load_delete() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let mut lib = StencilLibrary::load(dir);
    let n0 = lib.list().len();
    let m = lib.save(dir, stencil("Cool Logo #1!", &["logo"], "jondot")).unwrap();
    assert!(m.id.starts_with("cool-logo-1-"), "{}", m.id);
    assert_eq!(m.id.len(), "cool-logo-1-".len() + 6);
    assert!(!m.created.is_empty());
    assert!(dir.join(format!("{}.stencil.json.zst", m.id)).is_file());

    // Resave with the same id updates in place.
    let mut s = lib.get(&m.id).unwrap();
    s.meta.tags.push("red".into());
    let m2 = lib.save(dir, s).unwrap();
    assert_eq!(m2.id, m.id);
    assert_eq!(lib.list().len(), n0 + 1);

    let reloaded = StencilLibrary::load(dir);
    let got = reloaded.get(&m.id).unwrap();
    assert_eq!(got.meta.tags, vec!["logo", "red"]);
    assert_eq!(got.clip, stencil("x", &[], "").clip);
    assert_eq!(reloaded.list().len(), n0 + 1);

    // Saving over a built-in makes a user copy.
    let mut b = lib.get("builtin-star").unwrap();
    b.meta.name = "My star".into();
    let bm = lib.save(dir, b).unwrap();
    assert!(bm.id.starts_with("my-star-"));

    lib.delete(dir, &m.id).unwrap();
    assert!(lib.get(&m.id).is_none());
    assert!(!dir.join(format!("{}.stencil.json.zst", m.id)).exists());
    assert!(lib.delete(dir, &m.id).is_err());
    assert!(StencilLibrary::load(dir).get(&m.id).is_none());
}

#[test]
fn search_ranks_and_filters() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let mut lib = StencilLibrary::load(dir);
    lib.save(dir, stencil("Skull", &["horror", "logo"], "Lord Jazz")).unwrap();
    lib.save(dir, stencil("Big skull logo", &["horror"], "somebody")).unwrap();
    lib.save(dir, stencil("Dragon", &["fantasy"], "Lord Jazz")).unwrap();

    let names = |q: &str| lib.search(q).into_iter().map(|m| m.name).collect::<Vec<_>>();
    assert_eq!(names("skull")[..2], ["Skull", "Big skull logo"]);
    assert_eq!(names("SKULL horror"), ["Skull", "Big skull logo"]);
    assert_eq!(names("jazz"), ["Dragon", "Skull"]);
    assert_eq!(names("drgn"), ["Dragon"], "subsequence match");
    assert!(names("heart").contains(&"Half-block heart".to_string()));
    assert!(names("frame").contains(&"Double box".to_string()), "tag match");
    assert!(names("zzzzqq").is_empty());
    assert_eq!(lib.search("  ").len(), lib.list().len());
}

#[test]
fn slugs() {
    assert_eq!(slug("  Hello, World!! "), "hello-world");
    assert_eq!(slug("ÄÖ"), "stencil");
    assert_eq!(slug("a/b\\c"), "a-b-c");
}
