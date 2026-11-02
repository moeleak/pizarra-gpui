//! Pizarra: a free-hand vector drawing application with an infinite canvas.
//!
//! This is a GPUI frontend for the [`pizarra`] engine, ported from the GTK
//! frontend (pizarra-gtk).

mod app;
mod assets;
mod canvas;
mod config;
mod export;
mod files;
mod geometry;
mod prompt;
mod theme;

use std::path::PathBuf;

use gpui::{App, AppContext as _, Application, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size};

const USAGE: &str = "\
Pizarra — tu pizarrón virtual de confianza

Uso: pizarra-gpui [ARCHIVO.svg]

Atajos: Ctrl+N nuevo, Ctrl+O abrir, Ctrl+S guardar, Ctrl+Shift+S guardar como,
Ctrl+Shift+E exportar a PNG, Ctrl+Z / Ctrl+Shift+Z deshacer / rehacer,
L R P C K E Q Alt+Q G herramientas, Alt+C color, + - 0 zoom, F11 pantalla completa.

Configuración: $PIZARRA_CONFIG o ~/.config/pizarra/config.toml";

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let mut file = None;
    for arg in std::env::args_os().skip(1) {
        match arg.to_str() {
            Some("-h" | "--help") => {
                println!("{USAGE}");
                return;
            }
            Some("-V" | "--version") => {
                println!("pizarra-gpui {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            _ if file.is_none() => file = Some(PathBuf::from(arg)),
            _ => {
                eprintln!("{USAGE}");
                std::process::exit(2);
            }
        }
    }

    Application::new().with_assets(assets::Assets).run(move |cx: &mut App| {
        app::bind_keys(cx);
        cx.set_prompt_builder(prompt::build);
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(800.), px(600.)), cx))),
            titlebar: Some(TitlebarOptions { title: Some("Pizarra".into()), ..Default::default() }),
            app_id: Some("tk.categulario.pizarra-gpui".into()),
            window_min_size: Some(size(px(560.), px(320.))),
            focus: true,
            ..Default::default()
        };

        cx.open_window(options, |window, cx| cx.new(|cx| app::PizarraWindow::new(file, window, cx)))
            .expect("could not open the main window");
        cx.activate(true);
    });
}
