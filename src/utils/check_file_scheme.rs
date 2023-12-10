use lazy_static::lazy_static;
use owo_colors::OwoColorize;
use regex::Regex;

lazy_static! {
    static ref PLACEHOLDER_RE: Regex = Regex::new(r"\{[^{]+\}").unwrap();
}

const VALID_PLACEHOLDERS: [&str; 4] = ["{UPVOTES}", "{AUTHOR}", "{POSTID}", "{DATE}"];

pub fn check_file_scheme(placeholder: &str) {
    let res = PLACEHOLDER_RE
        .captures_iter(placeholder)
        .filter_map(|c| c.get(0))
        .map(|c| c.as_str())
        .filter(|&c| !VALID_PLACEHOLDERS.contains(&c))
        .collect::<Vec<_>>();

    match res.len() {
        0 => (),
        _ => {
            println!(
                "{} {}",
                "[INVALID_FILE_SCHEME]".bold().red(),
                res.join(" ").bold()
            );
            println!(
                "Valid placeholders: {}",
                VALID_PLACEHOLDERS.join(" ").bold()
            );
            std::process::exit(0)
        }
    }
}
