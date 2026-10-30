use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread::{self, JoinHandle};

const STDERR_LIMIT: usize = 64 * 1024;
static TEMP_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Encoder {
    Auto,
    Nvenc,
    X264,
}

impl Encoder {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "nvenc" | "h264_nvenc" => Ok(Self::Nvenc),
            "x264" | "libx264" => Ok(Self::X264),
            _ => Err(format!(
                "unknown encoder '{value}' (expected auto, nvenc, or x264)"
            )),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Nvenc => "h264_nvenc",
            Self::X264 => "libx264",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Quality {
    Draft,
    Balanced,
    High,
    Archival,
}

impl Quality {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.to_ascii_lowercase().as_str() {
            "draft" => Ok(Self::Draft),
            "balanced" => Ok(Self::Balanced),
            "high" => Ok(Self::High),
            "archival" => Ok(Self::Archival),
            _ => Err(format!(
                "unknown quality preset '{value}' (expected draft, balanced, high, or archival)"
            )),
        }
    }
}

pub fn max_dimension(encoder: Encoder) -> u32 {
    match encoder {
        Encoder::Nvenc | Encoder::Auto => 4096,
        Encoder::X264 => 8192,
    }
}

fn probe_encoder(name: &str) -> Result<(), String> {
    let result = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=256x256:r=1:d=0.1",
            "-frames:v",
            "1",
            "-an",
            "-c:v",
            name,
            "-f",
            "null",
            "-",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("could not run ffmpeg: {e}"))?;
    if result.status.success() {
        Ok(())
    } else {
        let detail = String::from_utf8_lossy(&result.stderr);
        Err(format!(
            "ffmpeg encoder {name} initialization failed: {}",
            detail.trim()
        ))
    }
}

pub fn select_encoder(request: Encoder) -> Result<Encoder, String> {
    select_encoder_with(request, probe_encoder)
}

fn select_encoder_with<F>(request: Encoder, mut probe: F) -> Result<Encoder, String>
where
    F: FnMut(&str) -> Result<(), String>,
{
    match request {
        Encoder::Nvenc => probe("h264_nvenc").map(|_| Encoder::Nvenc),
        Encoder::X264 => probe("libx264").map(|_| Encoder::X264),
        Encoder::Auto => match probe("h264_nvenc") {
            Ok(()) => Ok(Encoder::Nvenc),
            Err(nvenc) => {
                eprintln!(
                    "ffmpeg: NVENC unavailable ({}); falling back to libx264",
                    nvenc.lines().next().unwrap_or(&nvenc)
                );
                probe("libx264").map(|_| Encoder::X264)
            }
        },
    }
}

fn codec_args(encoder: Encoder, quality: Quality) -> Vec<String> {
    let (preset, crf) = match quality {
        Quality::Draft => ("superfast", 30),
        Quality::Balanced => ("medium", 23),
        Quality::High => ("slower", 18),
        Quality::Archival => ("veryslow", 10),
    };
    match encoder {
        Encoder::Nvenc => {
            let p = match quality {
                Quality::Draft => "p1",
                Quality::Balanced => "p4",
                Quality::High => "p6",
                Quality::Archival => "p7",
            };
            let cq = match quality {
                Quality::Draft => 30,
                Quality::Balanced => 23,
                Quality::High => 18,
                Quality::Archival => 10,
            };
            vec![
                "-c:v".into(),
                "h264_nvenc".into(),
                "-preset".into(),
                p.into(),
                "-rc".into(),
                "vbr".into(),
                "-cq".into(),
                cq.to_string(),
                "-b:v".into(),
                "0".into(),
            ]
        }
        _ => vec![
            "-c:v".into(),
            "libx264".into(),
            "-preset".into(),
            preset.into(),
            "-crf".into(),
            crf.to_string(),
        ],
    }
}

fn drain_stderr(mut reader: impl Read + Send + 'static) -> JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut kept = Vec::with_capacity(STDERR_LIMIT);
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if kept.len() < STDERR_LIMIT {
                        let take = n.min(STDERR_LIMIT - kept.len());
                        kept.extend_from_slice(&buf[..take]);
                    }
                }
            }
        }
        kept
    })
}

pub struct VideoWriter {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    stderr: Option<JoinHandle<Vec<u8>>>,
    temp_path: PathBuf,
    output: PathBuf,
    overwrite: bool,
    frame_len: usize,
}

impl VideoWriter {
    pub fn start(
        output: &Path,
        width: u32,
        height: u32,
        fps: u32,
        encoder: Encoder,
        quality: Quality,
        overwrite: bool,
    ) -> Result<Self, String> {
        if width == 0 || height == 0 || fps == 0 {
            return Err("video width, height, and fps must be nonzero".into());
        }
        let frame_len = (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(3))
            .ok_or("frame dimensions overflow addressable memory")?;
        if width % 2 != 0 || height % 2 != 0 {
            return Err("H.264 yuv420p output requires even width and height".into());
        }
        if encoder == Encoder::Auto {
            return Err(
                "resolve Encoder::Auto with select_encoder before starting VideoWriter".into(),
            );
        }
        if !overwrite && output.exists() {
            return Err(format!("output already exists: {}", output.display()));
        }
        let parent = output
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let file_name = output
            .file_name()
            .ok_or_else(|| "output path has no filename".to_string())?
            .to_string_lossy();
        let (temp_path, reservation) = loop {
            let id = TEMP_ID.fetch_add(1, Ordering::Relaxed);
            let p = parent.join(format!(
                ".{file_name}.{}.{}.tmp.mp4",
                std::process::id(),
                id
            ));
            match fs::OpenOptions::new().write(true).create_new(true).open(&p) {
                Ok(f) => break (p, f),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => {
                    return Err(format!(
                        "cannot create temporary output beside {}: {e}",
                        output.display()
                    ));
                }
            }
        };
        drop(reservation);
        let mut cmd = Command::new("ffmpeg");
        cmd.args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "rawvideo",
            "-pixel_format",
            "rgb24",
            "-video_size",
        ])
        .arg(format!("{width}x{height}"))
        .args(["-framerate"])
        .arg(fps.to_string())
        .args(["-i", "pipe:0", "-an"]);
        for arg in codec_args(encoder, quality) {
            cmd.arg(arg);
        }
        cmd.args([
            "-threads",
            "2",
            "-pix_fmt",
            "yuv420p",
            "-movflags",
            "+faststart",
            "-f",
            "mp4",
        ])
        .arg(&temp_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                let _ = fs::remove_file(&temp_path);
                return Err(format!("could not start ffmpeg: {e}"));
            }
        };
        let stdin = match child.stdin.take() {
            Some(s) => s,
            None => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = fs::remove_file(&temp_path);
                return Err("ffmpeg stdin unavailable".into());
            }
        };
        let stderr = child.stderr.take().map(drain_stderr);
        Ok(Self {
            child: Some(child),
            stdin: Some(stdin),
            stderr,
            temp_path,
            output: output.to_path_buf(),
            overwrite,
            frame_len,
        })
    }

    pub fn write_frame(&mut self, rgb: &[u8]) -> Result<(), String> {
        if self.child.is_none() {
            return Err("video writer is finished".into());
        }
        if rgb.len() != self.frame_len {
            return Err(format!(
                "frame has {} bytes; expected {} RGB bytes",
                rgb.len(),
                self.frame_len
            ));
        }
        if let Err(error) = write_frame_to(
            self.stdin.as_mut().ok_or("ffmpeg stdin is closed")?,
            rgb,
            self.frame_len,
        ) {
            self.stdin.take();
            let status = if let Some(mut child) = self.child.take() {
                let _ = child.kill();
                child.wait().ok()
            } else {
                None
            };
            let stderr = self
                .stderr
                .take()
                .and_then(|h| h.join().ok())
                .unwrap_or_default();
            return Err(format!(
                "writing frame to ffmpeg failed: {error}; status: {}; {}",
                status
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "unknown".into()),
                String::from_utf8_lossy(&stderr).trim()
            ));
        }
        Ok(())
    }

    pub fn finish(mut self) -> Result<(), String> {
        self.stdin.take();
        let status = self
            .child
            .take()
            .ok_or("video writer is finished")?
            .wait()
            .map_err(|e| format!("waiting for ffmpeg failed: {e}"))?;
        let stderr = self
            .stderr
            .take()
            .and_then(|h| h.join().ok())
            .unwrap_or_default();
        if !status.success() {
            let _ = fs::remove_file(&self.temp_path);
            return Err(format!(
                "ffmpeg exited with {status}: {}",
                String::from_utf8_lossy(&stderr).trim()
            ));
        }
        let commit = if self.overwrite {
            fs::rename(&self.temp_path, &self.output)
        } else {
            fs::hard_link(&self.temp_path, &self.output)
                .and_then(|_| fs::remove_file(&self.temp_path))
        };
        commit.map_err(|e| {
            let _ = fs::remove_file(&self.temp_path);
            format!("cannot commit video to {}: {e}", self.output.display())
        })
    }
}

fn write_frame_to(writer: &mut impl Write, rgb: &[u8], expected: usize) -> io::Result<()> {
    if rgb.len() != expected {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "frame has {} bytes; expected {expected} RGB bytes",
                rgb.len()
            ),
        ));
    }
    writer.write_all(rgb)
}

impl Drop for VideoWriter {
    fn drop(&mut self) {
        self.stdin.take();
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(handle) = self.stderr.take() {
            let _ = handle.join();
        }
        let _ = fs::remove_file(&self.temp_path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoder_and_quality_parse() {
        assert_eq!(Encoder::parse("AUTO"), Ok(Encoder::Auto));
        assert_eq!(Encoder::parse("nvenc"), Ok(Encoder::Nvenc));
        assert_eq!(Encoder::parse("libx264"), Ok(Encoder::X264));
        assert!(Encoder::parse("nope").is_err());
        assert_eq!(Quality::parse("DRAFT"), Ok(Quality::Draft));
        assert_eq!(Quality::parse("balanced"), Ok(Quality::Balanced));
        assert_eq!(Quality::parse("high"), Ok(Quality::High));
        assert_eq!(Quality::parse("archival"), Ok(Quality::Archival));
        assert!(Quality::parse("nope").is_err());
    }

    #[test]
    fn each_quality_maps_to_expected_codec_options() {
        let expected_x264 = [
            (Quality::Draft, "superfast", "30"),
            (Quality::Balanced, "medium", "23"),
            (Quality::High, "slower", "18"),
            (Quality::Archival, "veryslow", "10"),
        ];
        let expected_nvenc = [
            (Quality::Draft, "p1", "30"),
            (Quality::Balanced, "p4", "23"),
            (Quality::High, "p6", "18"),
            (Quality::Archival, "p7", "10"),
        ];
        for (quality, preset, value) in expected_x264 {
            let args = codec_args(Encoder::X264, quality);
            assert!(args.windows(2).any(|a| a == ["-preset", preset]));
            assert!(args.windows(2).any(|a| a == ["-crf", value]));
        }
        for (quality, preset, value) in expected_nvenc {
            let args = codec_args(Encoder::Nvenc, quality);
            assert!(args.windows(2).any(|a| a == ["-preset", preset]));
            assert!(args.windows(2).any(|a| a == ["-cq", value]));
            assert!(args.windows(2).any(|a| a == ["-rc", "vbr"]));
        }
    }

    #[test]
    fn auto_selection_falls_back_only_after_nvenc_probe_failure() {
        let mut called = Vec::new();
        let encoder = select_encoder_with(Encoder::Auto, |name| {
            called.push(name.to_string());
            if name == "h264_nvenc" {
                Err("no CUDA device".into())
            } else {
                Ok(())
            }
        })
        .unwrap();
        assert_eq!(encoder, Encoder::X264);
        assert_eq!(called, ["h264_nvenc", "libx264"]);

        let mut called = Vec::new();
        assert!(
            select_encoder_with(Encoder::Nvenc, |name| {
                called.push(name.to_string());
                Err("no device".into())
            })
            .is_err()
        );
        assert_eq!(called, ["h264_nvenc"]);

        let mut called = Vec::new();
        assert_eq!(
            select_encoder_with(Encoder::Auto, |name| {
                called.push(name.to_string());
                Ok(())
            }),
            Ok(Encoder::Nvenc)
        );
        assert_eq!(called, ["h264_nvenc"]);
    }

    #[test]
    fn frame_pipe_writer_writes_exact_rgb_frame() {
        let frame = [1u8, 2, 3, 4, 5, 6];
        let mut output = Vec::new();
        write_frame_to(&mut output, &frame, frame.len()).unwrap();
        assert_eq!(output, frame);
        assert!(write_frame_to(&mut output, &frame[..5], frame.len()).is_err());
    }
}
