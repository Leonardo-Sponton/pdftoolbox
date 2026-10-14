// As macros `tr!` e `trn!` da biblioteca valem em toda a interface.
#[macro_use]
extern crate pdftoolbox;

mod ui;

use std::sync::atomic::{AtomicBool, Ordering};

use adw::prelude::*;
use gtk::{gio, glib};

fn main() -> glib::ExitCode {
    // Antes de qualquer outra coisa: pode ajustar o ambiente para o GTK.
    ui::init_language();
    pdftoolbox::storage::cleanup_stale_jobs();
    let app = adw::Application::builder().application_id(ui::APP_ID).flags(gio::ApplicationFlags::HANDLES_OPEN).build();
    app.connect_startup(ui::startup);
    app.connect_activate(|app| ui::activate(app, &[]));
    app.connect_open(|app, files, _| ui::activate(app, files));
    // Ao sair (inclusive por SIGTERM/SIGINT), nenhum motor externo fica rodando
    // e as pastas temporárias das tarefas são apagadas.
    app.connect_shutdown(|_| {
        pdftoolbox::engines::kill_all_running();
        pdftoolbox::storage::cleanup_own_jobs();
    });
    watch_termination(&app);
    app.run()
}

static TERMINATE: AtomicBool = AtomicBool::new(false);

extern "C" fn on_terminate(_: libc::c_int) {
    // Só operações seguras em tratador de sinal: uma escrita atômica.
    TERMINATE.store(true, Ordering::SeqCst);
}

/// SIGTERM/SIGINT encerram o aplicativo pelo caminho normal (com `shutdown`).
fn watch_termination(app: &adw::Application) {
    // SAFETY: o tratador só grava um AtomicBool.
    unsafe {
        libc::signal(libc::SIGTERM, on_terminate as *const () as libc::sighandler_t);
        libc::signal(libc::SIGINT, on_terminate as *const () as libc::sighandler_t);
    }
    let app = app.downgrade();
    glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
        if TERMINATE.load(Ordering::SeqCst) {
            if let Some(app) = app.upgrade() {
                app.quit();
            }
            return glib::ControlFlow::Break;
        }
        glib::ControlFlow::Continue
    });
}
