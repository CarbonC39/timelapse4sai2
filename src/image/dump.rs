use std::fs::{self, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::Path;

/// Writes an 8-bit RGB image to a new PNG file.
///
/// The destination must not already exist. If encoding fails after the file is
/// created, the incomplete file is removed.
pub fn write_png(path: &Path, width: u32, height: u32, rgb: &[u8]) -> Result<(), String> {
    let expected_len = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(3))
        .ok_or_else(|| "image dimensions are too large".to_owned())?;
    if rgb.len() != expected_len {
        return Err(format!(
            "RGB data length mismatch: expected {expected_len} bytes, got {}",
            rgb.len()
        ));
    }

    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|err| format!("failed to create {}: {err}", path.display()))?;

    let result = (|| {
        let mut output = BufWriter::new(file);
        let mut encoder = png::Encoder::new(&mut output, width, height);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|err| format!("failed to write PNG header: {err}"))?;
        writer
            .write_image_data(rgb)
            .map_err(|err| format!("failed to write PNG image data: {err}"))?;
        writer
            .finish()
            .map_err(|err| format!("failed to finish PNG: {err}"))?;
        output.flush().map_err(|err| format!("failed to flush PNG: {err}"))?;
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(path);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::write_png;
    use std::fs::{self, File};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

    fn unique_path() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "timelapse4sai2-png-{}-{nanos}-{}.png",
            std::process::id(),
            NEXT_FILE.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn writes_and_roundtrips_rgb_png_without_overwriting() {
        let path = unique_path();
        let pixels = [
            255, 0, 17, // red-ish
            4, 128, 250, // blue-ish
            31, 62, 93, // distinct third pixel
        ];

        write_png(&path, 3, 1, &pixels).unwrap();
        let decoder = png::Decoder::new(File::open(&path).unwrap());
        let mut reader = decoder.read_info().unwrap();
        assert_eq!(reader.info().width, 3);
        assert_eq!(reader.info().height, 1);
        assert_eq!(reader.info().color_type, png::ColorType::Rgb);
        assert_eq!(reader.info().bit_depth, png::BitDepth::Eight);
        let mut buffer = vec![0; reader.output_buffer_size()];
        let frame = reader.next_frame(&mut buffer).unwrap();
        buffer.truncate(frame.buffer_size());
        assert_eq!(buffer, pixels);

        assert!(write_png(&path, 3, 1, &[0; 9]).is_err());
        fs::remove_file(path).unwrap();
    }
}
