use crate::{id3, utils::be32, Context, Error, Metadata, RangeReader};
pub(crate) async fn extract<R: RangeReader + ?Sized>(
    ctx: &mut Context<'_, R>,
) -> Result<Option<Metadata>, Error> {
    if ctx.size() < 12 {
        return Ok(None);
    }
    let header = ctx.read(0, 12).await?;
    if &header[..4] != b"FORM" || (&header[8..12] != b"AIFF" && &header[8..12] != b"AIFC") {
        return Ok(None);
    }
    let end = (8 + be32(&header, 4).unwrap() as u64).min(ctx.size());
    let mut pos = 12;
    while pos + 8 < end {
        ctx.item()?;
        let h = ctx.read(pos, 8).await?;
        let size = be32(&h, 4).unwrap() as u64;
        if size == 0 || pos + 8 + size > end {
            break;
        }
        if &h[..4] == b"ID3 " || &h[..4] == b"id3 " {
            let data = ctx.read(pos + 8, size as usize).await?;
            return id3::embedded(&data, ctx);
        }
        pos += 8 + size + (size % 2)
    }
    Ok(None)
}
