//! WAV parts: the 16-bit writer the recorder uses, header inspection, and header repair from the
//! file length after a crash.

use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::Error;

/// Bytes at the start of a part that may change after they are first written (the RIFF and data
/// sizes). hound's header is at most 68 bytes; the mirror rewrites this many on every pass.
pub const HEADER_REWRITE_BYTES: u64 = 4096;

/// Bytes before the first sample in a part the recorder writes (hound's 16-bit PCM header).
pub const PCM16_HEADER_BYTES: u64 = 44;

/// Writes one part: 16-bit PCM at the device rate, channels as captured.
pub struct PartWriter {
    path: PathBuf,
    writer: hound::WavWriter<BufWriter<File>>,
    /// Second handle on the same file, used for fsync (hound owns the first).
    sync_handle: File,
    channels: u16,
    sample_rate: u32,
    /// Whole frames accepted so far (handed to `write_f32` or `write_carry`).
    frames: u64,
    /// The recorder's own copy of the samples accepted since the last successful `flush`
    /// (about one second at most). If a write or flush fails, `take_salvage` hands it back, so
    /// the samples that never reached the disk can be written to the next part.
    pending: Vec<i16>,
    /// Frames at the start of this part that were carried over from a part that failed.
    carry_frames: u64,
}

/// What a part that failed still owes the recording (see `PartWriter::take_salvage`).
#[derive(Debug, Default)]
pub struct Salvage {
    /// Interleaved samples accepted since the last successful flush.
    pub pending: Vec<i16>,
    /// Whole frames accepted by the part in total.
    pub accepted_frames: u64,
    /// Frames at the start of the part that came from an earlier failed part.
    pub carry_frames: u64,
    pub channels: u16,
}

impl PartWriter {
    /// Creates the file (failing if it exists) and writes the header.
    pub fn create(path: &Path, sample_rate: u32, channels: u16) -> Result<Self, Error> {
        let file = OpenOptions::new()
            .write(true)
            .read(true)
            .create_new(true)
            .open(path)
            .map_err(|e| Error::io(path, e))?;
        let sync_handle = file.try_clone().map_err(|e| Error::io(path, e))?;
        let spec = hound::WavSpec {
            channels,
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        // 16 KiB: under 0.1 s of 48 kHz stereo (0.17 s mono). Audio reaches the OS file this
        // often even between flushes, so a killed process loses at most this much plus the
        // ring buffer's few milliseconds; recovery rebuilds the header from the file length.
        let writer = hound::WavWriter::new(BufWriter::with_capacity(16 * 1024, file), spec)
            .map_err(|e| Error::wav(path, e))?;
        Ok(Self {
            path: path.to_path_buf(),
            writer,
            sync_handle,
            channels,
            sample_rate,
            frames: 0,
            pending: Vec::new(),
            carry_frames: 0,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// Whole frames written so far.
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// Writes interleaved samples in [-1, 1]. `samples.len()` must be a multiple of the channel
    /// count; the recorder only passes whole frames.
    ///
    /// The samples count as accepted, and are kept in the pending copy, before the write is
    /// tried: if it fails part-way, `take_salvage` still holds all of them.
    pub fn write_f32(&mut self, samples: &[f32]) -> Result<(), Error> {
        let from = self.pending.len();
        self.pending.extend(samples.iter().map(|&s| f32_to_i16(s)));
        self.frames += (samples.len() / usize::from(self.channels.max(1))) as u64;
        self.write_pending_from(from)
    }

    /// Writes samples carried over from a part that failed, at the start of this part.
    pub fn write_carry(&mut self, samples: &[i16]) -> Result<(), Error> {
        let from = self.pending.len();
        self.pending.extend_from_slice(samples);
        let frames = (samples.len() / usize::from(self.channels.max(1))) as u64;
        self.frames += frames;
        self.carry_frames += frames;
        self.write_pending_from(from)
    }

    fn write_pending_from(&mut self, from: usize) -> Result<(), Error> {
        for i in from..self.pending.len() {
            self.writer
                .write_sample(self.pending[i])
                .map_err(|e| Error::wav(&self.path, e))?;
        }
        Ok(())
    }

    /// Hands back the pending copy and the counts needed to work out which samples reached the
    /// disk. Used after a failed write or flush.
    pub fn take_salvage(&mut self) -> Salvage {
        Salvage {
            pending: std::mem::take(&mut self.pending),
            accepted_frames: self.frames,
            carry_frames: self.carry_frames,
            channels: self.channels,
        }
    }

    /// Updates the header to the samples written so far and hands the bytes to the OS, so the
    /// file is a playable WAV up to this point.
    pub fn flush(&mut self) -> Result<(), Error> {
        self.writer.flush().map_err(|e| Error::wav(&self.path, e))?;
        // Everything accepted so far has been handed to the OS.
        self.pending.clear();
        Ok(())
    }

    /// Flushes, then asks the OS to put the data on the disk.
    pub fn sync(&mut self) -> Result<(), Error> {
        self.flush()?;
        self.sync_handle
            .sync_data()
            .map_err(|e| Error::io(&self.path, e))
    }

    /// Final header, flush and fsync. Returns (frames, sample_rate, channels).
    pub fn finish(self) -> Result<(u64, u32, u16), Error> {
        let Self {
            path,
            writer,
            sync_handle,
            channels,
            sample_rate,
            frames,
            ..
        } = self;
        writer.finalize().map_err(|e| Error::wav(&path, e))?;
        sync_handle.sync_all().map_err(|e| Error::io(&path, e))?;
        Ok((frames, sample_rate, channels))
    }
}

/// Converts a float sample to 16-bit with clamping. `i16 as f32 / 32768.0` round-trips exactly.
pub fn f32_to_i16(s: f32) -> i16 {
    let v = (s * 32768.0).round();
    if v >= 32767.0 {
        i16::MAX
    } else if v <= -32768.0 {
        i16::MIN
    } else {
        v as i16
    }
}

/// What a WAV file's header and length say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WavInfo {
    pub sample_rate: u32,
    pub channels: u16,
    pub bits_per_sample: u16,
    pub block_align: u16,
    /// Offset of the first sample byte.
    pub data_offset: u64,
    /// Data size the header claims.
    pub header_data_len: u32,
    /// RIFF size the header claims.
    pub header_riff_len: u32,
    /// Whole frames actually present in the file (from its length).
    pub frames_on_disk: u64,
    pub file_len: u64,
}

impl WavInfo {
    /// Duration in milliseconds of the frames on disk (rounded down).
    pub fn duration_ms(&self) -> u64 {
        frames_to_ms(self.frames_on_disk, self.sample_rate)
    }

    /// Bytes per second of audio.
    pub fn byte_rate(&self) -> u64 {
        u64::from(self.sample_rate) * u64::from(self.block_align)
    }

    /// True when the header sizes agree with the file length.
    pub fn header_matches_length(&self) -> bool {
        let data_len = self.frames_on_disk * u64::from(self.block_align);
        u64::from(self.header_data_len) == data_len
            && u64::from(self.header_riff_len) == self.data_offset - 8 + data_len
            && self.file_len == self.data_offset + data_len
    }
}

/// Milliseconds of `frames` at `rate`, rounded down.
pub fn frames_to_ms(frames: u64, rate: u32) -> u64 {
    if rate == 0 {
        0
    } else {
        frames * 1000 / u64::from(rate)
    }
}

fn le_u16(b: &[u8]) -> u16 {
    u16::from_le_bytes([b[0], b[1]])
}

fn le_u32(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

fn bad(path: &Path, msg: &str) -> Error {
    Error::BadWav {
        path: path.to_path_buf(),
        reason: msg.to_string(),
    }
}

/// Reads the RIFF chunks of a WAV file without trusting its size fields.
pub fn inspect(path: &Path) -> Result<WavInfo, Error> {
    let mut f = File::open(path).map_err(|e| Error::io(path, e))?;
    let file_len = f.metadata().map_err(|e| Error::io(path, e))?.len();
    let mut riff = [0u8; 12];
    f.read_exact(&mut riff)
        .map_err(|_| bad(path, "shorter than a RIFF header"))?;
    if &riff[0..4] != b"RIFF" || &riff[8..12] != b"WAVE" {
        return Err(bad(path, "not a RIFF WAVE file"));
    }
    let header_riff_len = le_u32(&riff[4..8]);
    let mut pos = 12u64;
    let mut fmt: Option<(u32, u16, u16, u16)> = None;
    loop {
        let mut ch = [0u8; 8];
        f.seek(SeekFrom::Start(pos))
            .map_err(|e| Error::io(path, e))?;
        f.read_exact(&mut ch)
            .map_err(|_| bad(path, "no data chunk"))?;
        let size = le_u32(&ch[4..8]);
        let body = pos + 8;
        if &ch[0..4] == b"fmt " {
            let mut body_bytes = [0u8; 16];
            f.read_exact(&mut body_bytes)
                .map_err(|_| bad(path, "short fmt chunk"))?;
            let channels = le_u16(&body_bytes[2..4]);
            let rate = le_u32(&body_bytes[4..8]);
            let block_align = le_u16(&body_bytes[12..14]);
            let bits = le_u16(&body_bytes[14..16]);
            if channels == 0 || rate == 0 || block_align == 0 {
                return Err(bad(path, "fmt chunk has zero channels, rate or block size"));
            }
            fmt = Some((rate, channels, bits, block_align));
        } else if &ch[0..4] == b"data" {
            let (sample_rate, channels, bits_per_sample, block_align) =
                fmt.ok_or_else(|| bad(path, "data chunk before fmt chunk"))?;
            let available = file_len.saturating_sub(body);
            return Ok(WavInfo {
                sample_rate,
                channels,
                bits_per_sample,
                block_align,
                data_offset: body,
                header_data_len: size,
                header_riff_len,
                frames_on_disk: available / u64::from(block_align),
                file_len,
            });
        }
        // Chunks are word aligned.
        pos = body + u64::from(size) + u64::from(size & 1);
        if pos >= file_len {
            return Err(bad(path, "no data chunk"));
        }
    }
}

/// What `repair_header` did.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct RepairReport {
    pub file: String,
    /// The header already matched the file length.
    pub already_valid: bool,
    /// Bytes of a trailing partial frame that were cut off.
    pub truncated_bytes: u64,
    pub frames: u64,
    pub sample_rate: u32,
    pub channels: u16,
}

/// Rewrites the RIFF and data sizes from the file length, cutting any trailing partial frame, so
/// a part left open by a crash plays in full. Idempotent.
pub fn repair_header(path: &Path) -> Result<RepairReport, Error> {
    let info = inspect(path)?;
    let file = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let data_len = info.frames_on_disk * u64::from(info.block_align);
    let new_len = info.data_offset + data_len;
    let truncated_bytes = info.file_len - new_len;
    if info.header_matches_length() {
        return Ok(RepairReport {
            file,
            already_valid: true,
            truncated_bytes: 0,
            frames: info.frames_on_disk,
            sample_rate: info.sample_rate,
            channels: info.channels,
        });
    }
    let data_len32 = u32::try_from(data_len).map_err(|_| bad(path, "data longer than 4 GiB"))?;
    let riff_len32 = u32::try_from(info.data_offset - 8 + data_len)
        .map_err(|_| bad(path, "file longer than 4 GiB"))?;
    let mut f = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|e| Error::io(path, e))?;
    if truncated_bytes > 0 {
        f.set_len(new_len).map_err(|e| Error::io(path, e))?;
    }
    f.seek(SeekFrom::Start(4)).map_err(|e| Error::io(path, e))?;
    f.write_all(&riff_len32.to_le_bytes())
        .map_err(|e| Error::io(path, e))?;
    f.seek(SeekFrom::Start(info.data_offset - 4))
        .map_err(|e| Error::io(path, e))?;
    f.write_all(&data_len32.to_le_bytes())
        .map_err(|e| Error::io(path, e))?;
    f.sync_all().map_err(|e| Error::io(path, e))?;
    Ok(RepairReport {
        file,
        already_valid: false,
        truncated_bytes,
        frames: info.frames_on_disk,
        sample_rate: info.sample_rate,
        channels: info.channels,
    })
}

/// Reads every sample of a 16-bit WAV (tests and analysis).
pub fn read_i16(path: &Path) -> Result<(hound::WavSpec, Vec<i16>), Error> {
    let mut reader = hound::WavReader::open(path).map_err(|e| Error::wav(path, e))?;
    let spec = reader.spec();
    let samples = reader
        .samples::<i16>()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| Error::wav(path, e))?;
    Ok((spec, samples))
}

/// Helper for tests in this crate and its dependents: write raw bytes.
pub fn append_bytes(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut f = OpenOptions::new().append(true).open(path)?;
    f.write_all(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    fn sine(frames: usize, channels: u16, rate: u32) -> Vec<i16> {
        let mut out = Vec::with_capacity(frames * usize::from(channels));
        for i in 0..frames {
            let t = i as f64 / f64::from(rate);
            for c in 0..channels {
                let f = 440.0 * f64::from(c + 1);
                out.push(((t * f * std::f64::consts::TAU).sin() * 20_000.0).round() as i16);
            }
        }
        out
    }

    #[test]
    fn writer_round_trip_is_sample_exact() -> Result<(), Box<dyn std::error::Error>> {
        let dir = TempDir::new("wav-roundtrip")?;
        for channels in [1u16, 2] {
            let path = dir.path().join(format!("part-{channels}.wav"));
            let src = sine(48_000 * 3 + 17, channels, 48_000);
            let floats: Vec<f32> = src.iter().map(|&s| f32::from(s) / 32768.0).collect();
            let mut w = PartWriter::create(&path, 48_000, channels)?;
            // Uneven chunks of whole frames, with a flush and a sync in between, as the recorder does.
            let step = usize::from(channels) * 977;
            for (i, chunk) in floats.chunks(step).enumerate() {
                w.write_f32(chunk)?;
                if i % 10 == 0 {
                    w.flush()?;
                }
                if i % 50 == 0 {
                    w.sync()?;
                }
            }
            let (frames, rate, ch) = w.finish()?;
            assert_eq!(
                (frames, rate, ch),
                ((48_000 * 3 + 17) as u64, 48_000, channels)
            );
            let (spec, back) = read_i16(&path)?;
            assert_eq!(spec.channels, channels);
            assert_eq!(spec.sample_rate, 48_000);
            assert_eq!(spec.bits_per_sample, 16);
            assert_eq!(back, src, "samples differ for {channels} channel(s)");
            assert!(inspect(&path)?.header_matches_length());
        }
        Ok(())
    }

    #[test]
    fn clamping() {
        assert_eq!(f32_to_i16(1.5), i16::MAX);
        assert_eq!(f32_to_i16(-1.5), i16::MIN);
        assert_eq!(f32_to_i16(-1.0), i16::MIN);
        assert_eq!(f32_to_i16(0.0), 0);
        assert_eq!(f32_to_i16(f32::from(1234i16) / 32768.0), 1234);
    }

    #[test]
    fn flushed_file_is_playable_before_close() -> Result<(), Box<dyn std::error::Error>> {
        let dir = TempDir::new("wav-flushed")?;
        let path = dir.path().join("part-0001.wav");
        let mut w = PartWriter::create(&path, 16_000, 1)?;
        w.write_f32(&vec![0.25; 16_000])?;
        w.flush()?;
        let info = inspect(&path)?;
        assert!(info.header_matches_length(), "{info:?}");
        assert_eq!(info.frames_on_disk, 16_000);
        drop(w);
        Ok(())
    }

    #[test]
    fn header_repair_on_truncated_file() -> Result<(), Box<dyn std::error::Error>> {
        let dir = TempDir::new("wav-repair")?;
        let path = dir.path().join("part-0001.wav");
        let src = sine(48_000, 2, 48_000);
        let mut w = PartWriter::create(&path, 48_000, 2)?;
        let floats: Vec<f32> = src.iter().map(|&s| f32::from(s) / 32768.0).collect();
        // One second flushed: the header says one second.
        w.write_f32(&floats)?;
        w.sync()?;
        // Simulate a crash: the process dies with the header still at one second while more data
        // (1.5 s plus half a frame) reached the file.
        drop(w);
        let extra = sine(72_000, 2, 48_000);
        let mut bytes: Vec<u8> = extra.iter().flat_map(|s| s.to_le_bytes()).collect();
        bytes.extend_from_slice(&[0x12, 0x34]); // half a stereo frame
        append_bytes(&path, &bytes)?;

        let before = inspect(&path)?;
        assert!(!before.header_matches_length());
        assert_eq!(before.header_data_len, 48_000 * 4);
        assert_eq!(before.frames_on_disk, 120_000);

        let report = repair_header(&path)?;
        assert!(!report.already_valid);
        assert_eq!(report.truncated_bytes, 2);
        assert_eq!(report.frames, 120_000);

        let after = inspect(&path)?;
        assert!(after.header_matches_length(), "{after:?}");
        let (_, back) = read_i16(&path)?;
        let mut expected = src.clone();
        expected.extend_from_slice(&extra);
        assert_eq!(back, expected);

        // Idempotent.
        assert!(repair_header(&path)?.already_valid);
        Ok(())
    }

    #[test]
    fn header_repair_when_header_was_never_updated() -> Result<(), Box<dyn std::error::Error>> {
        // A crash before the first flush leaves the zero sizes hound writes at creation.
        let dir = TempDir::new("wav-zero-header")?;
        let path = dir.path().join("part-0001.wav");
        let w = PartWriter::create(&path, 48_000, 1)?;
        let mut w = w;
        w.flush()?; // header only, zero data
        drop(w);
        let samples = sine(4_800, 1, 48_000);
        let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        append_bytes(&path, &bytes)?;
        let report = repair_header(&path)?;
        assert_eq!(report.frames, 4_800);
        let (_, back) = read_i16(&path)?;
        assert_eq!(back, samples);
        Ok(())
    }

    #[test]
    fn rejects_non_wav() -> Result<(), Box<dyn std::error::Error>> {
        let dir = TempDir::new("wav-bad")?;
        let path = dir.path().join("x.wav");
        std::fs::write(&path, b"hello")?;
        assert!(inspect(&path).is_err());
        assert!(repair_header(&path).is_err());
        Ok(())
    }
}
