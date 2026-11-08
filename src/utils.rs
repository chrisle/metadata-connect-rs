use crate::ArtworkMimeType;
#[path = "genres.rs"]
mod genres;
pub(crate) use genres::GENRES;
pub fn detect_image_type(data: &[u8]) -> Option<ArtworkMimeType> {
    if data.len() < 4 {
        return None;
    }
    if data.starts_with(&[255, 216, 255]) {
        Some(ArtworkMimeType::Jpeg)
    } else if data.starts_with(b"\x89PNG") {
        Some(ArtworkMimeType::Png)
    } else if data.starts_with(b"GIF") {
        Some(ArtworkMimeType::Gif)
    } else {
        None
    }
}
pub fn normalize_mime_type(mime: &str) -> ArtworkMimeType {
    let lower = mime.to_lowercase();
    if lower.contains("png") {
        ArtworkMimeType::Png
    } else if lower.contains("gif") {
        ArtworkMimeType::Gif
    } else {
        ArtworkMimeType::Jpeg
    }
}
pub fn clean_text(text: &str) -> Option<String> {
    let text = text.replace('\0', "");
    let text = text.trim_matches(|c: char| matches!(c, '\u{0009}'..='\u{000d}' | '\u{0020}' | '\u{00a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}'));
    (!text.is_empty()).then(|| text.to_owned())
}
pub fn parse_bpm(text: &str) -> Option<f64> {
    let start = text.find(|c: char| c.is_ascii_digit() || c == '.')?;
    let segment: String = text[start..]
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let mut dot = false;
    let prefix: String = segment
        .chars()
        .take_while(|c| {
            if *c == '.' {
                if dot {
                    false
                } else {
                    dot = true;
                    true
                }
            } else {
                true
            }
        })
        .collect();
    let n = prefix.parse::<f64>().ok()?;
    (n > 0.0 && n <= 500.0).then(|| (n * 10.0).round() / 10.0)
}
pub fn parse_year(text: &str) -> Option<u32> {
    let year = year_any(text)?;
    (1900..=2100).contains(&year).then_some(year)
}
pub(crate) fn year_any(text: &str) -> Option<u32> {
    text.as_bytes()
        .windows(4)
        .find(|b| b.iter().all(u8::is_ascii_digit))
        .and_then(|b| std::str::from_utf8(b).ok()?.parse().ok())
}
pub(crate) fn syncsafe(data: &[u8]) -> u32 {
    data.iter()
        .take(4)
        .fold(0, |n, b| (n << 7) | u32::from(b & 127))
}
pub(crate) fn be32(data: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_be_bytes(
        data.get(offset..offset.checked_add(4)?)?.try_into().ok()?,
    ))
}
pub(crate) fn le32(data: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        data.get(offset..offset.checked_add(4)?)?.try_into().ok()?,
    ))
}
pub(crate) fn text(data: &[u8], encoding: u8) -> String {
    match encoding {
        1 | 2 => {
            let be = encoding == 2;
            let units: Vec<_> = data
                .as_chunks::<2>()
                .0
                .iter()
                .map(|b| {
                    if be {
                        u16::from_be_bytes(*b)
                    } else {
                        u16::from_le_bytes(*b)
                    }
                })
                .collect();
            String::from_utf16_lossy(&units)
        }
        3 => String::from_utf8_lossy(data).into_owned(),
        _ => data.iter().map(|b| char::from(*b)).collect(),
    }
}
pub(crate) fn terminator(data: &[u8], encoding: u8) -> usize {
    if encoding == 1 || encoding == 2 {
        data.as_chunks::<2>()
            .0
            .iter()
            .position(|b| *b == [0, 0])
            .map_or(data.len() / 2 * 2, |n| n * 2)
    } else {
        data.iter().position(|b| *b == 0).unwrap_or(data.len())
    }
}
pub(crate) fn id3_text(data: &[u8]) -> Option<String> {
    if data.len() < 2 {
        return None;
    }
    let mut enc = data[0];
    let mut raw = &data[1..];
    if enc == 1 && raw.len() > 2 {
        if raw.starts_with(&[254, 255]) {
            enc = 2;
            raw = &raw[2..]
        } else if raw.starts_with(&[255, 254]) {
            raw = &raw[2..]
        }
    }
    clean_text(&text(&raw[..terminator(raw, enc)], enc))
}
pub(crate) fn genre(text: String) -> String {
    if let Some(rest) = text.strip_prefix('(') {
        if let Some((id, tail)) = rest.split_once(')') {
            if id.bytes().all(|c| c.is_ascii_digit()) && !id.is_empty() {
                if let Some(t) = clean_text(tail) {
                    return t;
                }
                if let Some(g) = id.parse::<usize>().ok().and_then(|n| GENRES.get(n)) {
                    return (*g).into();
                }
            }
        }
    }
    text
}
