use crate::{utils::*, ArtworkMimeType, Context, Error, Metadata, RangeReader};
use std::collections::BTreeMap;
fn picture(data: &[u8]) -> Option<(&[u8], ArtworkMimeType, u32)> {
    if data.len() < 32 {
        return None;
    }
    let kind = be32(data, 0)?;
    let ml = be32(data, 4)? as usize;
    let mime = String::from_utf8_lossy(data.get(8..8usize.checked_add(ml)?)?);
    let mut pos = 8usize.checked_add(ml)?;
    let dl = be32(data, pos)? as usize;
    pos = pos.checked_add(4)?.checked_add(dl)?.checked_add(16)?;
    let size = be32(data, pos)? as usize;
    let image = data.get(pos.checked_add(4)?..pos.checked_add(4)?.checked_add(size)?)?;
    if image.is_empty() {
        return None;
    }
    Some((image, normalize_mime_type(&mime), kind))
}
fn comments<R: RangeReader + ?Sized>(
    data: &[u8],
    ctx: &mut Context<'_, R>,
) -> Result<BTreeMap<String, String>, Error> {
    let mut out = BTreeMap::new();
    let Some(vendor) = le32(data, 0) else {
        return Ok(out);
    };
    let mut pos = 4usize
        .checked_add(vendor as usize)
        .ok_or(Error::Malformed)?;
    let Some(count) = le32(data, pos) else {
        return Ok(out);
    };
    pos += 4;
    for _ in 0..count {
        ctx.item()?;
        let Some(size) = le32(data, pos) else { break };
        pos += 4;
        let end = pos.checked_add(size as usize).ok_or(Error::Malformed)?;
        let Some(raw) = data.get(pos..end) else { break };
        pos = end;
        let text = String::from_utf8_lossy(raw);
        if let Some((key, value)) = text.split_once('=') {
            if !key.is_empty() {
                if key.len() > ctx.limits.max_text_bytes || value.len() > ctx.limits.max_text_bytes
                {
                    return Err(Error::LimitExceeded);
                }
                let value = value.to_owned();
                let slot = out.entry(key.to_uppercase()).or_insert_with(String::new);
                if slot.is_empty() {
                    *slot = value
                }
            }
        }
    }
    Ok(out)
}
fn field(map: &BTreeMap<String, String>, names: &[&str]) -> Option<String> {
    names
        .iter()
        .find_map(|n| map.get(*n).filter(|s| !s.is_empty()))
        .and_then(|s| clean_text(s))
}
pub(crate) async fn extract<R: RangeReader + ?Sized>(
    ctx: &mut Context<'_, R>,
) -> Result<Option<Metadata>, Error> {
    if ctx.size() < 4 {
        return Ok(None);
    }
    if ctx.read(0, 4).await? != b"fLaC" {
        return Ok(None);
    }
    let mut m = Metadata::default();
    let (mut front, mut other) = (None, None);
    let mut pos = 4;
    while pos < ctx.size() {
        ctx.item()?;
        if ctx.size() - pos < 4 {
            break;
        }
        let h = ctx.read(pos, 4).await?;
        let size = u32::from_be_bytes([0, h[1], h[2], h[3]]) as usize;
        let end = pos + 4 + size as u64;
        if size == 0 || end > ctx.size() {
            break;
        }
        match h[0] & 127 {
            4 => {
                let data = ctx.read(pos + 4, size).await?;
                let fields = comments(&data, ctx)?;
                if m.title.is_none() {
                    m.title = field(&fields, &["TITLE"])
                }
                if m.artist.is_none() {
                    m.artist = field(&fields, &["ARTIST"])
                }
                if m.album.is_none() {
                    m.album = field(&fields, &["ALBUM"])
                }
                if m.genre.is_none() {
                    m.genre = field(&fields, &["GENRE"])
                }
                if m.label.is_none() {
                    m.label = field(
                        &fields,
                        &["LABEL", "PUBLISHER", "ORGANIZATION", "ORGANISATION"],
                    )
                }
                if m.year.is_none() {
                    m.year = field(&fields, &["DATE", "YEAR"]).and_then(|s| parse_year(&s))
                }
                if m.bpm.is_none() {
                    m.bpm = field(&fields, &["BPM", "TEMPO"]).and_then(|s| parse_bpm(&s))
                }
                if m.key.is_none() {
                    m.key = field(&fields, &["KEY", "INITIALKEY"])
                }
            }
            6 => {
                let data = ctx.read(pos + 4, size).await?;
                if let Some((image, mime, kind)) = picture(&data) {
                    let art = (ctx.artwork(image)?, mime);
                    if kind == 3 {
                        front = Some(art)
                    } else if other.is_none() {
                        other = Some(art)
                    }
                }
            }
            _ => (),
        };
        pos = end;
        if h[0] & 128 != 0 {
            break;
        }
    }
    if let Some((data, mime)) = front.or(other) {
        m.artwork = Some(data);
        m.artwork_mime_type = Some(mime)
    }
    Ok(m.found())
}
