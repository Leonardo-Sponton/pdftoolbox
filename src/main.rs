mod ui;

use adw::prelude::*;
use gtk::{gio, glib};

fn main() -> glib::ExitCode {
    pdftoolbox::storage::cleanup_stale_jobs();
    let app = adw::Application::builder().application_id(ui::APP_ID).flags(gio::ApplicationFlags::HANDLES_OPEN).build();
    app.connect_startup(ui::startup);
    app.connect_activate(|app| ui::activate(app, &[]));
    app.connect_open(|app, files, _| ui::activate(app, files));
    app.run()
}
