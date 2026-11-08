//! Bounded metadata extraction through caller-owned transports. No implicit IO.
use serde::{Deserialize, Serialize};
use std::{
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
mod aiff;
mod flac;
mod id3;
mod mp4;
mod utils;
pub use utils::{clean_text, detect_image_type, normalize_mime_type, parse_bpm, parse_year};
pub const SUPPORTED_EXTENSIONS: &[&str] = &[
    "mp3", "m4a", "mp4", "m4p", "m4b", "aac", "flac", "aiff", "aif", "aifc",
];
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Mp3,
    Mp4,
    Flac,
    Aiff,
}
pub fn parser_for_extension(extension: &str) -> Option<Format> {
    let ext = extension.to_lowercase();
    match ext.strip_prefix('.').unwrap_or(&ext) {
        "mp3" => Some(Format::Mp3),
        "m4a" | "mp4" | "m4p" | "m4b" | "aac" => Some(Format::Mp4),
        "flac" => Some(Format::Flac),
        "aiff" | "aif" | "aifc" => Some(Format::Aiff),
        _ => None,
    }
}
pub fn is_extension_supported(extension: &str) -> bool {
    parser_for_extension(extension).is_some()
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArtworkMimeType {
    #[serde(rename = "image/jpeg")]
    Jpeg,
    #[serde(rename = "image/png")]
    Png,
    #[serde(rename = "image/gif")]
    Gif,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum PictureType {
    Other = 0,
    FileIcon32x32 = 1,
    OtherFileIcon = 2,
    FrontCover = 3,
    BackCover = 4,
    LeafletPage = 5,
    Media = 6,
    LeadArtist = 7,
    Artist = 8,
    Conductor = 9,
    Band = 10,
    Composer = 11,
    Lyricist = 12,
    RecordingLocation = 13,
    DuringRecording = 14,
    DuringPerformance = 15,
    MovieScreenCapture = 16,
    BrightColoredFish = 17,
    Illustration = 18,
    BandLogotype = 19,
    PublisherLogotype = 20,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Metadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artist: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub album: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub genre: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub year: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bpm: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artwork: Option<Vec<u8>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artwork_mime_type: Option<ArtworkMimeType>,
}
impl Metadata {
    fn found(self) -> Option<Self> {
        (self != Self::default()).then_some(self)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidLimits,
    Bounds,
    LimitExceeded,
    ShortRead,
    Reader,
    Malformed,
    Cancelled,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Metadata extraction error: {self:?}")
    }
}
impl std::error::Error for Error {}
pub type ReadFuture<'a> = Pin<Box<dyn Future<Output = Result<Vec<u8>, Error>> + Send + 'a>>;
/// Implementations must honor the requested bound and cancellation/drop behavior.
/// The extractor validates returned length, but cannot stop a transport allocating
/// outside this contract or blocking its own future. Never return whole audio files.
pub trait RangeReader: Send {
    fn size(&self) -> u64;
    fn extension(&self) -> &str;
    fn read(&mut self, offset: u64, length: usize) -> ReadFuture<'_>;
}
pub struct BufferReader<'a> {
    data: &'a [u8],
    extension: String,
}
impl<'a> BufferReader<'a> {
    pub fn new(data: &'a [u8], extension: impl Into<String>) -> Self {
        Self {
            data,
            extension: extension.into(),
        }
    }
}
impl RangeReader for BufferReader<'_> {
    fn size(&self) -> u64 {
        self.data.len() as u64
    }
    fn extension(&self) -> &str {
        &self.extension
    }
    fn read(&mut self, offset: u64, length: usize) -> ReadFuture<'_> {
        Box::pin(async move {
            let start = usize::try_from(offset).map_err(|_| Error::Bounds)?;
            let end = start
                .checked_add(length)
                .ok_or(Error::Bounds)?
                .min(self.data.len());
            Ok(self.data.get(start..end).ok_or(Error::Bounds)?.to_vec())
        })
    }
}
#[derive(Clone, Debug)]
pub struct Limits {
    pub max_read_bytes: usize,
    pub max_total_bytes: usize,
    pub max_reads: usize,
    pub max_items: usize,
    pub max_text_bytes: usize,
    pub max_artwork_bytes: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_read_bytes: 16 * 1024 * 1024,
            max_total_bytes: 32 * 1024 * 1024,
            max_reads: 4096,
            max_items: 16384,
            max_text_bytes: 256 * 1024,
            max_artwork_bytes: 8 * 1024 * 1024,
        }
    }
}
impl Limits {
    fn valid(&self) -> bool {
        self.max_read_bytes > 0
            && self.max_read_bytes <= 64 * 1024 * 1024
            && self.max_total_bytes > 0
            && self.max_total_bytes <= 128 * 1024 * 1024
            && self.max_reads > 0
            && self.max_reads <= 65536
            && self.max_items > 0
            && self.max_items <= 65536
            && self.max_text_bytes > 0
            && self.max_text_bytes <= 1024 * 1024
            && self.max_artwork_bytes > 0
            && self.max_artwork_bytes <= 32 * 1024 * 1024
    }
}
#[derive(Clone, Default)]
pub struct Cancellation(Arc<AtomicBool>);
impl Cancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release)
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}
struct Context<'a, R: RangeReader + ?Sized> {
    reader: &'a mut R,
    limits: &'a Limits,
    cancel: &'a Cancellation,
    reads: usize,
    bytes: usize,
    items: usize,
}
impl<R: RangeReader + ?Sized> Context<'_, R> {
    fn size(&self) -> u64 {
        self.reader.size()
    }
    fn check(&self) -> Result<(), Error> {
        if self.cancel.is_cancelled() {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }
    fn item(&mut self) -> Result<(), Error> {
        self.check()?;
        self.items += 1;
        if self.items > self.limits.max_items {
            Err(Error::LimitExceeded)
        } else {
            Ok(())
        }
    }
    async fn read(&mut self, offset: u64, length: usize) -> Result<Vec<u8>, Error> {
        self.check()?;
        if length > self.limits.max_read_bytes {
            return Err(Error::LimitExceeded);
        }
        let end = offset.checked_add(length as u64).ok_or(Error::Bounds)?;
        if end > self.size() {
            return Err(Error::Bounds);
        }
        self.reads += 1;
        self.bytes = self.bytes.checked_add(length).ok_or(Error::LimitExceeded)?;
        if self.reads > self.limits.max_reads || self.bytes > self.limits.max_total_bytes {
            return Err(Error::LimitExceeded);
        }
        let data = self.reader.read(offset, length).await?;
        self.check()?;
        if data.len() != length {
            return Err(if data.len() > length {
                Error::Reader
            } else {
                Error::ShortRead
            });
        }
        Ok(data)
    }
    fn text(&self, text: String) -> Result<Option<String>, Error> {
        if text.len() > self.limits.max_text_bytes {
            return Err(Error::LimitExceeded);
        }
        Ok(clean_text(&text))
    }
    fn artwork(&self, data: &[u8]) -> Result<Vec<u8>, Error> {
        if data.len() > self.limits.max_artwork_bytes {
            return Err(Error::LimitExceeded);
        }
        Ok(data.to_vec())
    }
}
pub async fn extract<R: RangeReader + ?Sized>(
    reader: &mut R,
    limits: &Limits,
    cancel: &Cancellation,
) -> Result<Option<Metadata>, Error> {
    let Some(format) = parser_for_extension(reader.extension()) else {
        return Ok(None);
    };
    extract_format(reader, format, limits, cancel).await
}
pub async fn extract_format<R: RangeReader + ?Sized>(
    reader: &mut R,
    format: Format,
    limits: &Limits,
    cancel: &Cancellation,
) -> Result<Option<Metadata>, Error> {
    if !limits.valid() {
        return Err(Error::InvalidLimits);
    }
    let mut ctx = Context {
        reader,
        limits,
        cancel,
        reads: 0,
        bytes: 0,
        items: 0,
    };
    match format {
        Format::Mp3 => id3::extract(&mut ctx).await,
        Format::Mp4 => mp4::extract(&mut ctx).await,
        Format::Flac => flac::extract(&mut ctx).await,
        Format::Aiff => aiff::extract(&mut ctx).await,
    }
}
/// Original extractMetadata behavior: unsupported formats and any failure become None.
pub async fn extract_compatible<R: RangeReader + ?Sized>(reader: &mut R) -> Option<Metadata> {
    extract(reader, &Limits::default(), &Cancellation::default())
        .await
        .ok()
        .flatten()
}

/// Direct ID3 parser regardless of extension.
pub async fn extract_from_mp3<R: RangeReader + ?Sized>(
    reader: &mut R,
    limits: &Limits,
    cancel: &Cancellation,
) -> Result<Option<Metadata>, Error> {
    extract_format(reader, Format::Mp3, limits, cancel).await
}
/// Direct MP4 parser regardless of extension.
pub async fn extract_from_mp4<R: RangeReader + ?Sized>(
    reader: &mut R,
    limits: &Limits,
    cancel: &Cancellation,
) -> Result<Option<Metadata>, Error> {
    extract_format(reader, Format::Mp4, limits, cancel).await
}
/// Direct FLAC parser regardless of extension.
pub async fn extract_from_flac<R: RangeReader + ?Sized>(
    reader: &mut R,
    limits: &Limits,
    cancel: &Cancellation,
) -> Result<Option<Metadata>, Error> {
    extract_format(reader, Format::Flac, limits, cancel).await
}
/// Direct AIFF parser regardless of extension.
pub async fn extract_from_aiff<R: RangeReader + ?Sized>(
    reader: &mut R,
    limits: &Limits,
    cancel: &Cancellation,
) -> Result<Option<Metadata>, Error> {
    extract_format(reader, Format::Aiff, limits, cancel).await
}
