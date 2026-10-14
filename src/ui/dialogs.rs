//! Diálogos de dependências e “Sobre”.

use adw::prelude::*;
use pdftoolbox::engines::Tool;

use super::{APP_ID, widgets};

/// Diagnóstico das ferramentas externas: presença, versão, uso e instalação.
pub fn dependencies(parent: &impl IsA<gtk::Widget>) {
    let page = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::builder()
        .title(tr!("Ferramentas externas"))
        .description(tr!(
            "Componentes nativos usados pelo PDFToolbox. O programa nunca instala nada nem executa comandos \
             administrativos: copie o comando e rode-o você mesmo em um terminal."
        ))
        .build();
    for tool in Tool::ALL {
        let (subtitle, icon) = match tool.version() {
            Some(v) => {
                (tr!("Versão {version} · {purpose}", version = v, purpose = tool.purpose()), "emblem-ok-symbolic")
            }
            None if tool.find().is_some() => {
                (tr!("Instalado · {purpose}", purpose = tool.purpose()), "emblem-ok-symbolic")
            }
            None => {
                let text = tr!(
                    "Não instalado · {purpose}\n{command}",
                    purpose = tool.purpose(),
                    command = tool.install_command()
                );
                (text, "dialog-warning-symbolic")
            }
        };
        let row = widgets::plain_row(tool.label(), &subtitle);
        row.set_subtitle_selectable(true);
        let image = gtk::Image::from_icon_name(icon);
        image.add_css_class(if icon == "emblem-ok-symbolic" { "success" } else { "warning" });
        row.add_prefix(&image);
        if tool.find().is_none() {
            let copy = widgets::icon_button("edit-copy-symbolic", tr!("Copiar comando de instalação"));
            let cmd = tool.install_command();
            copy.connect_clicked(move |b| {
                b.clipboard().set_text(&cmd);
                b.set_icon_name("emblem-ok-symbolic");
            });
            row.add_suffix(&copy);
        }
        group.add(&row);
    }
    page.add(&group);

    let privacy = adw::PreferencesGroup::builder()
        .title(tr!("Privacidade"))
        .description(tr!(
            "Todo o processamento acontece neste computador. O PDFToolbox não faz conexões de rede, não tem \
             telemetria e não verifica atualizações. Arquivos temporários ficam em ~/.cache/pdftoolbox e são \
             apagados ao fim de cada tarefa."
        ))
        .build();
    page.add(&privacy);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(&page));
    let dialog = adw::Dialog::builder()
        .title(tr!("Dependências"))
        .content_width(560)
        .content_height(560)
        .child(&toolbar)
        .build();
    dialog.present(Some(parent));
}

pub fn about(parent: &impl IsA<gtk::Widget>) {
    let engines: Vec<String> = Tool::ALL
        .iter()
        .map(|t| format!("{} {}", t.label(), t.version().unwrap_or_else(|| "(ausente)".into())))
        .collect();
    let about = adw::AboutDialog::builder()
        .application_name("PDFToolbox")
        .application_icon(APP_ID)
        .version(env!("CARGO_PKG_VERSION"))
        .developer_name("Leonardo")
        .license_type(gtk::License::Gpl30)
        .comments(tr!("Suíte local de documentos PDF para Linux. Seus arquivos não saem do seu computador."))
        .debug_info(format!(
            "PDFToolbox {}\nGTK {}.{}.{}\nlibadwaita {}.{}.{}\n{}",
            env!("CARGO_PKG_VERSION"),
            gtk::major_version(),
            gtk::minor_version(),
            gtk::micro_version(),
            adw::major_version(),
            adw::minor_version(),
            adw::micro_version(),
            engines.join("\n")
        ))
        .build();
    about.add_credit_section(
        Some(tr!("Motores de processamento")),
        &[
            "qpdf https://qpdf.sourceforge.io",
            "Poppler https://poppler.freedesktop.org",
            "Ghostscript https://ghostscript.com",
        ],
    );
    about.present(Some(parent));
}
