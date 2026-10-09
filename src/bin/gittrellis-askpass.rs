//! Askpass helper binary.
//!
//! Installed next to the main `gittrellis` binary and pointed at by the
//! `GIT_ASKPASS` / `SSH_ASKPASS` environment variables, so passphrase and
//! credential prompts appear as native dialogs when there is no terminal.

fn main() -> gtk::glib::ExitCode {
    gittrellis::ui::askpass::run()
}
