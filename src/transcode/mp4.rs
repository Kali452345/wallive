//! Just enough MP4 box walking for one Media Foundation workaround: its MP4
//! source applies a track's edit list (`edts`) to the empty sample table of
//! a fragmented file (DASH / YouTube downloads) and then reads no frames at
//! all. Such files import from a copy with the edit lists turned into
//! `free` boxes; import re-times every frame from zero anyway.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

struct BoxHeader {
    kind: [u8; 4],
    /// File offset of the box.
    pos: u64,
    header: u64,
    size: u64,
}

/// The box at `pos`, or `None` past `end` or if the header is malformed.
fn read_header<R: Read + Seek>(r: &mut R, pos: u64, end: u64) -> io::Result<Option<BoxHeader>> {
    if pos + 8 > end {
        return Ok(None);
    }
    r.seek(SeekFrom::Start(pos))?;
    let mut h = [0u8; 8];
    r.read_exact(&mut h)?;
    let kind = [h[4], h[5], h[6], h[7]];
    let (header, size) = match u32::from_be_bytes([h[0], h[1], h[2], h[3]]) {
        0 => (8, end - pos),
        1 => {
            if pos + 16 > end {
                return Ok(None);
            }
            let mut large = [0u8; 8];
            r.read_exact(&mut large)?;
            (16, u64::from_be_bytes(large))
        }
        n => (8, u64::from(n)),
    };
    if size < header || size > end - pos {
        return Ok(None);
    }
    Ok(Some(BoxHeader {
        kind,
        pos,
        header,
        size,
    }))
}

fn children<R: Read + Seek>(r: &mut R, parent: &BoxHeader) -> io::Result<Vec<BoxHeader>> {
    let end = parent.pos + parent.size;
    let mut pos = parent.pos + parent.header;
    let mut out = Vec::new();
    while let Some(b) = read_header(r, pos, end)? {
        pos += b.size;
        out.push(b);
    }
    Ok(out)
}

/// File offsets of the type field of every track's `edts` box, if the file
/// is a fragmented MP4 (its `moov` has an `mvex`); empty otherwise.
pub fn fragment_edit_lists<R: Read + Seek>(r: &mut R) -> io::Result<Vec<u64>> {
    let end = r.seek(SeekFrom::End(0))?;
    let mut pos = 0;
    while let Some(b) = read_header(r, pos, end)? {
        if &b.kind == b"moov" {
            let kids = children(r, &b)?;
            if !kids.iter().any(|k| &k.kind == b"mvex") {
                return Ok(Vec::new());
            }
            let mut out = Vec::new();
            for trak in kids.iter().filter(|k| &k.kind == b"trak") {
                for c in children(r, trak)? {
                    if &c.kind == b"edts" {
                        out.push(c.pos + 4);
                    }
                }
            }
            return Ok(out);
        }
        pos += b.size;
    }
    Ok(Vec::new())
}

/// Copies `src` to `dst` with every fragment edit list turned into a `free`
/// box. Returns `false` and writes nothing if `src` has none.
pub fn copy_without_fragment_edit_lists(src: &Path, dst: &Path) -> io::Result<bool> {
    let offsets = fragment_edit_lists(&mut io::BufReader::new(File::open(src)?))?;
    if offsets.is_empty() {
        return Ok(false);
    }
    std::fs::copy(src, dst)?;
    let mut f = OpenOptions::new().write(true).open(dst)?;
    for offset in offsets {
        f.seek(SeekFrom::Start(offset))?;
        f.write_all(b"free")?;
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn mp4_box(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(kind);
        v.extend_from_slice(body);
        v
    }

    fn file(fragmented: bool, edit_list: bool) -> Vec<u8> {
        let mut trak = mp4_box(b"tkhd", &[0; 20]);
        if edit_list {
            trak.extend(mp4_box(b"edts", &mp4_box(b"elst", &[0; 20])));
        }
        trak.extend(mp4_box(b"mdia", &[0; 16]));
        let mut moov = mp4_box(b"mvhd", &[0; 20]);
        if fragmented {
            moov.extend(mp4_box(b"mvex", &mp4_box(b"trex", &[0; 24])));
        }
        moov.extend(mp4_box(b"trak", &trak));
        let mut f = mp4_box(b"ftyp", b"dash\0\0\0\0iso6");
        f.extend(mp4_box(b"moov", &moov));
        f.extend(mp4_box(b"moof", &[0; 32]));
        f.extend(mp4_box(b"mdat", &[0; 64]));
        f
    }

    #[test]
    fn finds_edit_list_in_fragmented_file() {
        let f = file(true, true);
        let found = fragment_edit_lists(&mut Cursor::new(&f)).unwrap();
        assert_eq!(found.len(), 1);
        let at = found[0] as usize;
        assert_eq!(&f[at..at + 4], b"edts");
    }

    #[test]
    fn ignores_plain_and_edit_free_files() {
        for f in [
            file(false, true),
            file(true, false),
            Vec::new(),
            vec![0xff; 7],
        ] {
            assert!(
                fragment_edit_lists(&mut Cursor::new(&f))
                    .unwrap()
                    .is_empty()
            );
        }
    }

    #[test]
    fn stops_at_malformed_sizes() {
        let mut f = file(true, true);
        // Top-level box claiming to be larger than the file.
        f.extend_from_slice(&u32::MAX.to_be_bytes());
        f.extend_from_slice(b"junk");
        assert_eq!(fragment_edit_lists(&mut Cursor::new(&f)).unwrap().len(), 1);
        let mut bad = file(true, true);
        bad[0..4].copy_from_slice(&3u32.to_be_bytes()); // smaller than a header
        assert!(
            fragment_edit_lists(&mut Cursor::new(&bad))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn copy_renames_only_the_edit_list() {
        let dir = std::env::temp_dir().join(format!("wallive-mp4-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (src, dst) = (dir.join("in.mp4"), dir.join("out.mp4"));
        let f = file(true, true);
        std::fs::write(&src, &f).unwrap();
        assert!(copy_without_fragment_edit_lists(&src, &dst).unwrap());
        let out = std::fs::read(&dst).unwrap();
        let at = fragment_edit_lists(&mut Cursor::new(&f)).unwrap()[0] as usize;
        assert_eq!(&out[at..at + 4], b"free");
        assert_eq!(out.len(), f.len());
        assert_eq!(&out[..at], &f[..at]);
        assert_eq!(&out[at + 4..], &f[at + 4..]);
        assert!(std::fs::read(&src).unwrap() == f, "source untouched");

        std::fs::write(&src, file(false, true)).unwrap();
        let _ = std::fs::remove_file(&dst);
        assert!(!copy_without_fragment_edit_lists(&src, &dst).unwrap());
        assert!(!dst.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
