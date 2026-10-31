use std::path::Path;
use timelapse4sai2::{connection, export, image, recorder};

fn run() -> Result<(), String> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args.iter().any(|a| a == "--help" || a == "-h") {
        println!(
            "timelapse4sai2 [--pid PID] detect|canvases|dump INDEX FILE [--channel-order bgra|rgba]"
        );
        println!("timelapse4sai2 [--pid PID] record INDEX SESSION_DIR [OPTIONS]");
        println!("timelapse4sai2 info SESSION_DIR");
        println!("timelapse4sai2 verify SESSION_DIR");
        println!("timelapse4sai2 verify SESSION_DIR --dump-final FILE.png");
        println!("{}", export::HELP);
        let d = recorder::Config::default();
        println!(
            "  --idle-delay-ms {}\n  --min-capture-gap-ms {}\n  --max-active-gap-ms {}\n  --zstd-level {}\n  --checkpoint-every-events {}\n  --checkpoint-changed-ratio {}\n  --no-input-watch",
            d.idle_delay_ms,
            d.min_capture_gap_ms,
            d.max_active_gap_ms,
            d.zstd_level,
            d.checkpoint_every_events,
            d.checkpoint_changed_ratio
        );
        println!(
            "Target: SAI2 x64 Preview and Major Renovated builds. Unknown Renovated builds require an open canvas for layout validation. Ctrl-C drains recording storage."
        );
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("info") {
        if args.len() != 2 {
            return Err("usage: timelapse4sai2 info SESSION_DIR".into());
        }
        let session = recorder::storage::Session::open(Path::new(&args[1]))?;
        let summary = session.summary()?;
        for warning in &session.warnings {
            eprintln!("Warning: {warning}");
        }
        let seconds = summary.tracked_duration_ms / 1000;
        println!("Canvas: {}", summary.canvas_name);
        println!(
            "Tracked: {}h {}m {}s",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60
        );
        println!(
            "Captures: {}\n  Stable: {}\n  Provisional: {}",
            summary.captures, summary.stable_captures, summary.provisional_captures
        );
        println!("Size: {:.2} GiB", summary.disk_bytes as f64 / 1073741824.0);
        println!(
            "Dimensions: {}x{}",
            summary.current_width, summary.current_height
        );
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("verify") {
        if args.len() == 2 {
            let report = recorder::storage::verify(Path::new(&args[1]))?;
            for warning in &report.warnings {
                eprintln!("Warning: {warning}");
            }
            let summary = report.summary;
            println!(
                "Verified: {} captures ({} stable, {} provisional), {} unique blobs",
                summary.captures,
                summary.stable_captures,
                summary.provisional_captures,
                summary.unique_blobs
            );
            return Ok(());
        }
        if args.len() == 4 && args[2] == "--dump-final" {
            let report = recorder::storage::verify(Path::new(&args[1]))?;
            for warning in &report.warnings {
                eprintln!("Warning: {warning}");
            }
            return export::dump_final(Path::new(&args[1]), Path::new(&args[3]));
        }
        return Err("usage: timelapse4sai2 verify SESSION_DIR [--dump-final FILE.png]".into());
    }
    if args.first().map(String::as_str) == Some("export") {
        if args.len() < 3 {
            return Err(export::HELP.into());
        }
        let options = export::Options::parse(&args[3..])?;
        let report = export::run(Path::new(&args[1]), Path::new(&args[2]), options)?;
        for warning in &report.warnings {
            eprintln!("Warning: {warning}");
        }
        println!(
            "Exported {} video frames to {}",
            report.plan.output_video_frames, args[2]
        );
        return Ok(());
    }
    let mut pid = None;
    if args.first().is_some_and(|a| a == "--pid") {
        if args.len() < 3 {
            return Err("--pid requires PID and command".into());
        }
        pid = Some(
            args[1]
                .parse::<u32>()
                .ok()
                .filter(|&p| p > 0)
                .ok_or("invalid PID")?,
        );
        args.drain(..2);
    }
    let mut rgba = false;
    if let Some(i) = args.iter().position(|a| a == "--channel-order") {
        if args.first().map(String::as_str) != Some("dump") || i + 2 != args.len() {
            return Err("--channel-order bgra|rgba must follow dump output path".into());
        }
        rgba = match args[i + 1].as_str() {
            "bgra" => false,
            "rgba" => true,
            _ => return Err("expected bgra or rgba".into()),
        };
        args.truncate(i);
    }
    match args.first().map(String::as_str) {
        Some("detect" | "canvases") if args.len() == 1 => (),
        Some("dump") if args.len() == 3 => (),
        Some("record") if args.len() >= 3 => { recorder::Config::parse(&args[3..])?; },
        _ => return Err("usage: timelapse4sai2 [--pid PID] detect|canvases|dump INDEX FILE|record INDEX SESSION_DIR [OPTIONS] [--channel-order bgra|rgba]".into()),
    }
    let connection = connection::Connection::discover(pid)?;
    let reader = connection.reader();
    let process = &connection.process;
    let profile = connection.profile;
    let hash = &connection.executable_md5;
    let status = connection.compatibility;
    let resolved_root_rva = connection.resolved_root_rva;
    if args[0] == "detect" {
        println!("Found sai2.exe (PID: {})", process.pid);
        println!("Module base: {:#x}", process.base_address);
        println!(
            "Profile: {} ({})",
            profile.display_name,
            profile.generation.name()
        );
        println!("Executable MD5: {hash}");
        println!("Compatibility: {status:?}");
        if let Some(rva) = resolved_root_rva {
            println!("Session RVA: {rva:#x}");
        }
        return Ok(());
    }
    if status == timelapse4sai2::sai::profile::CompatibilityStatus::NeedsCanvas {
        return Err("NeedsCanvas: open a canvas in SAI2 to validate this Renovated build".into());
    }
    let canvases = connection.canvases();
    if args[0] == "canvases" {
        for (i, c) in canvases.iter().enumerate() {
            println!(
                "{i}: {}\n   {}x{}\n   address: {:#x}",
                c.name, c.width, c.height, c.address
            );
        }
    } else {
        let index = args[1]
            .parse::<usize>()
            .map_err(|_| "invalid canvas index")?;
        let c = canvases
            .get(index)
            .ok_or("canvas index out of range; run canvases again")?;
        if args[0] == "record" {
            eprintln!(
                "Recording {} to {}; Ctrl-C stops and drains storage.",
                c.name, args[2]
            );
            let metrics = recorder::record(
                &reader,
                c,
                Path::new(&args[2]),
                recorder::Config::parse(&args[3..])?,
            )?;
            for warning in &metrics.warnings {
                eprintln!("Warning: {warning}");
            }
            eprintln!(
                "Captures: {} ({} provisional)",
                metrics.logical_frames, metrics.provisional_frames
            );
            return Ok(());
        }
        let rgb = reader.composite(c, rgba)?;
        image::write_png(Path::new(&args[2]), c.width, c.height, &rgb)?;
        println!("Wrote {}", args[2]);
    }
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("timelapse4sai2: {e}");
        std::process::exit(1);
    }
}
