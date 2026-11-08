use crate::{utils::*, ArtworkMimeType, Context, Error, Metadata, RangeReader};
pub(crate) async fn extract<R: RangeReader + ?Sized>(
    ctx: &mut Context<'_, R>,
) -> Result<Option<Metadata>, Error> {
    if ctx.size() < 10 {
        return Ok(None);
    }
    let header = ctx.read(0, 10).await?;
    if &header[..3] != b"ID3" {
        return Ok(None);
    }
    let version = header[3];
    let size = syncsafe(&header[6..10]) as usize;
    let mut ext = 0;
    if header[5] & 64 != 0 {
        let e = ctx.read(10, 4).await?;
        ext = if version == 4 {
            syncsafe(&e)
        } else {
            be32(&e, 0).unwrap()
        } as usize;
    }
    if ext > size {
        return Err(Error::Malformed);
    }
    let tag = ctx.read(10 + ext as u64, size - ext).await?;
    parse(&tag, version, ctx)
}
pub(crate) fn embedded<R: RangeReader + ?Sized>(
    bytes: &[u8],
    ctx: &mut Context<'_, R>,
) -> Result<Option<Metadata>, Error> {
    if bytes.len() < 10 || &bytes[..3] != b"ID3" {
        return Ok(None);
    }
    let version = bytes[3];
    let size = syncsafe(&bytes[6..10]) as usize;
    let ext = if bytes[5] & 64 != 0 {
        (if version == 4 {
            syncsafe(bytes.get(10..14).ok_or(Error::Malformed)?)
        } else {
            be32(bytes, 10).ok_or(Error::Malformed)?
        }) as usize
    } else {
        0
    };
    if ext > size {
        return Err(Error::Malformed);
    }
    let tag = bytes.get(10 + ext..10 + size).ok_or(Error::Malformed)?;
    parse(tag, version, ctx)
}
fn picture(data: &[u8], v22: bool) -> Option<(&[u8], ArtworkMimeType, u8)> {
    if data.len() < 4 {
        return None;
    }
    let enc = data[0];
    let (mime, mut pos) = if v22 {
        (
            match data.get(1..4)? {
                b"PNG" => "image/png",
                b"GIF" => "image/gif",
                _ => "image/jpeg",
            }
            .to_owned(),
            4,
        )
    } else {
        let end = terminator(&data[1..], 0);
        (text(data.get(1..1 + end)?, 0), 2 + end)
    };
    let kind = *data.get(pos)?;
    pos += 1;
    let rest = data.get(pos..)?;
    let end = terminator(rest, enc);
    pos = pos.checked_add(end + if enc == 1 || enc == 2 { 2 } else { 1 })?;
    let image = data.get(pos..)?;
    if image.is_empty() {
        return None;
    }
    let mime = detect_image_type(image).unwrap_or(if mime.contains("png") {
        ArtworkMimeType::Png
    } else {
        ArtworkMimeType::Jpeg
    });
    Some((image, mime, kind))
}
fn parse<R: RangeReader + ?Sized>(
    tag: &[u8],
    version: u8,
    ctx: &mut Context<'_, R>,
) -> Result<Option<Metadata>, Error> {
    let mut m = Metadata::default();
    let mut front = None;
    let mut other = None;
    let mut pos = 0;
    let header = if version >= 3 { 10 } else { 6 };
    while pos + header < tag.len() {
        ctx.item()?;
        if tag[pos] == 0 {
            break;
        }
        let id = &tag[pos..pos + if version >= 3 { 4 } else { 3 }];
        let size = if version == 4 {
            syncsafe(&tag[pos + 4..pos + 8])
        } else if version >= 3 {
            be32(tag, pos + 4).unwrap()
        } else {
            u32::from_be_bytes([0, tag[pos + 3], tag[pos + 4], tag[pos + 5]])
        } as usize;
        if size == 0 {
            break;
        }
        let end = pos
            .checked_add(header)
            .and_then(|n| n.checked_add(size))
            .ok_or(Error::Malformed)?;
        let data = tag.get(pos + header..end).ok_or(Error::Malformed)?;
        let value = || ctx.text(id3_text(data).unwrap_or_default());
        match id {
            b"TIT2" | b"TT2" => {
                if m.title.is_none() {
                    m.title = value()?
                }
            }
            b"TPE1" | b"TP1" => {
                if m.artist.is_none() {
                    m.artist = value()?
                }
            }
            b"TALB" | b"TAL" => {
                if m.album.is_none() {
                    m.album = value()?
                }
            }
            b"TCON" | b"TCO" => {
                if m.genre.is_none() {
                    m.genre = value()?.map(genre)
                }
            }
            b"TPUB" | b"TPB" => {
                if m.label.is_none() {
                    m.label = value()?
                }
            }
            b"TYER" | b"TYE" | b"TDRC" => {
                if m.year.is_none() {
                    m.year = value()?.and_then(|s| parse_year(&s))
                }
            }
            b"TBPM" | b"TBP" => {
                if m.bpm.is_none() {
                    m.bpm = value()?.and_then(|s| parse_bpm(&s))
                }
            }
            b"TKEY" | b"TKE" => {
                if m.key.is_none() {
                    m.key = value()?
                }
            }
            b"APIC" | b"PIC" => {
                if let Some((image, mime, kind)) = picture(data, version < 3) {
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
    }
    if let Some((data, mime)) = front.or(other) {
        m.artwork = Some(data);
        m.artwork_mime_type = Some(mime)
    }
    Ok(m.found())
}
