mod export;
mod image;
mod process;
mod recorder;
mod sai;
use process::LinuxProcess;
use std::{fs::File, io::Read, path::Path};

fn run() -> Result<(), String> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args.iter().any(|a| a == "--help" || a == "-h") {
        println!(
            "timelapse4sai2 [--pid PID] detect|canvases|dump INDEX FILE [--channel-order bgra|rgba]"
        );
        println!("timelapse4sai2 [--pid PID] record INDEX SESSION_DIR [OPTIONS]");
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
            "Target: SAI2 x64 2026-07-02b Technical Preview. Ctrl-C drains recording storage."
        );
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("verify") {
        if args.len() == 2 {
            return recorder::storage::verify(Path::new(&args[1]));
        }
        if args.len() == 4 && args[2] == "--dump-final" {
            recorder::storage::verify(Path::new(&args[1]))?;
            return export::dump_final(Path::new(&args[1]), Path::new(&args[3]));
        }
        return Err("usage: timelapse4sai2 verify SESSION_DIR [--dump-final FILE.png]".into());
    }
    if args.first().map(String::as_str) == Some("export") {
        if args.len() < 3 {
            return Err(export::HELP.into());
        }
        let options = export::Options::parse(&args[3..])?;
        return export::run(Path::new(&args[1]), Path::new(&args[2]), options);
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
                .filter(|&p| p > 0 && p <= i32::MAX as u32)
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
    let process = LinuxProcess::discover(pid)?;
    // Hash the mapped executable through the target mount namespace (Bottles/Flatpak).
    let rooted = Path::new(&format!("/proc/{}/root", process.pid)).join(
        process
            .module_path
            .strip_prefix("/")
            .map_err(|e| e.to_string())?,
    );
    let mut file = File::open(&rooted)
        .map_err(|e| format!("cannot open mapped executable {}: {e}", rooted.display()))?;
    let mut digest = md5::Context::new();
    let mut block = [0u8; 65536];
    loop {
        let n = file.read(&mut block).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        digest.consume(&block[..n]);
    }
    let hash = format!("{:x}", digest.compute());
    if hash != sai::preview_2026_07_02b::MD5 {
        return Err(format!(
            "unsupported sai2.exe MD5 {hash}; expected {}",
            sai::preview_2026_07_02b::MD5
        ));
    }
    if args[0] == "detect" {
        println!("Found sai2.exe (PID: {})", process.pid);
        println!("Module base: {:#x}", process.base_address);
        return Ok(());
    }
    let reader = sai::reader::Reader {
        memory: &process,
        base: process.base_address,
        profile: &sai::preview_2026_07_02b::LAYOUT,
    };
    let canvases = reader.canvases()?;
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
            return recorder::record(
                &reader,
                c,
                Path::new(&args[2]),
                &hash,
                recorder::Config::parse(&args[3..])?,
            );
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
