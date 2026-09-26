//! The gallery's data: scene art from 16colo.rs (packs by year, groups,
//! artists, search) and from local folders, as [`Piece`]s that carry their
//! credits and load as documents. Your saved collection and recently opened
//! pieces live in [`Shelf`].
//!
//! API replies are cached under the harvest cache and reused for a day, so
//! browsing is fast and works offline for anything seen before.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::harvest::{self, PackInfo, SIXTEEN_API, SourceArt, http_get, json_text, json_texts, sanitize};

/// How long a cached API reply is used before asking again.
const FRESH: Duration = Duration::from_secs(24 * 3600);

/// Groups on the home screen, famous first: (16colo.rs tag, name).
pub const FEATURED_GROUPS: [(&str, &str); 10] = [
    ("acid", "ACiD Productions"),
    ("ice", "iCE Advertisements"),
    ("blocktronics", "Blocktronics"),
    ("fire", "Fire"),
    ("mistigris", "Mistigris"),
    ("dark", "Dark"),
    ("cia", "CiA"),
    ("legend design", "Legend Design"),
    ("remorse", "Remorse"),
    ("impure", "Impure"),
];

/// Where a piece comes from.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Origin {
    /// A file in a 16colo.rs pack.
    Pack { pack: String, file: String },
    /// A file on disk.
    Local(PathBuf),
}

/// One piece of art, with what's known about it before it loads.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Piece {
    pub origin: Origin,
    /// Title (the site's index, SAUCE, or the file name).
    pub title: String,
    pub artists: Vec<String>,
    pub group: String,
    pub year: u32,
}

impl Piece {
    pub fn file(&self) -> String {
        match &self.origin {
            Origin::Pack { file, .. } => file.clone(),
            Origin::Local(p) => p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        }
    }

    /// What [`harvest::fetch`] and the sourcing studio take.
    pub fn source(&self) -> String {
        match &self.origin {
            Origin::Pack { pack, file } => format!("https://16colo.rs/pack/{pack}/{file}"),
            Origin::Local(p) => p.to_string_lossy().into_owned(),
        }
    }

    /// A stable key for caches.
    pub fn key(&self) -> String {
        self.source()
    }

    /// "by artist / group, year" for cards.
    pub fn byline(&self) -> String {
        let mut s = String::new();
        if !self.artists.is_empty() {
            s.push_str(&self.artists.join(", "));
        }
        if !self.group.is_empty() {
            if !s.is_empty() {
                s.push_str(" / ");
            }
            s.push_str(&self.group);
        }
        if self.year > 0 {
            if !s.is_empty() {
                s.push_str(", ");
            }
            s.push_str(&self.year.to_string());
        }
        s
    }
}

/// A group, as the site lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub name: String,
    pub longname: String,
    pub releases: usize,
}

/// An artist and how many releases they're in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Artist {
    pub name: String,
    pub releases: usize,
}

/// Cached API GET: a copy younger than [`FRESH`] is used as is; otherwise
/// ask the site, falling back to any old copy when offline.
fn api(url: &str, cache: &Path, name: &str) -> Result<Vec<u8>> {
    let path = cache.join("api").join(sanitize(name));
    let fresh = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .is_ok_and(|t| SystemTime::now().duration_since(t).unwrap_or_default() < FRESH);
    if fresh && let Ok(b) = std::fs::read(&path) {
        return Ok(b);
    }
    match http_get(url) {
        Ok(b) => {
            std::fs::create_dir_all(path.parent().unwrap_or(cache)).ok();
            let _ = std::fs::write(&path, &b);
            Ok(b)
        }
        Err(e) => std::fs::read(&path).ok().filter(|b| !b.is_empty()).ok_or(e),
    }
}

/// Only characters the API's path segments and filters take.
fn slug(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')).collect()
}

/// The art files of a pack, with titles and artists.
pub fn pack_pieces(pack: &str, cache: &Path) -> Result<Vec<Piece>> {
    let body = api(&format!("{SIXTEEN_API}/pack/{}", slug(pack)), cache, &format!("pack-{pack}.json"))?;
    let d = harvest::parse_pack_detail(pack, &body)?;
    Ok(d.files
        .into_iter()
        .filter(|f| harvest::is_art_file(&f.name))
        .map(|f| Piece {
            title: f.content.first().cloned().filter(|t| !t.trim().is_empty()).unwrap_or_else(|| f.name.clone()),
            origin: Origin::Pack { pack: pack.to_string(), file: f.name },
            artists: f.artists,
            group: String::new(),
            year: d.year,
        })
        .collect())
}

/// The piece that best shows off a pack: real color art (ANSI, XBin, …)
/// before text, and not the pack's info files.
pub fn poster(pieces: &[Piece]) -> Option<&Piece> {
    let score = |p: &Piece| {
        let f = p.file().to_lowercase();
        let ext = f.rsplit('.').next().unwrap_or("");
        let art = matches!(ext, "ans" | "xb" | "bin" | "adf" | "idf" | "pcb" | "tnd" | "ice" | "avt");
        let info = ["file_id", "news", "info", "member", "list", "readme", "apply", "join", "stat"]
            .iter()
            .any(|w| f.contains(w));
        i32::from(art) * 2 - i32::from(info)
    };
    pieces.iter().max_by_key(|p| (score(p), std::cmp::Reverse(p.file())))
}

/// Every pack released in `year`.
pub fn year_packs(year: u32, cache: &Path) -> Result<Vec<PackInfo>> {
    let url = format!("{SIXTEEN_API}/year/{year}?pagesize=500&page=1");
    Ok(harvest::parse_pack_page(&api(&url, cache, &format!("year-{year}-1.json"))?)?.0)
}

/// Years with packs, newest first.
pub fn years(cache: &Path) -> Result<Vec<u32>> {
    let mut y: Vec<u32> = harvest::parse_years(&api(&format!("{SIXTEEN_API}/year"), cache, "years.json")?)?
        .into_iter()
        .map(|y| y.year)
        .collect();
    y.reverse();
    Ok(y)
}

/// `GET /v1/group?filter=`: `{"results": [{"acid": {"releases": 152, "longname": …}}, …]}`.
pub fn parse_groups(json: &[u8]) -> Result<Vec<Group>> {
    let v: Value = serde_json::from_slice(json).context("16colo.rs group list")?;
    Ok(v["results"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| r.as_object()?.iter().next().map(|(k, g)| (k.clone(), g.clone())))
        .map(|(name, g)| Group {
            longname: json_text(&g["longname"]).unwrap_or_else(|| name.clone()),
            releases: g["releases"].as_u64().unwrap_or(0) as usize,
            name,
        })
        .collect())
}

pub fn groups(filter: &str, cache: &Path) -> Result<Vec<Group>> {
    let q = slug(filter);
    parse_groups(&api(&format!("{SIXTEEN_API}/group?filter={q}&pagesize=100"), cache, &format!("groups-{q}.json"))?)
}

/// `GET /v1/group/<g>`: `{"results": {"packs": {"1996": ["pack", …]}}}`, newest first.
pub fn parse_group_packs(json: &[u8], group: &str) -> Result<Vec<PackInfo>> {
    let v: Value = serde_json::from_slice(json).context("16colo.rs group")?;
    let mut out: Vec<PackInfo> = v["results"]["packs"]
        .as_object()
        .into_iter()
        .flatten()
        .flat_map(|(y, packs)| {
            let year = y.parse().unwrap_or(0);
            json_texts(packs).into_iter().map(move |name| PackInfo { name, year, groups: vec![group.to_string()] })
        })
        .collect();
    out.sort_by(|a, b| b.year.cmp(&a.year).then(a.name.cmp(&b.name)));
    Ok(out)
}

pub fn group_packs(group: &str, cache: &Path) -> Result<Vec<PackInfo>> {
    let g = group.replace(' ', "%20");
    let body = api(&format!("{SIXTEEN_API}/group/{g}"), cache, &format!("group-{group}.json"))?;
    parse_group_packs(&body, group)
}

/// `GET /v1/artist?filter=`: `{"results": [{"artist": {"name": …, "releases": n}}]}`.
pub fn parse_artists(json: &[u8]) -> Result<Vec<Artist>> {
    let v: Value = serde_json::from_slice(json).context("16colo.rs artist list")?;
    Ok(v["results"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| {
            let a = &r["artist"];
            Some(Artist { name: json_text(&a["name"])?, releases: a["releases"].as_u64().unwrap_or(0) as usize })
        })
        .collect())
}

pub fn artists(filter: &str, cache: &Path) -> Result<Vec<Artist>> {
    let q = slug(filter);
    parse_artists(&api(&format!("{SIXTEEN_API}/artist?filter={q}&pagesize=100"), cache, &format!("artists-{q}.json"))?)
}

/// `GET /v1/artist/<a>`: `{"results": {"1995": {"pack": {"group": g, "files": [{"file": f}]}}}}`.
pub fn parse_artist_pieces(json: &[u8], artist: &str) -> Result<Vec<Piece>> {
    let v: Value = serde_json::from_slice(json).context("16colo.rs artist")?;
    let mut out = vec![];
    for (y, packs) in v["results"].as_object().into_iter().flatten() {
        for (pack, p) in packs.as_object().into_iter().flatten() {
            for f in p["files"].as_array().into_iter().flatten() {
                let Some(file) = json_text(&f["file"]) else { continue };
                if !harvest::is_art_file(&file) {
                    continue;
                }
                out.push(Piece {
                    title: file.clone(),
                    origin: Origin::Pack { pack: pack.clone(), file },
                    artists: vec![artist.to_string()],
                    group: json_text(&p["group"]).unwrap_or_default(),
                    year: y.parse().unwrap_or(0),
                });
            }
        }
    }
    out.sort_by(|a, b| b.year.cmp(&a.year).then(a.file().cmp(&b.file())));
    Ok(out)
}

pub fn artist_pieces(artist: &str, cache: &Path) -> Result<Vec<Piece>> {
    let a = slug(&artist.replace(' ', "_"));
    let body = api(&format!("{SIXTEEN_API}/artist/{a}"), cache, &format!("artist-{a}.json"))?;
    let out = parse_artist_pieces(&body, artist)?;
    if out.is_empty() {
        return Err(anyhow!("16colo.rs has no artist page for {artist:?}"));
    }
    Ok(out)
}

/// Packs whose name matches.
pub fn search_packs(q: &str, cache: &Path) -> Result<Vec<PackInfo>> {
    harvest::sixteen_colors_search(q, cache)
}

fn cache_path(cache: &Path, pack: &str, file: &str) -> PathBuf {
    cache.join("gallery").join(sanitize(pack)).join(sanitize(file))
}

/// A piece's file if it's on disk (local, or downloaded before); never
/// touches the network.
pub fn cached_bytes(piece: &Piece, cache: &Path) -> Option<Vec<u8>> {
    let b = match &piece.origin {
        Origin::Local(p) => std::fs::read(p).ok()?,
        Origin::Pack { pack, file } => std::fs::read(cache_path(cache, pack, file)).ok()?,
    };
    (!b.is_empty()).then_some(b)
}

/// A piece's art if its file is on disk (see [`cached_bytes`]).
pub fn cached_doc(piece: &Piece, cache: &Path) -> Option<acidtrip_core::Document> {
    let b = cached_bytes(piece, cache)?;
    harvest::parse_art(&piece.file(), &b, harvest::Attribution::default()).ok().map(|a| a.doc)
}

/// A piece's file: read from disk, or downloaded once into the cache.
pub fn bytes(piece: &Piece, cache: &Path) -> Result<Vec<u8>> {
    match &piece.origin {
        Origin::Local(p) => std::fs::read(p).with_context(|| format!("reading {}", p.display())),
        Origin::Pack { pack, file } => {
            if let Some(b) = cached_bytes(piece, cache) {
                return Ok(b);
            }
            let path = cache_path(cache, pack, file);
            let b = http_get(&format!("https://16colo.rs/pack/{pack}/raw/{file}"))?;
            std::fs::create_dir_all(path.parent().unwrap_or(cache)).ok();
            let _ = std::fs::write(&path, &b);
            Ok(b)
        }
    }
}

/// Load a piece: its art (credits from the site, then SAUCE) and its bytes.
pub fn load(piece: &Piece, cache: &Path) -> Result<(SourceArt, Vec<u8>)> {
    let b = bytes(piece, cache)?;
    let (pack, source) = match &piece.origin {
        Origin::Pack { pack, .. } => (pack.clone(), piece.source()),
        Origin::Local(p) => (String::new(), p.to_string_lossy().into_owned()),
    };
    let a = harvest::Attribution {
        author: piece.artists.first().cloned().unwrap_or_default(),
        group: piece.group.clone(),
        title: if piece.title == piece.file() { String::new() } else { piece.title.clone() },
        pack,
        file: piece.file(),
        source,
    };
    Ok((harvest::parse_art(&piece.file(), &b, a)?, b))
}

/// Art files under a folder (recursive), titled by file name.
pub fn local_pieces(dir: &Path) -> Vec<Piece> {
    let mut out = vec![];
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                stack.push(p);
            } else if p.file_name().is_some_and(|n| harvest::is_art_file(&n.to_string_lossy())) && out.len() < 2000 {
                let title = p.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                out.push(Piece { title, origin: Origin::Local(p), artists: vec![], group: String::new(), year: 0 });
            }
        }
    }
    out
}

/// Your gallery state: recently opened pieces, the saved collection (copies
/// kept in the library, so they work offline), and local folders to browse.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Shelf {
    #[serde(default)]
    pub recent: Vec<Piece>,
    #[serde(default)]
    pub saved: Vec<Piece>,
    #[serde(default)]
    pub folders: Vec<PathBuf>,
}

const RECENT_MAX: usize = 24;

impl Shelf {
    pub fn load(path: &Path) -> Shelf {
        std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::write(path, serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }

    pub fn opened(&mut self, p: &Piece) {
        self.recent.retain(|r| r != p);
        self.recent.insert(0, p.clone());
        self.recent.truncate(RECENT_MAX);
    }

    pub fn is_saved(&self, p: &Piece) -> bool {
        self.saved.iter().any(|s| {
            s.key() == p.key()
                || matches!(&s.origin, Origin::Local(_))
                    && s.title == p.title
                    && s.artists == p.artists
                    && s.year == p.year
        })
    }

    /// Keep a copy of `art` in `dir` (the collection folder) and list it.
    pub fn keep(&mut self, p: &Piece, art_bytes: &[u8], dir: &Path) -> Result<Piece> {
        std::fs::create_dir_all(dir)?;
        let name = match &p.origin {
            Origin::Pack { pack, file } => format!("{}-{}", sanitize(pack), sanitize(file)),
            Origin::Local(_) => sanitize(&p.file()),
        };
        let path = dir.join(name);
        std::fs::write(&path, art_bytes).with_context(|| format!("writing {}", path.display()))?;
        let kept = Piece { origin: Origin::Local(path), ..p.clone() };
        self.saved.retain(|s| s.key() != kept.key());
        self.saved.insert(0, kept.clone());
        Ok(kept)
    }

    pub fn forget(&mut self, p: &Piece) {
        self.saved.retain(|s| s.key() != p.key());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_groups_packs_and_artists() {
        let g = br#"{"page":{},"results":[{"acid":{"releases":152,"longname":"ACiD Productions"}},{"acid rain":{"releases":0}}]}"#;
        let gs = parse_groups(g).unwrap();
        assert_eq!(gs[0], Group { name: "acid".into(), longname: "ACiD Productions".into(), releases: 152 });
        assert_eq!(gs[1].longname, "acid rain");
        let gp = br#"{"results":{"packs":{"1993":["acdu0193","acdu0293"],"2001":["acid-93"]}}}"#;
        let packs = parse_group_packs(gp, "acid").unwrap();
        assert_eq!(packs.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["acid-93", "acdu0193", "acdu0293"]);
        assert_eq!(packs[0].year, 2001);
        let a = br#"{"results":[{"artist":{"name":"lord jazz","releases":431}},{"artist":{"name":111,"releases":1}}]}"#;
        assert_eq!(parse_artists(a).unwrap()[1], Artist { name: "111".into(), releases: 1 });
    }

    #[test]
    fn parses_an_artists_pieces_newest_first() {
        let j = br#"{"results":{"1995":{"ane-1295":{"group":"anemia","files":[{"file":"CG-GE.ANS"},{"file":"INSTALL.EXE"}]}},
                     "1997":{"twi-9703":{"group":"twilight","files":[{"file":"CG-MALP.ANS"}]}}}}"#;
        let p = parse_artist_pieces(j, "coug").unwrap();
        assert_eq!(p.len(), 2, "only art files");
        assert_eq!(p[0].origin, Origin::Pack { pack: "twi-9703".into(), file: "CG-MALP.ANS".into() });
        assert_eq!(p[0].byline(), "coug / twilight, 1997");
        assert_eq!(p[0].source(), "https://16colo.rs/pack/twi-9703/CG-MALP.ANS");
    }

    #[test]
    fn a_packs_poster_is_its_art_not_its_info_files() {
        let piece = |f: &str| Piece {
            origin: Origin::Pack { pack: "p".into(), file: f.into() },
            title: f.into(),
            artists: vec![],
            group: String::new(),
            year: 0,
        };
        let v = ["ACID-NEWS.ANS", "FILE_ID.DIZ", "RM-FOO.ASC", "RM-LOGO.ANS", "ZZ-LAST.ANS"].map(piece);
        assert_eq!(poster(&v).unwrap().file(), "RM-LOGO.ANS");
    }

    #[test]
    fn the_shelf_remembers_and_keeps_copies() {
        let dir = tempfile::tempdir().unwrap();
        let piece = Piece {
            origin: Origin::Pack { pack: "twi-9703".into(), file: "CG-MALP.ANS".into() },
            title: "Malpractice".into(),
            artists: vec!["coug".into()],
            group: "twilight".into(),
            year: 1997,
        };
        let mut s = Shelf::default();
        s.opened(&piece);
        s.opened(&piece);
        assert_eq!(s.recent.len(), 1);
        let kept = s.keep(&piece, b"\x1b[0mhi", &dir.path().join("collection")).unwrap();
        assert!(matches!(&kept.origin, Origin::Local(p) if p.exists()));
        assert!(s.is_saved(&piece), "the pack piece reads as saved");
        let path = dir.path().join("gallery.json");
        s.save(&path).unwrap();
        assert_eq!(Shelf::load(&path), s);
        s.forget(&kept);
        assert!(s.saved.is_empty());
        assert_eq!(local_pieces(&dir.path().join("collection")).len(), 1);
    }
}
