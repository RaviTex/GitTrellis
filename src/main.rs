//! Stratum — a native Git GUI client for Linux.

fn main() -> gtk::glib::ExitCode {
    stratum::ui::app::run()
}
