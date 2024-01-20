use owo_colors::OwoColorize;
use std::process::Command;

const DEPENDENCIES: [(&str, &str, &str); 1] = [
    ("yt-dlp", "yt-dlp", "--version"),
    // ("avifenc", "avifenc", "--version"),
    // ("ffmpeg", "ffmpeg", "-version"),
];

pub fn check_deps() -> Result<(), Box<dyn std::error::Error>> {
    let mut collect: Vec<String> = Vec::with_capacity(DEPENDENCIES.len());
    for (name, cmd, arg) in DEPENDENCIES.iter() {
        if Command::new(cmd).arg(arg).output().is_err() {
            collect.push(name.red().bold().to_string());
        };
    }

    if collect.is_empty() {
        return Ok(());
    }

    let missing_deps = collect.join(", ");
    println!("Missing CLI dependencies: {}", missing_deps);
    std::process::exit(0)
}
