//! Opening a project that links very large media must not read the media into memory: a
//! Premiere project linking a 31.7 GB screen recording and 3.5 GB of WAVs took 46 GB of memory and
//! 50 GB of swap before the kernel killed FilmCraft (and froze the desktop on the way).

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::json;

use crate::media_test_util::tmp_dir;
use crate::{FsServices, Services, Session};

const LEN: u64 = 40 << 30;

/// The real filesystem, except that `*.wav` files are 40 GB recordings that exist only as a WAV
/// header (a `data` chunk running to the end of the file): reading one whole is an error, and
/// every byte read from them is counted.
struct HugeWavs {
    header: Vec<u8>,
    read: Arc<AtomicU64>,
    whole_reads: Arc<AtomicU64>,
}

impl HugeWavs {
    fn new() -> Self {
        let mut header = filmcraft_media::wav::write_wav16(&[], 2, 48_000);
        header[40..44].copy_from_slice(&u32::MAX.to_le_bytes());
        Self { header, read: Default::default(), whole_reads: Default::default() }
    }
}

struct HugeReader {
    header: Vec<u8>,
    read: Arc<AtomicU64>,
}

impl filmcraft_media::ByteReader for HugeReader {
    fn len(&self) -> u64 {
        LEN
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> std::io::Result<()> {
        if offset + buf.len() as u64 > LEN {
            return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "past the end"));
        }
        self.read.fetch_add(buf.len() as u64, Ordering::Relaxed);
        for (i, b) in buf.iter_mut().enumerate() {
            *b = self.header.get(offset as usize + i).copied().unwrap_or(0);
        }
        Ok(())
    }
}

fn is_huge(path: &str) -> bool {
    path.ends_with(".wav")
}

impl Services for HugeWavs {
    fn read_file(&self, path: &str) -> std::io::Result<Vec<u8>> {
        if is_huge(path) {
            self.whole_reads.fetch_add(1, Ordering::Relaxed);
            return Err(std::io::Error::other(format!("{path}: read whole")));
        }
        FsServices.read_file(path)
    }
    fn write_file(&self, path: &str, data: &[u8]) -> std::io::Result<()> {
        FsServices.write_file(path, data)
    }
    fn file_size(&self, path: &str) -> std::io::Result<u64> {
        if is_huge(path) { Ok(LEN) } else { FsServices.file_size(path) }
    }
    fn read_range(&self, path: &str, offset: u64, len: usize) -> std::io::Result<Vec<u8>> {
        if !is_huge(path) {
            return FsServices.read_range(path, offset, len);
        }
        let n = (LEN.saturating_sub(offset)).min(len as u64) as usize;
        let mut v = vec![0u8; n];
        filmcraft_media::ByteReader::read_at(&HugeReader { header: self.header.clone(), read: self.read.clone() }, offset, &mut v)?;
        Ok(v)
    }
    fn reader(&self, path: &str) -> Option<std::io::Result<filmcraft_media::SharedReader>> {
        if is_huge(path) {
            return Some(Ok(Arc::new(HugeReader { header: self.header.clone(), read: self.read.clone() })));
        }
        FsServices.reader(path)
    }
}

const RATE: &str = "<rate><timebase>24</timebase><ntsc>FALSE</ntsc></rate>";

fn xml_linking(file: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE xmeml>
<xmeml version="4">
  <sequence id="sequence-1">
    <name>Cut</name>
    <duration>240</duration>
    {RATE}
    <media>
      <audio>
        <track>
          <clipitem id="clipitem-1">
            <name>talk</name>
            {RATE}
            <start>0</start><end>240</end><in>0</in><out>240</out>
            <file id="file-1">
              <name>{file}</name>
              <pathurl>{file}</pathurl>
              {RATE}
              <duration>240</duration>
              <media><audio><samplecharacteristics><samplerate>48000</samplerate></samplecharacteristics></audio></media>
            </file>
          </clipitem>
        </track>
      </audio>
    </media>
  </sequence>
</xmeml>
"#
    )
}

#[test]
fn an_import_links_huge_media_without_reading_it_into_memory() {
    let d = tmp_dir("huge-media-link");
    let doc = d.join("cut.xml");
    std::fs::write(&doc, xml_linking("recording.wav")).unwrap();
    let host = HugeWavs::new();
    let (read, whole) = (host.read.clone(), host.whole_reads.clone());
    let mut s = Session::new(Arc::new(host));
    let r = s.execute("file.import", json!({"paths": [doc.to_string_lossy()]})).unwrap();
    assert_eq!(r["documents"][0]["linkedMedia"], 1, "{r}");
    assert_eq!(whole.load(Ordering::Relaxed), 0, "a linked file was read whole");
    let read = read.load(Ordering::Relaxed);
    assert!(read < 8 << 20, "read {read} bytes of a 40 GB file");
    // the linked item knows the file's real length
    let item = s.project.items.values().find(|i| i.name.contains("recording")).unwrap();
    let filmcraft_project::ItemKind::Media(m) = &item.kind else { panic!("not media") };
    assert!(!m.offline);
    assert_eq!(m.info.file_size, Some(LEN));
    let _ = std::fs::remove_dir_all(&d);
}
