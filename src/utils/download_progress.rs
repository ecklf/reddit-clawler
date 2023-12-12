use indicatif::{ProgressBar, ProgressState, ProgressStyle};
use std::cmp::min;

pub struct DownloadProgress {
    pub control: ProgressBar,
    pub total_count: u64,
}

impl DownloadProgress {
    pub fn new(len: u64) -> Self {
        let stats = ProgressBar::new(len);
        stats.set_style(
            ProgressStyle::with_template(
                "[{spinner:.202}] — [{elapsed_precise}] — [{wide_bar:.202}] — {msg} ({eta})",
            )
            .unwrap()
            .with_key(
                "eta",
                |state: &ProgressState, w: &mut dyn std::fmt::Write| {
                    write!(w, "{:.1}s", state.eta().as_secs_f64()).unwrap()
                },
            )
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"])
            .progress_chars("█▉▊▋▌▍▎▏  "),
        );

        DownloadProgress {
            control: stats,
            total_count: len,
        }
    }

    pub fn bytes_to_mb(&self, bytes: f64) -> String {
        let mb = bytes / 1024.0 / 1024.0;
        format!("{:.2} MB", mb)
    }

    pub fn update_progress(&self, current_count: u64, total_count: u64, bytes_downloaded: f64) {
        let progress_pos = min(current_count, total_count);

        let msg = format!(
            "{}/{} - {}",
            progress_pos,
            total_count,
            self.bytes_to_mb(bytes_downloaded)
        );

        self.control.set_position(progress_pos);
        self.control.set_message(msg);
    }

    pub fn post_report(&self, current_count: u64, total_count: u64, bytes_downloaded: f64) {
        let msg = format!(
            "Downloaded {}/{} - {}",
            current_count,
            total_count,
            self.bytes_to_mb(bytes_downloaded)
        );

        self.control.finish_with_message(msg);
    }
}
