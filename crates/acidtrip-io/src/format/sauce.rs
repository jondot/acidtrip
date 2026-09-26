//! SAUCE records (via `icy_sauce`) ↔ `DocMeta`.

use acidtrip_core::model::AspectRatio;
use acidtrip_core::{DocMeta, Document, cp437};
use bstr::BString;
use icy_sauce::{SauceDataType, SauceDate, SauceRecord, SauceRecordBuilder, StripMode};

/// SAUCE Character file types.
pub const CHAR_ASCII: u8 = 0;
pub const CHAR_ANSI: u8 = 1;
pub const CHAR_ANSIMATION: u8 = 2;
pub const CHAR_PCBOARD: u8 = 4;
pub const CHAR_AVATAR: u8 = 5;
pub const CHAR_TUNDRA: u8 = 8;

/// Split a file into its content (SAUCE, comments and the EOF byte before
/// them removed) and the parsed record.
pub fn split(data: &[u8]) -> (&[u8], Option<SauceRecord>) {
    match SauceRecord::from_bytes(data) {
        Ok(Some(rec)) => (icy_sauce::strip_sauce(data, StripMode::LastStripFinalEof), Some(rec)),
        _ => (data, None),
    }
}

fn text(b: &[u8]) -> String {
    let s: String = b.iter().map(|&c| cp437::to_char(c)).collect();
    s.trim_end_matches([' ', '\0']).to_string()
}

fn bytes(s: &str, max: usize) -> BString {
    let mut v: Vec<u8> = s.chars().map(cp437::from_char_lossy).collect();
    v.truncate(max);
    BString::from(v)
}

fn is_text(rec: &SauceRecord) -> bool {
    matches!(rec.header().data_type, SauceDataType::Character | SauceDataType::BinaryText | SauceDataType::XBin)
}

/// Copy SAUCE fields into the document metadata.
pub fn apply(rec: &SauceRecord, meta: &mut DocMeta) {
    let h = rec.header();
    let s = &mut meta.sauce;
    s.title = text(&h.title);
    s.author = text(&h.author);
    s.group = text(&h.group);
    s.date =
        if h.date.year > 0 { format!("{:04}{:02}{:02}", h.date.year, h.date.month, h.date.day) } else { String::new() };
    s.comments = rec.comments().iter().map(|c| text(c)).collect();
    s.attach = true;
    if is_text(rec) {
        meta.ice = h.t_flags & 1 != 0;
        meta.letter_spacing_9px = (h.t_flags >> 1) & 3 == 2;
        meta.aspect = if (h.t_flags >> 3) & 3 == 1 { AspectRatio::Legacy } else { AspectRatio::Square };
        let font = text(&h.t_info_s);
        if !font.is_empty() {
            meta.font_name = font;
        }
    }
}

/// Canvas width implied by the record, if any.
pub fn width(rec: Option<&SauceRecord>) -> Option<usize> {
    let h = rec?.header();
    let w = match h.data_type {
        SauceDataType::Character | SauceDataType::XBin => h.t_info1 as usize,
        SauceDataType::BinaryText => h.file_type as usize * 2,
        _ => 0,
    };
    (w > 0).then_some(w)
}

/// Canvas height (in rows) the record declares, if any: TInfo2 of text
/// character files and XBin. RIPscript and the like keep pixels there.
pub fn height(rec: Option<&SauceRecord>) -> Option<usize> {
    let h = rec?.header();
    let rows = match h.data_type {
        SauceDataType::Character
            if matches!(
                h.file_type,
                CHAR_ASCII | CHAR_ANSI | CHAR_ANSIMATION | CHAR_PCBOARD | CHAR_AVATAR | CHAR_TUNDRA
            ) =>
        {
            h.t_info2 as usize
        }
        SauceDataType::XBin => h.t_info2 as usize,
        _ => 0,
    };
    (rows > 0).then_some(rows)
}

/// Rows a loaded text screen has at least: the height the record declares
/// (so blank rows at the bottom come back), else one 25-line screen.
pub fn min_rows(rec: Option<&SauceRecord>) -> usize {
    height(rec).unwrap_or(25).min(20_000)
}

#[derive(Clone, Copy)]
pub enum Kind {
    /// Character data with the given SAUCE file type; TInfo1/2 = width/height.
    Character(u8),
    /// BinaryText: width lives in the file type (width / 2).
    Binary,
    XBin,
}

/// Append a SAUCE record (with its 0x1A EOF marker) to `out`.
pub fn append(out: &mut Vec<u8>, doc: &Document, kind: Kind, width: usize, height: usize) -> anyhow::Result<()> {
    let m = &doc.meta;
    let date = parse_date(&m.sauce.date).unwrap_or_else(|| {
        use chrono::Datelike;
        let now = chrono::Local::now();
        SauceDate::new(now.year(), now.month() as u8, now.day() as u8)
    });
    let spacing = if m.letter_spacing_9px { 2 } else { 1 };
    let aspect = if m.aspect == AspectRatio::Legacy { 1 } else { 2 };
    let flags = u8::from(m.ice) | spacing << 1 | aspect << 3;
    let mut b = SauceRecordBuilder::default()
        .title_truncate(bytes(&m.sauce.title, 35))
        .author_truncate(bytes(&m.sauce.author, 20))
        .group_truncate(bytes(&m.sauce.group, 20))
        .date(date)
        .file_size(out.len() as u32);
    b = match kind {
        Kind::Character(ft) => b
            .data_type(SauceDataType::Character)
            .file_type(ft)
            .t_info1(width.min(u16::MAX as usize) as u16)
            .t_info2(height.min(u16::MAX as usize) as u16),
        Kind::Binary => b.data_type(SauceDataType::BinaryText).file_type((width / 2).min(255) as u8),
        Kind::XBin => b
            .data_type(SauceDataType::XBin)
            .t_info1(width.min(u16::MAX as usize) as u16)
            .t_info2(height.min(u16::MAX as usize) as u16),
    };
    if !matches!(kind, Kind::XBin) {
        b = b.t_flags(flags).t_info_s(bytes(&m.font_name, 22))?;
    }
    for c in m.sauce.comments.iter().take(255) {
        b = b.add_comment(bytes(c, 64))?;
    }
    b.build().write(out)?;
    Ok(())
}

fn parse_date(s: &str) -> Option<SauceDate> {
    if s.len() != 8 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(SauceDate::new(s[0..4].parse().ok()?, s[4..6].parse().ok()?, s[6..8].parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use acidtrip_core::DocKind;

    #[test]
    fn roundtrip_meta() {
        let mut d = Document::new(DocKind::Classic, 80, 1);
        d.meta.sauce.title = "Test ░".into();
        d.meta.sauce.author = "jondot".into();
        d.meta.sauce.group = "ACiD".into();
        d.meta.sauce.date = "19960314".into();
        d.meta.sauce.comments = vec!["hello".into(), "world".into()];
        d.meta.letter_spacing_9px = true;
        d.meta.aspect = AspectRatio::Legacy;
        let mut out = b"hi".to_vec();
        append(&mut out, &d, Kind::Character(CHAR_ANSI), 132, 50).unwrap();
        let (body, rec) = split(&out);
        assert_eq!(body, b"hi");
        let rec = rec.unwrap();
        assert_eq!(width(Some(&rec)), Some(132));
        assert_eq!(height(Some(&rec)), Some(50));
        let mut m = DocMeta { ice: false, ..DocMeta::default() };
        apply(&rec, &mut m);
        assert_eq!(m.sauce.title, "Test ░");
        assert_eq!(m.sauce.date, "19960314");
        assert_eq!(m.sauce.comments, vec!["hello", "world"]);
        assert!(m.ice && m.letter_spacing_9px && m.sauce.attach);
        assert_eq!(m.aspect, AspectRatio::Legacy);
        assert_eq!(m.font_name, "IBM VGA");
    }
}
