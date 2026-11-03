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

use std::ffi::OsStr;
use std::path::PathBuf;

use gpui::{App, AppContext as _, Application, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size};

const USAGE: &str = "\
Pizarra — your trustworthy virtual whiteboard

Usage: pizarra-gpui [FILE.svg]

Shortcuts: Ctrl+N new, Ctrl+O open, Ctrl+S save, Ctrl+Shift+S save as,
Ctrl+Shift+E export to PNG, Ctrl+Z / Ctrl+Shift+Z undo / redo,
L R P C K E Q Alt+Q G tools, Alt+C color, + - 0 zoom, F11 full screen.

Configuration: $PIZARRA_CONFIG or ~/.config/pizarra/config.toml

Graphics backend: $PIZARRA_BACKEND (x11 or wayland; x11 is preferred by
default so that drawing tablets work).";

/// GPUI cannot read tablet (pen) input on native Wayland, and since KWin 6.3
/// and Mutter no longer emulate pens as mouse pointers a stylus cannot draw at
/// all there. XWayland exposes tablets as ordinary XInput devices, which GPUI
/// does understand, so use X11 whenever it is available. Set
/// `PIZARRA_BACKEND=wayland` to keep the native Wayland backend (without pen
/// support) and `PIZARRA_BACKEND=x11` to force the fallback.
fn choose_backend() {
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    {
        let requested = std::env::var_os("PIZARRA_BACKEND");
        let use_wayland = match requested.as_deref() {
            Some(backend) if backend == OsStr::new("wayland") => true,
            Some(backend) if backend == OsStr::new("x11") => false,
            _ => false,
        };

        if use_wayland {
            return;
        }
        // GPUI picks Wayland whenever WAYLAND_DISPLAY is set. Clearing it
        // makes it pick the X11/XWayland backend instead.
        if std::env::var_os("DISPLAY").is_some_and(|display| !display.is_empty()) {
            // SAFETY: called before any other thread is spawned.
            unsafe { std::env::remove_var("WAYLAND_DISPLAY") };
        }
    }
}

fn main() {
    choose_backend();
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
