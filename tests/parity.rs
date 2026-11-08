use metadata_connect_rs::*;
use serde_json::Value;
use std::future::Future;
fn normalize(mut value: Value) -> Value {
    fn visit(value: &mut Value) {
        match value {
            Value::Number(n) => *n = serde_json::Number::from_f64(n.as_f64().unwrap()).unwrap(),
            Value::Array(a) => a.iter_mut().for_each(visit),
            Value::Object(o) => o.values_mut().for_each(visit),
            _ => (),
        }
    }
    visit(&mut value);
    value
}
fn manifest() -> Value {
    serde_json::from_str(include_str!("fixtures/oracle.json")).unwrap()
}
#[tokio::test]
async fn original_typescript_extraction_cases_and_documented_corrections() {
    for case in manifest()["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let bytes = std::fs::read(format!(
            "{}/tests/fixtures/{name}.bin",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        let mut reader = BufferReader::new(&bytes, case["extension"].as_str().unwrap());
        let actual = extract(&mut reader, &Limits::default(), &Cancellation::default())
            .await
            .unwrap();
        let value = serde_json::to_value(actual).unwrap();
        assert_eq!(
            normalize(value),
            normalize(case["expected"].clone()),
            "case {name}"
        );
    }
}
#[test]
fn extension_and_utility_contract() {
    assert_eq!(SUPPORTED_EXTENSIONS.len(), 10);
    for e in SUPPORTED_EXTENSIONS {
        assert!(is_extension_supported(&e.to_uppercase()));
        assert!(is_extension_supported(&format!(".{e}")));
    }
    for e in ["wav", "ogg", "txt", "..mp3", ""] {
        assert!(!is_extension_supported(e));
    }
    for case in manifest()["utilities"]["bpm"].as_array().unwrap() {
        assert_eq!(
            normalize(serde_json::to_value(parse_bpm(case["input"].as_str().unwrap())).unwrap()),
            normalize(case["output"].clone())
        );
    }
    for case in manifest()["utilities"]["year"].as_array().unwrap() {
        assert_eq!(
            serde_json::to_value(parse_year(case["input"].as_str().unwrap())).unwrap(),
            case["output"]
        );
    }
    assert_eq!(
        detect_image_type(&[255, 216, 255, 0]),
        Some(ArtworkMimeType::Jpeg)
    );
    assert_eq!(detect_image_type(b"GIF8"), Some(ArtworkMimeType::Gif));
    assert_eq!(normalize_mime_type("IMAGE/PNG"), ArtworkMimeType::Png);
    assert_eq!(clean_text(" \0é\u{feff}"), Some("é".into()));
}
#[tokio::test]
async fn buffer_reader_clamps_public_reads_and_invalid_formats_do_no_io() {
    let bytes = b"Short";
    let mut reader = BufferReader::new(bytes, "MP3");
    assert_eq!(reader.read(0, 100).await.unwrap(), bytes);
    assert_eq!(reader.read(7, 1).await, Err(Error::Bounds));
    let mut reader = BufferReader::new(bytes, "wav");
    assert_eq!(extract_compatible(&mut reader).await, None);
}
#[tokio::test]
async fn hostile_truncations_and_frame_lengths_never_panic() {
    let bytes = include_bytes!("fixtures/id3-v23-all.bin");
    for end in 0..bytes.len() {
        let mut r = BufferReader::new(&bytes[..end], "mp3");
        let _ = extract(&mut r, &Limits::default(), &Cancellation::default()).await;
    }
    let mut bytes = bytes.to_vec();
    bytes[6..10].fill(127);
    let mut r = BufferReader::new(&bytes, "mp3");
    assert!(
        extract(&mut r, &Limits::default(), &Cancellation::default())
            .await
            .is_err()
    );
}
#[tokio::test]
async fn caps_read_counts_artwork_and_cancellation_are_explicit() {
    let bytes = include_bytes!("fixtures/id3-v23-all.bin");
    for limits in [
        Limits {
            max_read_bytes: 9,
            ..Limits::default()
        },
        Limits {
            max_total_bytes: 10,
            ..Limits::default()
        },
        Limits {
            max_reads: 1,
            ..Limits::default()
        },
        Limits {
            max_items: 1,
            ..Limits::default()
        },
        Limits {
            max_artwork_bytes: 3,
            ..Limits::default()
        },
        Limits {
            max_text_bytes: 2,
            ..Limits::default()
        },
    ] {
        let mut r = BufferReader::new(bytes, "mp3");
        assert_eq!(
            extract(&mut r, &limits, &Cancellation::default()).await,
            Err(Error::LimitExceeded)
        );
    }
    let mut r = BufferReader::new(bytes, "mp3");
    let token = Cancellation::default();
    token.cancel();
    assert_eq!(
        extract(&mut r, &Limits::default(), &token).await,
        Err(Error::Cancelled)
    );
    let mut r = BufferReader::new(bytes, "mp3");
    assert_eq!(
        extract(
            &mut r,
            &Limits {
                max_reads: 0,
                ..Limits::default()
            },
            &Cancellation::default()
        )
        .await,
        Err(Error::InvalidLimits)
    );
}
struct Dishonest {
    mode: u8,
    reads: usize,
}
impl RangeReader for Dishonest {
    fn size(&self) -> u64 {
        100
    }
    fn extension(&self) -> &str {
        "mp3"
    }
    fn read(&mut self, _: u64, len: usize) -> ReadFuture<'_> {
        self.reads += 1;
        Box::pin(async move {
            match self.mode {
                0 => Ok(vec![0; len + 1]),
                1 => Ok(vec![]),
                _ => Err(Error::Reader),
            }
        })
    }
}
#[tokio::test]
async fn reader_errors_and_oversized_short_responses_do_not_parse() {
    for (mode, expected) in [
        (0, Error::Reader),
        (1, Error::ShortRead),
        (2, Error::Reader),
    ] {
        let mut reader = Dishonest { mode, reads: 0 };
        assert_eq!(
            extract(&mut reader, &Limits::default(), &Cancellation::default()).await,
            Err(expected)
        );
        assert_eq!(reader.reads, 1);
    }
}

struct Sparse {
    size: u64,
    segments: Vec<(u64, Vec<u8>)>,
    requested: usize,
    largest: usize,
}
impl RangeReader for Sparse {
    fn size(&self) -> u64 {
        self.size
    }
    fn extension(&self) -> &str {
        "m4a"
    }
    fn read(&mut self, offset: u64, length: usize) -> ReadFuture<'_> {
        self.requested += length;
        self.largest = self.largest.max(length);
        Box::pin(async move {
            let mut output = vec![0; length];
            for (start, data) in &self.segments {
                for (i, byte) in output.iter_mut().enumerate() {
                    let p = offset.checked_add(i as u64).ok_or(Error::Bounds)?;
                    if let Some(local) = p
                        .checked_sub(*start)
                        .and_then(|p| usize::try_from(p).ok())
                        .filter(|p| *p < data.len())
                    {
                        *byte = data[local]
                    }
                }
            }
            Ok(output)
        })
    }
}
#[tokio::test]
async fn sparse_multi_gigabyte_audio_reads_only_metadata_and_skips_audio_body() {
    let file = include_bytes!("fixtures/mp4-all-corrected.bin");
    let gap = 5u64 * 1024 * 1024 * 1024;
    let mut mdat = 1u32.to_be_bytes().to_vec();
    mdat.extend(b"mdat");
    mdat.extend((gap - 16).to_be_bytes());
    let mut reader = Sparse {
        size: gap + (file.len() - 16) as u64,
        segments: vec![
            (0, file[..16].to_vec()),
            (16, mdat),
            (gap, file[16..].to_vec()),
        ],
        requested: 0,
        largest: 0,
    };
    let metadata = extract_compatible(&mut reader).await.unwrap();
    assert_eq!(metadata.title.as_deref(), Some("Title"));
    assert!(reader.requested < 4096);
    assert!(reader.largest < 1024);
}
#[tokio::test]
async fn malicious_sparse_u64_end_cannot_overflow_atom_or_parent_math() {
    let position = u64::MAX - 8;
    let mut ftyp = (1u32).to_be_bytes().to_vec();
    ftyp.extend(b"ftyp");
    ftyp.extend(position.to_be_bytes());
    let mut extended = (1u32).to_be_bytes().to_vec();
    extended.extend(b"moov");
    let mut reader = Sparse {
        size: u64::MAX,
        segments: vec![(0, ftyp), (position, extended)],
        requested: 0,
        largest: 0,
    };
    assert_eq!(
        extract(&mut reader, &Limits::default(), &Cancellation::default()).await,
        Err(Error::Malformed)
    );
    assert!(reader.requested < 100);
}
#[tokio::test]
async fn every_fixture_truncation_all_formats_and_mutated_headers_are_bounded() {
    for name in [
        "mp4-all-corrected",
        "flac-all",
        "flac-artwork",
        "aiff-label",
    ] {
        let case = manifest()["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == name)
            .unwrap()
            .clone();
        let extension = case["extension"].as_str().unwrap();
        let bytes = std::fs::read(format!(
            "{}/tests/fixtures/{name}.bin",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        for end in 0..bytes.len() {
            let mut r = BufferReader::new(&bytes[..end], extension);
            let _ = extract(&mut r, &Limits::default(), &Cancellation::default()).await;
        }
        for i in 0..bytes.len().min(64) {
            let mut mutated = bytes.clone();
            mutated[i] = 255;
            let mut r = BufferReader::new(&mutated, extension);
            let _ = extract(&mut r, &Limits::default(), &Cancellation::default()).await;
        }
    }
}
#[tokio::test]
async fn artwork_front_cover_precedence_and_latest_front_cover() {
    fn frame(kind: u8, byte: u8) -> Vec<u8> {
        let data = vec![
            0, b'i', b'm', b'a', b'g', b'e', b'/', b'p', b'n', b'g', 0, kind, 0, 137, 80, 78, 71,
            byte,
        ];
        let mut f = b"APIC".to_vec();
        f.extend((data.len() as u32).to_be_bytes());
        f.extend([0, 0]);
        f.extend(data);
        f
    }
    let body = [frame(3, 1), frame(4, 2), frame(3, 3)].concat();
    let mut tag = vec![
        73,
        68,
        51,
        3,
        0,
        0,
        0,
        0,
        (body.len() >> 7) as u8,
        (body.len() & 127) as u8,
    ];
    tag.extend(body);
    let mut r = BufferReader::new(&tag, "mp3");
    assert_eq!(
        extract_compatible(&mut r)
            .await
            .unwrap()
            .artwork
            .unwrap()
            .last(),
        Some(&3)
    );
}
struct Pending {
    dropped: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
impl RangeReader for Pending {
    fn size(&self) -> u64 {
        100
    }
    fn extension(&self) -> &str {
        "mp3"
    }
    fn read(&mut self, _: u64, _: usize) -> ReadFuture<'_> {
        struct Guard(std::sync::Arc<std::sync::atomic::AtomicBool>);
        impl Drop for Guard {
            fn drop(&mut self) {
                self.0.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }
        let guard = Guard(self.dropped.clone());
        Box::pin(async move {
            let _guard = guard;
            std::future::pending().await
        })
    }
}
#[test]
fn dropping_extraction_drops_in_flight_transport_read() {
    use std::task::{Context, Poll, Waker};
    let dropped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut reader = Pending {
        dropped: dropped.clone(),
    };
    let limits = Limits::default();
    let cancel = Cancellation::default();
    let mut future = Box::pin(extract(&mut reader, &limits, &cancel));
    let waker = Waker::noop();
    assert!(matches!(
        future.as_mut().poll(&mut Context::from_waker(waker)),
        Poll::Pending
    ));
    cancel.cancel();
    assert!(!dropped.load(std::sync::atomic::Ordering::SeqCst));
    drop(future);
    assert!(dropped.load(std::sync::atomic::Ordering::SeqCst));
}
