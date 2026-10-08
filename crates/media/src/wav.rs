//! Minimal WAV (RIFF/RF64 PCM 8/16/24/32-bit int, 32/64-bit float) source, with the Broadcast Wave
//! start time: `bext` TimeReference (EBU Tech 3285), else iXML `BWF_TIME_REFERENCE_LOW/HIGH`. The
//! full `riff` crate (all BWF metadata, AIFF) will replace this.
//!
//! The source reads through a [`ByteReader`](crate::ByteReader): opening a file reads its chunk headers, and audio is
//! read from the `data` chunk as it is asked for. A multi-hour recording costs a file handle, not
//! its size in memory.

use std::sync::Arc;

use filmcraft_frame::{AudioBuffer, VideoFrame};
use filmcraft_time::Tick;

use crate::reader::{MemReader, SharedReader};
use crate::{AudioStreamInfo, FrameRequest, MediaError, MediaInfo, MediaKind, MediaSource, Result};

pub fn sniff(b: &[u8]) -> bool {
    b.len() >= 12 && (&b[0..4] == b"RIFF" || &b[0..4] == b"RF64") && &b[8..12] == b"WAVE"
}

/// Chunks walked before giving up on finding `fmt ` / `data` (a hostile file of empty chunks).
const MAX_CHUNKS: usize = 100_000;
/// The largest iXML chunk read for its time reference.
const MAX_IXML: u64 = 1 << 20;
/// The most sample bytes one [`MediaSource::audio`] call reads.
const MAX_READ: u64 = 256 << 20;

pub struct WavSource {
    info: MediaInfo,
    reader: SharedReader,
    data_off: u64,
    /// Whole sample frames in the `data` chunk.
    frames: u64,
    channels: usize,
    bits: u16,
    float: bool,
    rate: u32,
    time_reference: Option<u64>,
}

fn le16(b: &[u8], o: usize) -> u16 {
    match b.get(o..o + 2) {
        Some(&[a, c]) => u16::from_le_bytes([a, c]),
        _ => 0,
    }
}
fn le32(b: &[u8], o: usize) -> u32 {
    match b.get(o..o + 4) {
        Some(&[a, c, d, e]) => u32::from_le_bytes([a, c, d, e]),
        _ => 0,
    }
}

impl WavSource {
    /// A WAV file held in memory.
    pub fn parse(name: &str, bytes: Arc<[u8]>) -> Result<Self> {
        Self::open_reader(name, Arc::new(MemReader(bytes)))
    }

    /// A WAV file read through `reader`: only the chunk headers (and `fmt `, `bext`, `iXML`) now.
    pub fn open_reader(name: &str, reader: SharedReader) -> Result<Self> {
        let size = reader.len();
        let io = |e: std::io::Error| MediaError::Io(format!("{name}: {e}"));
        let read = |at: u64, n: u64| -> Result<Vec<u8>> {
            let n = usize::try_from(n.min(size.saturating_sub(at))).map_err(|_| MediaError::Decode(format!("{name}: chunk too large")))?;
            let mut v = vec![0u8; n];
            reader.read_at(at, &mut v).map_err(io)?;
            Ok(v)
        };
        let mut pos: u64 = 12;
        let mut fmt = None;
        let mut data = None;
        let mut bext_tr = None;
        let mut ixml_tr = None;
        for _ in 0..MAX_CHUNKS {
            if pos.saturating_add(8) > size {
                break;
            }
            let h = read(pos, 8)?;
            let Some(id) = h.get(0..4).and_then(|i| <[u8; 4]>::try_from(i).ok()) else { break };
            let mut len = le32(&h, 4) as u64;
            let body = pos + 8;
            if &id == b"data" && (len == 0xFFFF_FFFF || body + len > size) {
                len = size - body;
            }
            if body + len > size {
                break;
            }
            match &id {
                b"fmt " if len >= 16 => {
                    let f = read(body, len.min(64))?;
                    let mut tag = le16(&f, 0);
                    let ch = le16(&f, 2);
                    let rate = le32(&f, 4);
                    let bits = le16(&f, 14);
                    if tag == 0xFFFE && len >= 40 {
                        tag = le16(&f, 24);
                    }
                    fmt = Some((tag, ch, rate, bits));
                }
                b"data" => data = Some((body, len)),
                // bext: Description 256, Originator 32, OriginatorReference 32, OriginationDate 10,
                // OriginationTime 8, then TimeReference (low u32, high u32)
                b"bext" if len >= 346 => {
                    let t = read(body + 338, 8)?;
                    bext_tr = Some(le32(&t, 0) as u64 | (le32(&t, 4) as u64) << 32);
                }
                b"iXML" if len <= MAX_IXML => ixml_tr = ixml_time_reference(&read(body, len)?),
                _ => {}
            }
            pos = body + len + (len & 1);
        }
        let (tag, ch, rate, bits) = fmt.ok_or_else(|| MediaError::Decode(format!("{name}: missing fmt chunk")))?;
        let (off, len) = data.ok_or_else(|| MediaError::Decode(format!("{name}: missing data chunk")))?;
        let float = tag == 3;
        if !(tag == 1 || float) || ch == 0 || rate == 0 || !matches!(bits, 8 | 16 | 24 | 32 | 64) {
            return Err(MediaError::Unsupported(format!("{name}: WAV format tag {tag}, {bits} bits")));
        }
        let frame_bytes = ch as u64 * bits as u64 / 8;
        let frames = len / frame_bytes;
        let time_reference = bext_tr.or(ixml_tr);
        let mut info = MediaInfo {
            name: name.into(),
            kind: MediaKind::AudioOnly,
            duration: Tick::from_units(i64::try_from(frames).unwrap_or(i64::MAX), rate as i64),
            video: None,
            audio: Some(AudioStreamInfo {
                sample_rate: rate,
                channels: ch as u32,
                codec: if float { "PCM float".into() } else { "PCM".into() },
                bits_per_sample: Some(bits as u32),
            }),
            container: if time_reference.is_some() { "Broadcast WAV".into() } else { "WAV".into() },
            start_timecode: None,
            file_size: Some(size),
        };
        // start timecode: frames at the media's frame rate (the default rate for audio-only media)
        if let Some(tr) = time_reference {
            let r = info.frame_rate();
            let n = tr as i128 * r.num as i128;
            let d = rate as i128 * r.den as i128;
            if d > 0 {
                info.start_timecode = i64::try_from((n + d / 2) / d).ok();
            }
        }
        Ok(Self { info, reader, data_off: off, frames, channels: ch as usize, bits, float, rate, time_reference })
    }

    /// Broadcast Wave start time: samples since midnight.
    pub fn time_reference(&self) -> Option<u64> {
        self.time_reference
    }

    fn frame_bytes(&self) -> usize {
        self.channels * self.bits as usize / 8
    }

    /// Sample `ch` of frame `frame` within `b`, the bytes of the frames read from `first`.
    fn sample(&self, b: &[u8], first: u64, frame: u64, ch: usize) -> f32 {
        let bps = self.bits as usize / 8;
        let Some(o) =
            usize::try_from(frame - first).ok().and_then(|f| f.checked_mul(self.channels)).and_then(|i| i.checked_add(ch)).and_then(|i| i.checked_mul(bps))
        else {
            return 0.0;
        };
        let Some(s) = b.get(o..o + bps) else { return 0.0 };
        match (self.bits, self.float, s) {
            (8, _, &[a]) => (a as f32 - 128.0) / 128.0,
            (16, _, &[a, c]) => i16::from_le_bytes([a, c]) as f32 / 32768.0,
            (24, _, &[a, c, d]) => ((i32::from_le_bytes([0, a, c, d]) >> 8) as f32) / 8_388_608.0,
            (32, false, &[a, c, d, e]) => i32::from_le_bytes([a, c, d, e]) as f32 / 2_147_483_648.0,
            (32, true, &[a, c, d, e]) => f32::from_le_bytes([a, c, d, e]),
            (64, true, s) => s.try_into().map(f64::from_le_bytes).unwrap_or(0.0) as f32,
            _ => 0.0,
        }
    }
}

impl MediaSource for WavSource {
    fn info(&self) -> &MediaInfo {
        &self.info
    }
    fn video_frame(&self, _req: FrameRequest) -> Result<Arc<VideoFrame>> {
        Err(MediaError::NoStream("video"))
    }
    fn audio(&self, start: i64, frames: usize, sample_rate: u32) -> Result<AudioBuffer> {
        let mut out = AudioBuffer::silence(sample_rate, self.channels, frames);
        if frames == 0 || sample_rate == 0 || self.frames == 0 {
            return Ok(out);
        }
        // Linear-interpolating resample when rates differ (the audio crate provides sinc resampling).
        let ratio = self.rate as f64 / sample_rate as f64;
        let src_of = |i: usize| (start as f64 + i as f64) * ratio;
        // the source frames this call touches, read in one go
        let lo = src_of(0).floor().max(0.0);
        let hi = (src_of(frames - 1).floor() + 2.0).max(0.0);
        let first = (lo as u64).min(self.frames);
        let end = (hi as u64).min(self.frames);
        if end <= first {
            return Ok(out);
        }
        let fb = self.frame_bytes() as u64;
        let span = (end - first).saturating_mul(fb);
        if span > MAX_READ {
            return Err(MediaError::Decode(format!("{}: audio request too large", self.info.name)));
        }
        let mut b = vec![0u8; span as usize];
        self.reader.read_at(self.data_off + first * fb, &mut b).map_err(|e| MediaError::Io(format!("{}: {e}", self.info.name)))?;
        for i in 0..frames {
            let src = src_of(i);
            if src < 0.0 {
                continue;
            }
            let i0 = src.floor() as u64;
            if i0 >= end {
                break;
            }
            let fr = (src - i0 as f64) as f32;
            for c in 0..self.channels {
                let a = self.sample(&b, first, i0, c);
                let n = if i0 + 1 < end { self.sample(&b, first, i0 + 1, c) } else { a };
                if let Some(v) = out.channels.get_mut(c).and_then(|ch| ch.get_mut(i)) {
                    *v = a + (n - a) * fr;
                }
            }
        }
        Ok(out)
    }
}

/// TimeReference from an iXML chunk (`<BWF_TIME_REFERENCE_LOW>` / `<BWF_TIME_REFERENCE_HIGH>`).
fn ixml_time_reference(x: &[u8]) -> Option<u64> {
    let text = String::from_utf8_lossy(x);
    let field = |tag: &str| -> Option<u64> {
        let open = format!("<{tag}>");
        let a = text.find(&open)? + open.len();
        let b = a + text[a..].find('<')?;
        text[a..b].trim().parse().ok()
    };
    let low = field("BWF_TIME_REFERENCE_LOW")?;
    Some(low | field("BWF_TIME_REFERENCE_HIGH").unwrap_or(0) << 32)
}

/// Encode interleaved f32 samples as a 16-bit PCM WAV file.
pub fn write_wav16(samples: &[f32], channels: u16, rate: u32) -> Vec<u8> {
    let data_len = samples.len() * 2;
    let mut v = Vec::with_capacity(44 + data_len);
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
    v.extend_from_slice(b"WAVEfmt ");
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&channels.to_le_bytes());
    v.extend_from_slice(&rate.to_le_bytes());
    v.extend_from_slice(&(rate * channels as u32 * 2).to_le_bytes());
    v.extend_from_slice(&(channels * 2).to_le_bytes());
    v.extend_from_slice(&16u16.to_le_bytes());
    v.extend_from_slice(b"data");
    v.extend_from_slice(&(data_len as u32).to_le_bytes());
    for s in samples {
        v.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes());
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let s: Vec<f32> = (0..200).map(|i| ((i as f32) * 0.1).sin() * 0.5).collect();
        let bytes = write_wav16(&s, 2, 48000);
        let src = WavSource::parse("t.wav", bytes.into()).unwrap();
        assert_eq!(src.info().audio.as_ref().unwrap().channels, 2);
        let a = src.audio(0, 100, 48000).unwrap();
        assert!((a.channels[0][3] - s[6]).abs() < 1e-4);
        assert!((a.channels[1][3] - s[7]).abs() < 1e-4);
        assert_eq!(src.info().duration, Tick::from_units(100, 48000));
        assert_eq!(src.time_reference(), None);
        assert_eq!(src.info().start_timecode, None);
    }

    /// Insert a chunk before `data`.
    fn with_chunk(wav: &[u8], id: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = wav[..36].to_vec();
        v.extend_from_slice(id);
        v.extend_from_slice(&(body.len() as u32).to_le_bytes());
        v.extend_from_slice(body);
        if body.len() % 2 == 1 {
            v.push(0);
        }
        v.extend_from_slice(&wav[36..]);
        let riff = (v.len() - 8) as u32;
        v[4..8].copy_from_slice(&riff.to_le_bytes());
        v
    }

    #[test]
    fn bwf_time_reference_from_bext() {
        let wav = write_wav16(&[0.0; 96], 2, 48_000);
        // 01:00:00:00 at 23.976 fps: 86 400 frames of 2002 samples
        let tr: u64 = 86_400 * 2002;
        let mut bext = vec![0u8; 602];
        bext[338..342].copy_from_slice(&(tr as u32).to_le_bytes());
        bext[342..346].copy_from_slice(&((tr >> 32) as u32).to_le_bytes());
        let src = WavSource::parse("bwf.wav", with_chunk(&wav, b"bext", &bext).into()).unwrap();
        assert_eq!(src.time_reference(), Some(tr));
        assert_eq!(src.info().frame_rate(), filmcraft_time::FrameRate::FPS_23_976);
        assert_eq!(src.info().start_timecode, Some(86_400));
        assert_eq!(src.info().container, "Broadcast WAV");
        // the audio is unaffected
        assert_eq!(src.info().duration, Tick::from_units(48, 48_000));
    }

    #[test]
    fn bwf_time_reference_from_ixml_and_large_values() {
        let wav = write_wav16(&[0.0; 4], 1, 48_000);
        let tr: u64 = (1 << 32) + 5;
        let xml = format!(
            "<?xml version=\"1.0\"?><BWFXML><BEXT><BWF_TIME_REFERENCE_LOW>{}</BWF_TIME_REFERENCE_LOW><BWF_TIME_REFERENCE_HIGH>{}</BWF_TIME_REFERENCE_HIGH></BEXT></BWFXML>",
            tr & 0xFFFF_FFFF,
            tr >> 32
        );
        let src = WavSource::parse("ixml.wav", with_chunk(&wav, b"iXML", xml.as_bytes()).into()).unwrap();
        assert_eq!(src.time_reference(), Some(tr));
        // a short bext (no TimeReference) is ignored
        let src = WavSource::parse("short.wav", with_chunk(&wav, b"bext", &[0u8; 100]).into()).unwrap();
        assert_eq!(src.time_reference(), None);
    }

    /// A WAV of `len` bytes that exists only as its header (the `data` chunk runs to the end of
    /// the file and reads as silence), counting the bytes read.
    pub(crate) struct HugeWav {
        header: Vec<u8>,
        len: u64,
        pub(crate) read: std::sync::atomic::AtomicU64,
    }

    impl HugeWav {
        pub(crate) fn new(len: u64) -> Self {
            let mut header = write_wav16(&[], 2, 48_000);
            // an unknown data size: the chunk runs to the end of the file (as recorders leave it)
            header[40..44].copy_from_slice(&u32::MAX.to_le_bytes());
            Self { header, len, read: Default::default() }
        }
    }

    impl crate::reader::ByteReader for HugeWav {
        fn len(&self) -> u64 {
            self.len
        }
        fn read_at(&self, offset: u64, buf: &mut [u8]) -> std::io::Result<()> {
            if offset + buf.len() as u64 > self.len {
                return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "past the end"));
            }
            self.read.fetch_add(buf.len() as u64, std::sync::atomic::Ordering::Relaxed);
            for (i, b) in buf.iter_mut().enumerate() {
                *b = self.header.get(offset as usize + i).copied().unwrap_or(0);
            }
            Ok(())
        }
    }

    /// Opening a project that linked a multi-gigabyte recording read the whole file into memory
    /// (twice over), and the system ran out of memory. A WAV now costs its headers and the samples
    /// asked for.
    #[test]
    fn a_huge_wav_opens_and_plays_without_being_read_whole() {
        let huge = Arc::new(HugeWav::new(40 << 30));
        let src = WavSource::open_reader("long.wav", huge.clone()).unwrap();
        let frames = ((40u64 << 30) - 44) / 4;
        assert_eq!(src.info().duration, Tick::from_units(frames as i64, 48_000));
        assert_eq!(src.info().file_size, Some(40 << 30));
        // a second of audio from three hours in
        let a = src.audio(3 * 3600 * 48_000, 48_000, 48_000).unwrap();
        assert_eq!((a.channels.len(), a.channels[0].len()), (2, 48_000));
        // and past the end: silence, no error
        assert!(src.audio(frames as i64 + 10, 100, 48_000).unwrap().channels[0].iter().all(|&v| v == 0.0));
        let read = huge.read.load(std::sync::atomic::Ordering::Relaxed);
        assert!(read < 1 << 20, "read {read} bytes");
    }

    #[test]
    fn audio_read_through_a_reader_matches_and_resamples() {
        let s: Vec<f32> = (0..2000).map(|i| ((i as f32) * 0.01).sin() * 0.5).collect();
        let src = WavSource::parse("t.wav", write_wav16(&s, 1, 48_000).into()).unwrap();
        let a = src.audio(500, 10, 48_000).unwrap();
        for (i, v) in a.channels[0].iter().enumerate() {
            assert!((v - s[500 + i]).abs() < 1e-4);
        }
        // half the rate: every other source sample
        let h = src.audio(100, 10, 24_000).unwrap();
        for (i, v) in h.channels[0].iter().enumerate() {
            assert!((v - s[200 + 2 * i]).abs() < 1e-4);
        }
        // a request starting before the file: silence, then the first samples
        let e = src.audio(-5, 10, 48_000).unwrap();
        assert_eq!(e.channels[0][..5], [0.0; 5]);
        assert!((e.channels[0][5] - s[0]).abs() < 1e-4);
    }
}
