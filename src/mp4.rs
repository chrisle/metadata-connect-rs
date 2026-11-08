use crate::{utils::*, Context, Error, Metadata, RangeReader};
#[derive(Clone, Copy)]
struct Atom {
    kind: [u8; 4],
    body: u64,
    end: u64,
}
async fn header<R: RangeReader + ?Sized>(
    ctx: &mut Context<'_, R>,
    pos: u64,
    parent_end: u64,
) -> Result<Option<Atom>, Error> {
    if pos.checked_add(8).is_none_or(|end| end > parent_end) {
        return Ok(None);
    }
    ctx.item()?;
    let h = ctx.read(pos, 8).await?;
    let mut size = be32(&h, 0).unwrap() as u64;
    let mut count = 8;
    if size == 1 {
        if pos.checked_add(16).is_none_or(|end| end > parent_end) {
            return Err(Error::Malformed);
        }
        let ext = ctx.read(pos + 8, 8).await?;
        size = u64::from_be_bytes(ext.try_into().unwrap());
        count = 16
    } else if size == 0 {
        size = parent_end - pos
    }
    if size < count {
        return Err(Error::Malformed);
    }
    let end = pos
        .checked_add(size)
        .filter(|end| *end <= parent_end)
        .ok_or(Error::Malformed)?;
    Ok(Some(Atom {
        kind: h[4..8].try_into().unwrap(),
        body: pos + count,
        end,
    }))
}
async fn find<R: RangeReader + ?Sized>(
    ctx: &mut Context<'_, R>,
    start: u64,
    end: u64,
    kind: &[u8; 4],
) -> Result<Option<Atom>, Error> {
    let mut pos = start;
    while pos < end {
        let Some(a) = header(ctx, pos, end).await? else {
            break;
        };
        if &a.kind == kind {
            return Ok(Some(a));
        }
        pos = a.end
    }
    Ok(None)
}
async fn data<R: RangeReader + ?Sized>(
    ctx: &mut Context<'_, R>,
    atom: Atom,
    skip: u64,
) -> Result<Option<Vec<u8>>, Error> {
    let Some(d) = find(ctx, atom.body, atom.end, b"data").await? else {
        return Ok(None);
    };
    if d.end - d.body < skip {
        return Ok(None);
    }
    let len = usize::try_from(d.end - d.body - skip).map_err(|_| Error::LimitExceeded)?;
    Ok(Some(ctx.read(d.body + skip, len).await?))
}
async fn text_value<R: RangeReader + ?Sized>(
    ctx: &mut Context<'_, R>,
    atom: Atom,
) -> Result<Option<String>, Error> {
    let Some(bytes) = data(ctx, atom, 8).await? else {
        return Ok(None);
    };
    ctx.text(String::from_utf8_lossy(&bytes).into_owned())
}
async fn numeric<R: RangeReader + ?Sized>(
    ctx: &mut Context<'_, R>,
    atom: Atom,
) -> Result<Option<u16>, Error> {
    let Some(bytes) = data(ctx, atom, 8).await? else {
        return Ok(None);
    };
    Ok(bytes
        .get(..2)
        .map(|b| u16::from_be_bytes(b.try_into().unwrap())))
}
pub(crate) async fn extract<R: RangeReader + ?Sized>(
    ctx: &mut Context<'_, R>,
) -> Result<Option<Metadata>, Error> {
    if ctx.size() < 8 {
        return Ok(None);
    }
    let first = ctx.read(0, 8).await?;
    if &first[4..8] != b"ftyp" {
        return Ok(None);
    }
    let Some(moov) = find(ctx, 0, ctx.size(), b"moov").await? else {
        return Ok(None);
    };
    let Some(udta) = find(ctx, moov.body, moov.end, b"udta").await? else {
        return Ok(None);
    };
    let Some(meta) = find(ctx, udta.body, udta.end, b"meta").await? else {
        return Ok(None);
    };
    if meta.body.checked_add(4).is_none_or(|end| end > meta.end) {
        return Err(Error::Malformed);
    }
    let Some(ilst) = find(ctx, meta.body + 4, meta.end, b"ilst").await? else {
        return Ok(None);
    };
    let mut m = Metadata::default();
    let mut pos = ilst.body;
    while pos < ilst.end {
        let Some(a) = header(ctx, pos, ilst.end).await? else {
            break;
        };
        match &a.kind {
            b"\xa9nam" => {
                if m.title.is_none() {
                    m.title = text_value(ctx, a).await?
                }
            }
            b"\xa9ART" => {
                if m.artist.is_none() {
                    m.artist = text_value(ctx, a).await?
                }
            }
            b"\xa9alb" => {
                if m.album.is_none() {
                    m.album = text_value(ctx, a).await?
                }
            }
            b"\xa9gen" => {
                if m.genre.is_none() {
                    m.genre = text_value(ctx, a).await?
                }
            }
            b"gnre" => {
                if m.genre.is_none() {
                    m.genre = numeric(ctx, a)
                        .await?
                        .and_then(|n| n.checked_sub(1))
                        .and_then(|n| GENRES.get(n as usize))
                        .map(|s| (*s).into())
                }
            }
            b"\xa9day" => {
                if m.year.is_none() {
                    m.year = text_value(ctx, a).await?.and_then(|s| year_any(&s))
                }
            }
            b"tmpo" => {
                if m.bpm.is_none() {
                    m.bpm = numeric(ctx, a)
                        .await?
                        .filter(|n| *n > 0 && *n < 500)
                        .map(f64::from)
                }
            }
            b"----" => {
                if m.label.is_none() {
                    if let Some(name) = find(ctx, a.body, a.end, b"name").await? {
                        if name.end - name.body >= 4 {
                            let bytes = ctx
                                .read(
                                    name.body + 4,
                                    usize::try_from(name.end - name.body - 4)
                                        .map_err(|_| Error::LimitExceeded)?,
                                )
                                .await?;
                            let name = ctx
                                .text(String::from_utf8_lossy(&bytes).into_owned())?
                                .unwrap_or_default()
                                .to_uppercase();
                            if ["LABEL", "PUBLISHER", "ORGANIZATION"].contains(&name.as_str()) {
                                m.label = text_value(ctx, a).await?
                            }
                        }
                    }
                }
            }
            b"covr" if m.artwork.is_none() => {
                if let Some(bytes) = data(ctx, a, 8).await? {
                    if !bytes.is_empty() {
                        m.artwork = Some(ctx.artwork(&bytes)?);
                        m.artwork_mime_type =
                            Some(detect_image_type(&bytes).unwrap_or(crate::ArtworkMimeType::Jpeg))
                    }
                }
            }
            _ => (),
        };
        pos = a.end
    }
    Ok(m.found())
}
