# metadata-connect-rs

Native Rust audio metadata extraction through caller-owned asynchronous range
readers. Port of [chrisle/metadata-connect](https://github.com/chrisle/metadata-connect)
v1.2.2 at `c2ae262650a7bbe7af5fb5222a6ab6b882ecf865`.
MIT license and copyright attribution are retained in LICENSE.

Supported aliases, in original order: `mp3`, `m4a`, `mp4`, `m4p`, `m4b`, `aac`,
`flac`, `aiff`, `aif`, `aifc`. AAC means the original MP4-container parser;
raw ADTS/AAC, WAV, Ogg and ID3v1 are not additional supported formats.

Fields: optional title, artist, album, genre, label/publisher, year, BPM, musical
key, artwork bytes and JPEG/PNG/GIF MIME. ID3v2.2/2.3/2.4 text/picture frames,
FLAC comments/pictures, MP4 iTunes atoms/freeform labels, and AIFF/AIFC embedded
ID3 are supported. All 80 original numeric genres are retained. Artwork uses the
latest front cover when available, otherwise the first other picture; MP4 keeps
its first cover atom. Format parsers can also be called directly.

## Usage

```rust
use metadata_connect_rs::{BufferReader, Cancellation, Limits, extract};

async fn metadata(bytes: &[u8]) -> Result<(), metadata_connect_rs::Error> {
    let mut reader = BufferReader::new(bytes, "mp3");
    if let Some(tags) = extract(&mut reader, &Limits::default(),
                                &Cancellation::default()).await? {
        println!("{:?}: {:?}", tags.artist, tags.title);
    }
    Ok(())
}
```

Implement `RangeReader` for a remote transport, caller-opened local file, or other
source. Its `size()` is the complete logical size, `extension()` selects a parser,
and `read(offset, length)` returns only the requested bounded bytes. The parser
skips audio data instead of downloading it: a synthetic sparse 5 GiB MP4 test
reads less than 4 KiB of metadata. This crate creates no sockets, opens no file
paths, starts no worker, performs no discovery, and writes no files.

`extract` and `extract_from_mp3/mp4/flac/aiff` return typed errors;
`extract_compatible` maps failures to `None` as original `extractMetadata` does.
Unsupported extensions and absent tags return `None`. Diagnostics are returned
as errors rather than emitted through the original formatted logger hook.

## Bounds and cancellation

Default limits: 16 MiB per read, 32 MiB total requested bytes, 4096 reads, 16384
parsed items, 256 KiB per text field and 8 MiB per artwork. Configuration itself
has finite hard maxima (64 MiB/read, 128 MiB total, 65536 reads/items, 1 MiB text,
32 MiB artwork). Offset/end arithmetic is checked and atoms must remain inside
parents. Responses must have exactly the requested length: oversized responses
are transport contract failures; short reads return an error.

These limits are not an exact heap cap. Input buffers, decoded text, and selected
artwork can coexist, including front-cover/fallback candidates. Requested bytes
are charged before each read and parse items before iteration. A transport must
honor read bounds before allocating; a reader that allocates a whole file before
returning cannot be made safe by checking its response afterward.

The cancellation token is cooperative: checked before and after each read and
while parsing. It does not wake an indefinitely pending transport future. The
caller supplies network deadlines or races/drops the extraction future; dropping
it drops the in-flight transport read future, verified by a regression test.
Readers remain responsible for canceling their own underlying operation and
rejecting stale completions. The crate does not own a runtime or detached task.

## Intentional differences

* MP4 atom identifiers compare raw four-byte values. The original ASCII decoder
  masked `©` to `)`, preventing title/artist/album/genre/year atoms from matching.
  Native parsing extracts them correctly; the oracle records both outcomes.
* ID3v2.2 PIC uses its actual three-byte image format instead of APIC's
  NUL-terminated MIME layout. The original misparsed it; the corrected vector is
  separately recorded.
* Invalid lengths, parent escapes, arithmetic overflow and resource exhaustion
  return errors; no partial tag from malformed frames is fabricated. Empty tag
  results become `None` rather than a JavaScript object containing only undefined
  fields. BufferReader's public reads still clamp at EOF like the original.
* Invalid UTF-8/UTF-16 uses replacement characters, as supported Rust strings
  cannot retain lone JS surrogate code units. Unsynchronization, compressed and
  encrypted ID3 frames are not newly implemented; the port follows the original
  supported plain-frame behavior.

## Verification

```sh
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
```

Eleven native tests cover the shared 24-vector corpus, all aliases/utilities,
artwork selection, read/count/text/artwork limits, errors, all-format truncation
and header mutation, sparse multi-gigabyte reads, malicious `u64::MAX` offsets,
and dropping an in-flight read. All fixtures are synthetic and redistributable.
No device, production capture or platform transport is represented by these tests.

The optional TypeScript oracle runs the original two test files unchanged (30
assertion cases under a restricted Vitest-compatible shim) and original parsers
against 24 shared fixtures: 22 parity cases plus two corrections. It checks the
source Git revision and does not open sockets. It needs Node and TypeScript only
for development; the Rust library has no Node runtime dependency.

From a standalone checkout:

```sh
npm ci --prefix scripts --ignore-scripts
git clone --no-checkout https://github.com/chrisle/metadata-connect.git scripts/original
git -C scripts/original checkout c2ae262650a7bbe7af5fb5222a6ab6b882ecf865
METADATA_SOURCE=scripts/original node scripts/oracle.mjs --check
```

In the parent workspace the source defaults to `packages/metadata-connect`.
`METADATA_SOURCE` overrides that path; `METADATA_TYPESCRIPT` can point to an
already installed TypeScript module instead of `scripts/node_modules/typescript`.
Fixtures can be regenerated by omitting `--check`, but review all resulting
changes against the pinned source before committing.

Application adoption, remote-reader integration and installed desktop/hardware
acceptance are separate from this standalone parser port. No crates.io release
is implied by this source repository.
