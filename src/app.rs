//! The main window: a header bar with the drawing controls, the canvas, and
//! the dialogs around them. Behaviour follows the GTK frontend (pizarra-gtk
//! 1.7.5): same shortcuts, same save/unsaved-changes logic, same messages.

use std::cell::Cell;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use anyhow::Result;
use gpui::{
    AnyView, App, AsyncWindowContext, Bounds, ContentMask, Context, Corner, CursorStyle, DispatchPhase, ElementId,
    FocusHandle, FontWeight, HitboxBehavior, KeyBinding, Modifiers, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, PathPromptOptions, Pixels, Point, PromptLevel, Render, ScrollDelta,
    ScrollWheelEvent, SharedString, WeakEntity, Window, actions, anchored, canvas, deferred, div, fill, img,
    point, prelude::*, px, svg,
};
use pizarra::{
    Pizarra, SaveStatus, SelectedTool, ShapeTool, ShouldRedraw, Vec2D, color::Color,
    deserialize::DeserializeResult, key::Key, point::Unit, ui::Flags,
};

use crate::canvas::{self, ShapeLayer};
use crate::export::ExportJob;
use crate::{config, files, theme};

const TOOLBAR_HEIGHT: f32 = 46.0;
const KEY_CONTEXT: &str = "Pizarra";

/// GPUI reports three "lines" per wheel notch on Linux, GTK reports one.
const LINES_PER_NOTCH: f64 = 3.0;

const THICKNESS_RANGE: (f64, f64, f64) = (2.0, 50.0, 2.0); // min, max, step
const ALPHA_STEP: f64 = 0.1;

// Messages of the GTK frontend, split into GPUI's message + detail.
const UNSAVED_SINCE_LAST_TIME: (&str, &str) = ("Hiciste algunos trazos desde la última vez", "¿Los quieres guardar?");
const UNSAVED_NEW_FILE: (&str, &str) = ("Hay algunos trazos aquí", "¿Los quieres guardar?");
const CLOSE_NEW_FILE: (&str, &str) = ("Ya hiciste algunos dibujos", "¿Deseas guardar?");
const CLOSE_SINCE_LAST_TIME: (&str, &str) = ("Hay cambios desde la última vez que guardaste", "¿Quieres guardarlos?");

actions!(
    pizarra,
    [
        NewDrawing,
        Open,
        Save,
        SaveAs,
        Export,
        Quit,
        Undo,
        Redo,
        ToggleFullscreen,
        About,
        ZoomIn,
        ZoomOut,
        ZoomHome,
        ToolPen,
        ToolRectangle,
        ToolPolygon,
        ToolCircle,
        ToolCircleThreePoints,
        ToolEllipse,
        ToolGrid,
        ToolFreeGrid,
        ToolEraser,
        ChooseColor,
        Cancel,
    ]
);

pub fn bind_keys(cx: &mut App) {
    let ctx = Some(KEY_CONTEXT);
    cx.bind_keys([
        KeyBinding::new("ctrl-n", NewDrawing, ctx),
        KeyBinding::new("ctrl-o", Open, ctx),
        KeyBinding::new("ctrl-s", Save, ctx),
        KeyBinding::new("ctrl-shift-s", SaveAs, ctx),
        KeyBinding::new("ctrl-shift-e", Export, ctx),
        KeyBinding::new("ctrl-q", Quit, ctx),
        KeyBinding::new("ctrl-z", Undo, ctx),
        KeyBinding::new("ctrl-shift-z", Redo, ctx),
        KeyBinding::new("ctrl-y", Redo, ctx),
        KeyBinding::new("f11", ToggleFullscreen, ctx),
        KeyBinding::new("l", ToolPen, ctx),
        KeyBinding::new("r", ToolRectangle, ctx),
        KeyBinding::new("p", ToolPolygon, ctx),
        KeyBinding::new("c", ToolCircle, ctx),
        KeyBinding::new("k", ToolCircleThreePoints, ctx),
        KeyBinding::new("e", ToolEllipse, ctx),
        KeyBinding::new("q", ToolGrid, ctx),
        KeyBinding::new("alt-q", ToolFreeGrid, ctx),
        KeyBinding::new("g", ToolEraser, ctx),
        KeyBinding::new("alt-c", ChooseColor, ctx),
        KeyBinding::new("+", ZoomIn, ctx),
        KeyBinding::new("shift-+", ZoomIn, ctx),
        KeyBinding::new("=", ZoomIn, ctx),
        KeyBinding::new("shift-=", ZoomIn, ctx),
        KeyBinding::new("-", ZoomOut, ctx),
        KeyBinding::new("0", ZoomHome, ctx),
        KeyBinding::new("escape", Cancel, ctx),
    ]);
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tool {
    Pen,
    Rectangle,
    Polygon,
    Circle,
    CircleThreePoints,
    Ellipse,
    Grid,
    FreeGrid,
    Eraser,
}

impl Tool {
    const ALL: [Tool; 9] = [
        Tool::Pen,
        Tool::Rectangle,
        Tool::Polygon,
        Tool::Circle,
        Tool::CircleThreePoints,
        Tool::Ellipse,
        Tool::Grid,
        Tool::FreeGrid,
        Tool::Eraser,
    ];

    fn label(self) -> &'static str {
        match self {
            Tool::Pen => "Lápiz",
            Tool::Rectangle => "Rectángulo",
            Tool::Polygon => "Polígono",
            Tool::Circle => "Círculo por el centro y un punto",
            Tool::CircleThreePoints => "Círculo por tres puntos",
            Tool::Ellipse => "Elipse por los focos y un punto",
            Tool::Grid => "Cuadrícula",
            Tool::FreeGrid => "Cuadrícula libre",
            Tool::Eraser => "Goma",
        }
    }

    fn shortcut(self) -> &'static str {
        match self {
            Tool::Pen => "L",
            Tool::Rectangle => "R",
            Tool::Polygon => "P",
            Tool::Circle => "C",
            Tool::CircleThreePoints => "K",
            Tool::Ellipse => "E",
            Tool::Grid => "Q",
            Tool::FreeGrid => "Alt+Q",
            Tool::Eraser => "G",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Tool::Pen => "icons/line.svg",
            Tool::Rectangle => "icons/rectangle.svg",
            Tool::Polygon => "icons/polygon.svg",
            Tool::Circle => "icons/circle_by_center_and_point.svg",
            Tool::CircleThreePoints => "icons/circle_by_three_points.svg",
            Tool::Ellipse => "icons/ellipse_by_foci_and_point.svg",
            Tool::Grid => "icons/grid.svg",
            Tool::FreeGrid => "icons/free_grid.svg",
            Tool::Eraser => "icons/eraser.svg",
        }
    }

    fn selected(self) -> SelectedTool {
        match self {
            Tool::Pen => SelectedTool::Shape(ShapeTool::Path),
            Tool::Rectangle => SelectedTool::Shape(ShapeTool::Rectangle),
            Tool::Polygon => SelectedTool::Shape(ShapeTool::Polygon),
            Tool::Circle => SelectedTool::Shape(ShapeTool::CircleByCenterAndPoint),
            Tool::CircleThreePoints => SelectedTool::Shape(ShapeTool::CircleThroughThreePoints),
            Tool::Ellipse => SelectedTool::Shape(ShapeTool::ThreePointEllipse),
            Tool::Grid => SelectedTool::Shape(ShapeTool::Grid),
            Tool::FreeGrid => SelectedTool::Shape(ShapeTool::FreeGrid),
            Tool::Eraser => SelectedTool::Eraser,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Popover {
    Color,
    Tools,
    Thickness,
    Alpha,
    Menu,
}

impl Popover {
    const COUNT: usize = 5;

    fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Slider {
    Thickness,
    Alpha,
    Red,
    Green,
    Blue,
}

/// Why the current drawing is about to be discarded; picks the wording.
#[derive(Clone, Copy)]
enum Discard {
    Replace,
    Close,
}

enum Loaded {
    Ok,
    Partial(Vec<String>),
    Failed(&'static str, String),
}

pub struct PizarraWindow {
    controller: Pizarra,
    focus_handle: FocusHandle,

    shapes: ShapeLayer,
    canvas_bounds: Option<Bounds<Pixels>>,
    /// Buttons pressed on the canvas and not released yet. While non-empty the
    /// canvas keeps receiving moves and releases outside its bounds, like a GTK
    /// implicit grab.
    held_buttons: Vec<MouseButton>,

    // What the header bar shows; re-applied to the controller when it is
    // replaced (new drawing, open).
    tool: Tool,
    color: Color,
    alpha: f64,
    thickness: f64,

    popover: Option<Popover>,
    anchors: Rc<[Cell<Option<Bounds<Pixels>>>; Popover::COUNT]>,
    dragging: Option<Slider>,
    show_about: bool,

    /// A dialog flow (save, open, ...) is running.
    busy: bool,
    /// The unsaved-changes question was answered; let the window close.
    allow_close: bool,
    /// The open file was only partially understood: saving must not overwrite
    /// it without asking.
    save_needs_dialog: bool,
    last_dir: Option<PathBuf>,
    window_title: String,
}

impl PizarraWindow {
    pub fn new(file: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let config = config::read();
        let controller = Pizarra::new(Vec2D::new_screen(1.0, 1.0), config);
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle);

        let this = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            this.update(cx, |this, cx| this.should_close(window, cx)).unwrap_or(true)
        });

        if let Some(file) = file {
            cx.defer_in(window, move |this, window, cx| {
                this.run_flow(window, cx, async move |this, cx| Self::open_path(&this, cx, file).await);
            });
        }

        let thickness = config.thickness.val().clamp(THICKNESS_RANGE.0, THICKNESS_RANGE.1);
        let alpha = f64::from(config.stroke_color.a) / 255.0;
        let mut this = Self {
            controller,
            focus_handle,
            shapes: ShapeLayer::default(),
            canvas_bounds: None,
            held_buttons: Vec::new(),
            tool: Tool::Pen,
            color: config.stroke_color.with_alpha(255),
            alpha,
            thickness,
            popover: None,
            anchors: Rc::new(Default::default()),
            dragging: None,
            show_about: false,
            busy: false,
            allow_close: false,
            save_needs_dialog: false,
            last_dir: None,
            window_title: String::new(),
        };
        this.apply_ui_state();
        this
    }

    // ---------------------------------------------------------------------
    // State helpers

    /// Pushes the header bar settings into the controller.
    fn apply_ui_state(&mut self) {
        self.controller.set_tool(self.tool.selected());
        self.controller.set_color(self.color.with_alpha(alpha_u8(self.alpha)));
        self.controller.set_stroke(self.thickness.into());
    }

    fn redraw(&mut self, hint: ShouldRedraw, cx: &mut Context<Self>) {
        if hint == ShouldRedraw::All {
            self.shapes.invalidate();
        }
        cx.notify();
    }

    fn invalidate(&mut self, cx: &mut Context<Self>) {
        self.redraw(ShouldRedraw::All, cx);
    }

    fn has_unsaved_changes(&self) -> bool {
        matches!(self.controller.get_save_status(), SaveStatus::NewAndChanged | SaveStatus::Unsaved(_))
    }

    /// The file plain "Save" may write to without asking.
    fn current_path(&self) -> Option<PathBuf> {
        match self.controller.get_save_status() {
            SaveStatus::Saved(path) | SaveStatus::Unsaved(path) if !self.save_needs_dialog => Some(path.clone()),
            _ => None,
        }
    }

    fn saved_path(&self) -> Option<&Path> {
        match self.controller.get_save_status() {
            SaveStatus::Saved(path) | SaveStatus::Unsaved(path) => Some(path),
            _ => None,
        }
    }

    fn dialog_dir(&self) -> PathBuf {
        self.last_dir
            .clone()
            .or_else(|| self.saved_path().and_then(Path::parent).map(Path::to_path_buf))
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."))
    }

    fn suggested_name(&self, extension: &str) -> String {
        let stem = self
            .saved_path()
            .and_then(Path::file_stem)
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "dibujo".into());
        if self.save_needs_dialog && extension == "svg" {
            format!("{stem}-pizarra.svg")
        } else {
            format!("{stem}.{extension}")
        }
    }

    fn titles(&self) -> (String, Option<String>) {
        let name = |p: &Path| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let dir = |p: &Path| p.parent().map(|d| d.display().to_string());
        match self.controller.get_save_status() {
            SaveStatus::NewAndEmpty => ("Pizarra".into(), None),
            SaveStatus::NewAndChanged => ("*Dibujo sin guardar".into(), None),
            SaveStatus::Unsaved(path) => (format!("*{}", name(path)), dir(path)),
            SaveStatus::Saved(path) => (name(path), dir(path)),
        }
    }

    fn sync_window_title(&mut self, window: &mut Window) {
        let (title, _) = self.titles();
        let full = if title == "Pizarra" { title } else { format!("{title} — Pizarra") };
        if full != self.window_title {
            window.set_window_title(&full);
            self.window_title = full;
        }
    }

    fn replace_controller(&mut self, mut controller: Pizarra, cx: &mut Context<Self>) {
        controller.resize(self.controller.get_dimensions());
        self.controller = controller;
        self.held_buttons.clear();
        self.apply_ui_state();
        self.invalidate(cx);
    }

    fn write_svg(&mut self, path: &Path, cx: &mut Context<Self>) -> Result<()> {
        files::write_atomically(path, self.controller.to_svg().as_bytes())?;
        self.controller.set_saved(path.to_path_buf());
        self.save_needs_dialog = false;
        self.last_dir = path.parent().map(Path::to_path_buf);
        cx.notify();
        Ok(())
    }

    fn load(&mut self, path: &Path, cx: &mut Context<Self>) -> Loaded {
        let svg = match fs::read_to_string(path) {
            Ok(svg) => svg,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                return Loaded::Failed("Ese archivo no existe :(", err.to_string());
            }
            Err(err) => return Loaded::Failed("No pude leer los contenidos del archivo :(", err.to_string()),
        };

        let (mut controller, warnings) = match Pizarra::from_svg(&svg, self.controller.config()) {
            DeserializeResult::Ok(controller) => (controller, Vec::new()),
            DeserializeResult::Partial(partial) => {
                (partial.read, partial.errors.iter().map(ToString::to_string).collect())
            }
            DeserializeResult::Err(err) => {
                return Loaded::Failed("No pude interpretar el formato de este archivo :(", err.to_string());
            }
        };

        controller.set_saved(path.to_path_buf());
        self.save_needs_dialog = !warnings.is_empty();
        self.last_dir = path.parent().map(Path::to_path_buf);
        self.replace_controller(controller, cx);

        if warnings.is_empty() { Loaded::Ok } else { Loaded::Partial(warnings) }
    }

    // ---------------------------------------------------------------------
    // Dialog flows. They run as tasks because file choosers and prompts are
    // asynchronous in GPUI; `busy` keeps two of them from overlapping.

    fn run_flow<F>(&mut self, window: &mut Window, cx: &mut Context<Self>, flow: F)
    where
        F: AsyncFnOnce(WeakEntity<Self>, &mut AsyncWindowContext) -> Result<()> + 'static,
    {
        if self.busy {
            return;
        }
        self.busy = true;
        self.popover = None;
        cx.notify();

        cx.spawn_in(window, async move |this, cx| {
            if let Err(err) = flow(this.clone(), cx).await {
                log::error!("{err:#}");
            }
            this.update(cx, |this, cx| {
                this.busy = false;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    async fn ask(
        this: &WeakEntity<Self>,
        cx: &mut AsyncWindowContext,
        level: PromptLevel,
        message: &str,
        detail: Option<&str>,
        answers: &[&str],
    ) -> Result<Option<usize>> {
        let answer = this.update_in(cx, |_, window, cx| window.prompt(level, message, detail, answers, cx))?;
        Ok(answer.await.ok())
    }

    async fn show_error(this: &WeakEntity<Self>, cx: &mut AsyncWindowContext, message: &str, detail: &str) -> Result<()> {
        Self::ask(this, cx, PromptLevel::Critical, message, Some(detail), &["Chales"]).await?;
        Ok(())
    }

    /// Asks what to do with unsaved changes. `Ok(true)` means the caller may
    /// throw the current drawing away.
    async fn confirm_discard(this: &WeakEntity<Self>, cx: &mut AsyncWindowContext, why: Discard) -> Result<bool> {
        let status = this.read_with(cx, |this, _| this.controller.get_save_status().clone())?;
        let (message, detail) = match (status, why) {
            (SaveStatus::NewAndEmpty | SaveStatus::Saved(_), _) => return Ok(true),
            (SaveStatus::NewAndChanged, Discard::Replace) => UNSAVED_NEW_FILE,
            (SaveStatus::Unsaved(_), Discard::Replace) => UNSAVED_SINCE_LAST_TIME,
            (SaveStatus::NewAndChanged, Discard::Close) => CLOSE_NEW_FILE,
            (SaveStatus::Unsaved(_), Discard::Close) => CLOSE_SINCE_LAST_TIME,
        };

        let answer = Self::ask(this, cx, PromptLevel::Warning, message, Some(detail), &["Sí", "No", "Cancelar"]).await?;
        match answer {
            Some(0) => Self::save(this, cx, false).await,
            Some(1) => Ok(true),
            _ => Ok(false),
        }
    }

    /// Saves to the current file, asking for one when there is none or when
    /// `ask` is set. `Ok(true)` when the drawing was written.
    async fn save(this: &WeakEntity<Self>, cx: &mut AsyncWindowContext, ask: bool) -> Result<bool> {
        let (current, dir, name) =
            this.read_with(cx, |this, _| (this.current_path(), this.dialog_dir(), this.suggested_name("svg")))?;

        let path = match current.filter(|_| !ask) {
            Some(path) => path,
            None => match Self::ask_new_path(this, cx, &dir, &name).await? {
                Some(path) => files::ensure_extension(&path, "svg"),
                None => return Ok(false),
            },
        };

        match this.update(cx, |this, cx| this.write_svg(&path, cx))? {
            Ok(()) => Ok(true),
            Err(err) => {
                Self::show_error(this, cx, "Falló esto:", &format!("{err:#}")).await?;
                Ok(false)
            }
        }
    }

    async fn ask_new_path(
        this: &WeakEntity<Self>,
        cx: &mut AsyncWindowContext,
        dir: &Path,
        name: &str,
    ) -> Result<Option<PathBuf>> {
        let receiver = this.update(cx, |_, cx| cx.prompt_for_new_path(dir, Some(name)))?;
        match receiver.await {
            Ok(Ok(path)) => Ok(path),
            Ok(Err(err)) => {
                Self::show_error(this, cx, "No pude mostrar el selector de archivos", &format!("{err:#}")).await?;
                Ok(None)
            }
            Err(_) => Ok(None),
        }
    }

    async fn open_path(this: &WeakEntity<Self>, cx: &mut AsyncWindowContext, path: PathBuf) -> Result<()> {
        match this.update(cx, |this, cx| this.load(&path, cx))? {
            Loaded::Ok => Ok(()),
            Loaded::Partial(errors) => {
                let mut detail = String::from(
                    "Lo abrí, pero puede que algo no se vea igual. Al guardar te preguntaré dónde, para no \
                     reemplazar el original.\n",
                );
                for err in errors.iter().take(5) {
                    detail.push_str(&format!("\n• {err}"));
                }
                if errors.len() > 5 {
                    detail.push_str(&format!("\n… y {} más", errors.len() - 5));
                }
                Self::ask(this, cx, PromptLevel::Info, "Este archivo no fue hecho con pizarra", Some(&detail), &["Ok"])
                    .await?;
                Ok(())
            }
            Loaded::Failed(message, detail) => Self::show_error(this, cx, message, &detail).await,
        }
    }

    // ---------------------------------------------------------------------
    // Actions

    fn new_drawing(&mut self, _: &NewDrawing, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.controller.get_save_status(), SaveStatus::NewAndEmpty) {
            return;
        }
        self.run_flow(window, cx, async |this, cx| {
            if Self::confirm_discard(&this, cx, Discard::Replace).await? {
                this.update(cx, |this, cx| {
                    this.controller.reset();
                    this.save_needs_dialog = false;
                    this.held_buttons.clear();
                    this.apply_ui_state();
                    this.invalidate(cx);
                })?;
            }
            Ok(())
        });
    }

    fn open(&mut self, _: &Open, window: &mut Window, cx: &mut Context<Self>) {
        self.run_flow(window, cx, async |this, cx| {
            if !Self::confirm_discard(&this, cx, Discard::Replace).await? {
                return Ok(());
            }

            let receiver = this.update(cx, |_, cx| {
                cx.prompt_for_paths(PathPromptOptions {
                    files: true,
                    directories: false,
                    multiple: false,
                    prompt: Some("Abrir".into()),
                })
            })?;

            let path = match receiver.await {
                Ok(Ok(paths)) => paths.and_then(|paths| paths.into_iter().next()),
                Ok(Err(err)) => {
                    Self::show_error(&this, cx, "No pude mostrar el selector de archivos", &format!("{err:#}")).await?;
                    None
                }
                Err(_) => None,
            };

            match path {
                Some(path) => Self::open_path(&this, cx, path).await,
                None => Ok(()),
            }
        });
    }

    fn save(&mut self, _: &Save, window: &mut Window, cx: &mut Context<Self>) {
        match self.controller.get_save_status() {
            SaveStatus::NewAndEmpty | SaveStatus::Saved(_) => {}
            SaveStatus::NewAndChanged | SaveStatus::Unsaved(_) => {
                self.run_flow(window, cx, async |this, cx| Self::save(&this, cx, false).await.map(drop));
            }
        }
    }

    fn save_as(&mut self, _: &SaveAs, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.controller.get_save_status(), SaveStatus::NewAndEmpty) {
            return;
        }
        self.run_flow(window, cx, async |this, cx| Self::save(&this, cx, true).await.map(drop));
    }

    fn export(&mut self, _: &Export, window: &mut Window, cx: &mut Context<Self>) {
        self.run_flow(window, cx, async |this, cx| {
            let job = this.read_with(cx, |this, _| ExportJob::new(&this.controller))?;
            let job = match job {
                Ok(job) => job,
                Err(err) => return Self::show_error(&this, cx, "No hay nada que exportar", &format!("{err:#}")).await,
            };

            let (dir, name) = this.read_with(cx, |this, _| (this.dialog_dir(), this.suggested_name("png")))?;
            let Some(path) = Self::ask_new_path(&this, cx, &dir, &name).await? else {
                return Ok(());
            };
            let path = files::ensure_extension(&path, "png");

            let written = {
                let path = path.clone();
                cx.background_executor().spawn(async move { job.write_png(&path) }).await
            };
            match written {
                Ok(()) => {
                    this.update(cx, |this, _| this.last_dir = path.parent().map(Path::to_path_buf))?;
                    Ok(())
                }
                Err(err) => Self::show_error(&this, cx, "No pude exportar el dibujo", &format!("{err:#}")).await,
            }
        });
    }

    fn quit(&mut self, _: &Quit, window: &mut Window, cx: &mut Context<Self>) {
        if self.should_close(window, cx) {
            window.remove_window();
        }
    }

    /// Called when the window is asked to close. Returns whether it may close
    /// right away; otherwise the unsaved-changes question is shown first.
    fn should_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.allow_close || !self.has_unsaved_changes() {
            return true;
        }
        self.run_flow(window, cx, async |this, cx| {
            if Self::confirm_discard(&this, cx, Discard::Close).await? {
                this.update_in(cx, |this, window, _| {
                    this.allow_close = true;
                    window.remove_window();
                })?;
            }
            Ok(())
        });
        false
    }

    fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        self.controller.undo();
        self.invalidate(cx);
    }

    fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        self.controller.redo();
        self.invalidate(cx);
    }

    fn toggle_fullscreen(&mut self, _: &ToggleFullscreen, window: &mut Window, _: &mut Context<Self>) {
        window.toggle_fullscreen();
    }

    fn about(&mut self, _: &About, _: &mut Window, cx: &mut Context<Self>) {
        self.popover = None;
        self.show_about = true;
        cx.notify();
    }

    fn zoom_in(&mut self, _: &ZoomIn, _: &mut Window, cx: &mut Context<Self>) {
        self.controller.zoom_in();
        self.invalidate(cx);
    }

    fn zoom_out(&mut self, _: &ZoomOut, _: &mut Window, cx: &mut Context<Self>) {
        self.controller.zoom_out();
        self.invalidate(cx);
    }

    fn zoom_home(&mut self, _: &ZoomHome, _: &mut Window, cx: &mut Context<Self>) {
        self.controller.go_home();
        self.invalidate(cx);
    }

    fn choose_color(&mut self, _: &ChooseColor, _: &mut Window, cx: &mut Context<Self>) {
        self.toggle_popover(Popover::Color, cx);
    }

    fn cancel(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) {
        if self.show_about {
            self.show_about = false;
        } else if self.popover.is_some() {
            self.popover = None;
        } else {
            let hint = self.controller.handle_key_released(Key::Escape);
            self.redraw(hint, cx);
        }
        cx.notify();
    }

    fn set_tool(&mut self, tool: Tool, cx: &mut Context<Self>) {
        self.tool = tool;
        self.controller.set_tool(tool.selected());
        self.popover = None;
        cx.notify();
    }

    fn set_color(&mut self, color: Color, cx: &mut Context<Self>) {
        self.color = color.with_alpha(255);
        self.controller.set_color(self.color.with_alpha(alpha_u8(self.alpha)));
        cx.notify();
    }

    fn set_thickness(&mut self, value: f64, cx: &mut Context<Self>) {
        let (min, max, step) = THICKNESS_RANGE;
        self.thickness = ((value / step).round() * step).clamp(min, max);
        self.controller.set_stroke(self.thickness.into());
        cx.notify();
    }

    fn set_alpha(&mut self, value: f64, cx: &mut Context<Self>) {
        self.alpha = ((value / ALPHA_STEP).round() * ALPHA_STEP).clamp(0.0, 1.0);
        self.controller.set_alpha(alpha_u8(self.alpha));
        cx.notify();
    }

    fn toggle_popover(&mut self, popover: Popover, cx: &mut Context<Self>) {
        self.popover = if self.popover == Some(popover) { None } else { Some(popover) };
        self.dragging = None;
        cx.notify();
    }

    fn close_popover(&mut self, cx: &mut Context<Self>) {
        self.popover = None;
        self.dragging = None;
        cx.notify();
    }

    // ---------------------------------------------------------------------
    // Canvas input

    fn to_canvas(&self, position: Point<Pixels>) -> Vec2D<pizarra::ScreenUnit> {
        let origin = self.canvas_bounds.map(|b| b.origin).unwrap_or_default();
        Vec2D::new_screen(f64::from(position.x - origin.x), f64::from(position.y - origin.y))
    }

    fn canvas_mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle);
        let Some(button) = to_pizarra_button(event.button) else { return };
        if !self.held_buttons.contains(&event.button) {
            self.held_buttons.push(event.button);
        }
        let hint = self.controller.handle_mouse_button_pressed_flags(button, self.to_canvas(event.position), None);
        self.redraw(hint, cx);
    }

    fn canvas_mouse_move(&mut self, event: &MouseMoveEvent, hovered: bool, cx: &mut Context<Self>) {
        if !hovered && self.held_buttons.is_empty() {
            return;
        }

        // A release that happened where we could not see it (e.g. outside the
        // window): finish the gesture instead of leaving the tool stuck.
        if event.pressed_button.is_none() && !self.held_buttons.is_empty() {
            for button in std::mem::take(&mut self.held_buttons) {
                if let Some(button) = to_pizarra_button(button) {
                    let at = self.to_canvas(event.position);
                    self.controller.handle_mouse_button_released_flags(button, at, flags(&event.modifiers), None);
                }
            }
            self.shapes.invalidate();
        }

        let hint = self.controller.handle_mouse_move_flags(self.to_canvas(event.position), flags(&event.modifiers), None);
        self.redraw(hint, cx);
    }

    fn canvas_mouse_up(&mut self, event: &MouseUpEvent, cx: &mut Context<Self>) {
        let Some(ix) = self.held_buttons.iter().position(|b| *b == event.button) else { return };
        self.held_buttons.remove(ix);
        let Some(button) = to_pizarra_button(event.button) else { return };
        self.controller.handle_mouse_button_released_flags(
            button,
            self.to_canvas(event.position),
            flags(&event.modifiers),
            None,
        );
        // A finished shape moves from the live layer to the stored one.
        self.invalidate(cx);
    }

    fn canvas_scroll(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        // pizarra expects GTK deltas: one unit per notch, positive when
        // scrolling down, multiplied by `scroll_factor` to get pixels.
        let factor = self.controller.config().scroll_factor;
        let (mut dx, mut dy) = match event.delta {
            ScrollDelta::Lines(d) => (-f64::from(d.x) / LINES_PER_NOTCH, -f64::from(d.y) / LINES_PER_NOTCH),
            // Touchpads: move the drawing exactly as far as the fingers did.
            ScrollDelta::Pixels(d) if factor != 0.0 => (f64::from(d.x) / factor, f64::from(d.y) / factor),
            ScrollDelta::Pixels(_) => return,
        };
        // Shift+wheel rotates in pizarra, but GPUI turns it into horizontal
        // scrolling on X11.
        if event.modifiers.shift && dy == 0.0 {
            (dx, dy) = (0.0, dx);
        }
        self.controller.scroll(Vec2D::new_screen(dx, dy), flags(&event.modifiers));
        self.invalidate(cx);
    }

    fn paint_canvas(&mut self, bounds: Bounds<Pixels>, window: &mut Window) {
        if self.canvas_bounds.map(|b| b.size) != Some(bounds.size) {
            self.controller
                .resize(Vec2D::new_screen(f64::from(bounds.size.width), f64::from(bounds.size.height)));
            self.shapes.invalidate();
        }
        self.canvas_bounds = Some(bounds);

        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            window.paint_quad(fill(bounds, theme::to_hsla(self.controller.bgcolor())));

            canvas::paint(window, self.shapes.paths(&self.controller, bounds));

            let (world, screen) = canvas::transforms(&self.controller, bounds);
            if let Some(commands) = self.controller.draw_commands_for_current_shape() {
                canvas::paint(window, &canvas::build(&commands, world, screen, bounds));
            }
            let helpers = self.controller.draw_commands_for_tool();
            if !helpers.is_empty() {
                canvas::paint(window, &canvas::build(&helpers, world, screen, bounds));
            }
        });
    }

    // ---------------------------------------------------------------------
    // Rendering

    fn render_canvas(&self, cx: &Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        div().flex_1().w_full().min_h_0().overflow_hidden().child(
            canvas(
                |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
                move |bounds, hitbox, window, cx| {
                    window.set_cursor_style(CursorStyle::Crosshair, &hitbox);
                    entity.update(cx, |this, _| this.paint_canvas(bounds, window));

                    let (this, hb) = (entity.clone(), hitbox.clone());
                    window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                        if phase == DispatchPhase::Bubble && hb.is_hovered(window) {
                            this.update(cx, |this, cx| this.canvas_mouse_down(event, window, cx));
                        }
                    });
                    let (this, hb) = (entity.clone(), hitbox.clone());
                    window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                        if phase == DispatchPhase::Bubble {
                            let hovered = hb.is_hovered(window);
                            this.update(cx, |this, cx| this.canvas_mouse_move(event, hovered, cx));
                        }
                    });
                    let this = entity.clone();
                    window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                        if phase == DispatchPhase::Bubble {
                            this.update(cx, |this, cx| this.canvas_mouse_up(event, cx));
                        }
                    });
                    let this = entity.clone();
                    window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
                        if phase == DispatchPhase::Bubble && hitbox.should_handle_scroll(window) {
                            this.update(cx, |this, cx| this.canvas_scroll(event, cx));
                        }
                    });
                },
            )
            .size_full(),
        )
    }

    fn render_toolbar(&self, cx: &Context<Self>) -> impl IntoElement {
        let (title, subtitle) = self.titles();
        let swatch = theme::to_hsla(self.color.with_alpha(alpha_u8(self.alpha)));

        let color_button = self
            .popover_button(Popover::Color, "Color del trazo (Alt+C)", cx)
            .child(
                div()
                    .size(px(20.))
                    .rounded_full()
                    .border_1()
                    .border_color(theme::muted())
                    .bg(swatch),
            );
        let tool_button = self
            .popover_button(Popover::Tools, "Cambia la herramienta de dibujo", cx)
            .child(icon(self.tool.icon(), theme::accent()));
        let thickness_button = self
            .popover_button(Popover::Thickness, "Grosor del trazo", cx)
            .gap_1()
            .child(icon("icons/thickness.svg", theme::accent()))
            .child(div().text_xs().text_color(theme::muted()).child(format!("{}", self.thickness)));
        let alpha_button = self
            .popover_button(Popover::Alpha, "Opacidad del trazo", cx)
            .gap_1()
            .child(icon("icons/alpha.svg", theme::accent()))
            .child(div().text_xs().text_color(theme::muted()).child(format!("{:.0}%", self.alpha * 100.0)));

        let titles = div()
            .flex_1()
            .min_w_0()
            .px_2()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .overflow_hidden()
            .child(div().font_weight(FontWeight::SEMIBOLD).truncate().child(title))
            .children(subtitle.map(|s| div().text_xs().text_color(theme::muted()).truncate().child(s)));

        div()
            .flex_none()
            .h(px(TOOLBAR_HEIGHT))
            .w_full()
            .px_2()
            .flex()
            .flex_row()
            .items_center()
            .gap_1()
            .bg(theme::toolbar_bg())
            .border_b_1()
            .border_color(theme::border())
            .child(color_button)
            .child(tool_button)
            .child(thickness_button)
            .child(alpha_button)
            .child(separator())
            .child(self.icon_button("undo", "icons/undo.svg", "Deshacer (Ctrl+Z)", cx, |this, w, cx| {
                this.undo(&Undo, w, cx)
            }))
            .child(self.icon_button("redo", "icons/redo.svg", "Rehacer (Ctrl+Shift+Z)", cx, |this, w, cx| {
                this.redo(&Redo, w, cx)
            }))
            .child(titles)
            .child(self.icon_button("zoom-out", "icons/zoom_out.svg", "Alejar (-)", cx, |this, w, cx| {
                this.zoom_out(&ZoomOut, w, cx)
            }))
            .child(self.icon_button("zoom-home", "icons/zoom_home.svg", "Regresar al origen (0)", cx, |this, w, cx| {
                this.zoom_home(&ZoomHome, w, cx)
            }))
            .child(self.icon_button("zoom-in", "icons/zoom_in.svg", "Acercar (+)", cx, |this, w, cx| {
                this.zoom_in(&ZoomIn, w, cx)
            }))
            .child(separator())
            .child(
                self.popover_button(Popover::Menu, "Opciones de la pizarra", cx)
                    .child(icon("icons/menu.svg", theme::text())),
            )
    }

    fn icon_button(
        &self,
        id: &'static str,
        path: &'static str,
        tooltip_text: &'static str,
        cx: &Context<Self>,
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> impl IntoElement {
        button_base(id)
            .tooltip(tooltip(tooltip_text))
            .on_click(cx.listener(move |this, _, window, cx| on_click(this, window, cx)))
            .child(icon(path, theme::text()))
    }

    /// A header bar button that opens `popover`, remembering where it is so
    /// the popover can be placed under it.
    fn popover_button(&self, popover: Popover, tooltip_text: &'static str, cx: &Context<Self>) -> gpui::Stateful<gpui::Div> {
        let anchors = self.anchors.clone();
        let open = self.popover == Some(popover);
        button_base(ElementId::Name(format!("popover-{popover:?}").into()))
            .relative()
            .when(open, |b| b.bg(theme::pressed()))
            .tooltip(tooltip(tooltip_text))
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_popover(popover, cx)))
            .on_scroll_wheel(cx.listener(move |this, event: &ScrollWheelEvent, _, cx| {
                // Like GTK's scale buttons, the wheel adjusts the value.
                let up = event.delta.pixel_delta(px(1.)).y > px(0.);
                let sign = if up { 1.0 } else { -1.0 };
                match popover {
                    Popover::Thickness => this.set_thickness(this.thickness + sign * THICKNESS_RANGE.2, cx),
                    Popover::Alpha => this.set_alpha(this.alpha + sign * ALPHA_STEP, cx),
                    _ => {}
                }
            }))
            .child(
                canvas(move |bounds, _, _| anchors[popover.index()].set(Some(bounds)), |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
    }

    fn render_popover(&self, popover: Popover, cx: &Context<Self>) -> impl IntoElement {
        let anchor = self.anchors[popover.index()].get().unwrap_or_default();
        let (corner, position) = match popover {
            Popover::Menu => (Corner::TopRight, anchor.bottom_right()),
            _ => (Corner::TopLeft, anchor.bottom_left()),
        };

        let content = match popover {
            Popover::Color => self.render_color_popover(cx).into_any_element(),
            Popover::Tools => self.render_tools_popover(cx).into_any_element(),
            Popover::Thickness => self.render_value_popover(Slider::Thickness, cx).into_any_element(),
            Popover::Alpha => self.render_value_popover(Slider::Alpha, cx).into_any_element(),
            Popover::Menu => self.render_menu_popover(cx).into_any_element(),
        };

        let close = |cx: &Context<Self>| cx.listener(|this, _: &MouseDownEvent, _, cx| this.close_popover(cx));

        deferred(
            div()
                .id("popover-scrim")
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .occlude()
                .on_mouse_down(MouseButton::Left, close(cx))
                .on_mouse_down(MouseButton::Middle, close(cx))
                .on_mouse_down(MouseButton::Right, close(cx))
                .child(
                    anchored().anchor(corner).position(position + point(px(0.), px(4.))).snap_to_window_with_margin(px(8.)).child(
                        div()
                            .id("popover")
                            .occlude()
                            .p_2()
                            .rounded_lg()
                            .bg(theme::popover_bg())
                            .border_1()
                            .border_color(theme::border())
                            .shadow_lg()
                            .text_color(theme::text())
                            .child(content),
                    ),
                ),
        )
        .with_priority(1)
    }

    fn render_tools_popover(&self, cx: &Context<Self>) -> impl IntoElement {
        div().w(px(300.)).flex().flex_col().children(Tool::ALL.into_iter().map(|tool| {
            let selected = tool == self.tool;
            menu_row(ElementId::Name(format!("tool-{tool:?}").into()))
                .when(selected, |row| row.bg(theme::pressed()))
                .on_click(cx.listener(move |this, _, _, cx| this.set_tool(tool, cx)))
                .child(icon(tool.icon(), theme::accent()))
                .child(div().flex_1().child(tool.label()))
                .child(div().text_xs().text_color(theme::muted()).child(tool.shortcut()))
        }))
    }

    fn render_menu_popover(&self, cx: &Context<Self>) -> impl IntoElement {
        type Handler = fn(&mut PizarraWindow, &mut Window, &mut Context<PizarraWindow>);
        let sections: [(&str, &[(&str, &str, Handler)]); 4] = [
            (
                "Archivo",
                &[
                    ("Nuevo", "Ctrl+N", |t, w, cx| t.new_drawing(&NewDrawing, w, cx)),
                    ("Abrir", "Ctrl+O", |t, w, cx| t.open(&Open, w, cx)),
                    ("Guardar", "Ctrl+S", |t, w, cx| t.save(&Save, w, cx)),
                    ("Guardar como", "Ctrl+Shift+S", |t, w, cx| t.save_as(&SaveAs, w, cx)),
                    ("Exportar", "Ctrl+Shift+E", |t, w, cx| t.export(&Export, w, cx)),
                    ("Salir", "Ctrl+Q", |t, w, cx| t.quit(&Quit, w, cx)),
                ],
            ),
            (
                "Editar",
                &[
                    ("Deshacer", "Ctrl+Z", |t, w, cx| t.undo(&Undo, w, cx)),
                    ("Rehacer", "Ctrl+Shift+Z", |t, w, cx| t.redo(&Redo, w, cx)),
                ],
            ),
            ("Ver", &[("Pantalla completa", "F11", |t, w, cx| t.toggle_fullscreen(&ToggleFullscreen, w, cx))]),
            ("Ayuda", &[("Acerca de", "", |t, w, cx| t.about(&About, w, cx))]),
        ];

        div().w(px(250.)).flex().flex_col().children(sections.into_iter().enumerate().map(|(ix, (title, items))| {
            div()
                .flex()
                .flex_col()
                .when(ix > 0, |d| d.mt_1().pt_1().border_t_1().border_color(theme::border()))
                .child(div().px_2().py_1().text_xs().text_color(theme::muted()).child(title))
                .children(items.iter().map(|&(label, shortcut, handler)| {
                    menu_row(ElementId::Name(format!("menu-{label}").into()))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.close_popover(cx);
                            handler(this, window, cx);
                        }))
                        .child(div().flex_1().child(label))
                        .child(div().text_xs().text_color(theme::muted()).child(shortcut))
                }))
        }))
    }

    fn render_color_popover(&self, cx: &Context<Self>) -> impl IntoElement {
        let current = self.color;
        let palette = div().flex().flex_row().gap_1().children(theme::PALETTE.iter().enumerate().map(|(col, shades)| {
            div().flex().flex_col().gap_1().children(shades.iter().enumerate().map(move |(row, &hex)| {
                let color = theme::palette_color(hex);
                let selected = color == current;
                div()
                    .id(ElementId::NamedInteger("swatch".into(), (col * 5 + row) as u64))
                    .size(px(26.))
                    .rounded_md()
                    .cursor_pointer()
                    .bg(theme::to_hsla(color))
                    .border_2()
                    .border_color(if selected { theme::text() } else { theme::border() })
                    .hover(|s| s.border_color(theme::muted()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_color(color, cx);
                        this.close_popover(cx);
                    }))
            }))
        }));

        let channel = |label: &'static str, slider: Slider, value: u8| {
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .child(div().w(px(14.)).text_xs().text_color(theme::muted()).child(label))
                .child(self.render_slider(slider, cx))
                .child(div().w(px(28.)).text_xs().text_color(theme::muted()).child(value.to_string()))
        };

        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(palette)
            .child(div().mt_1().text_xs().text_color(theme::muted()).child("Personalizado"))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(channel("R", Slider::Red, current.r))
                            .child(channel("G", Slider::Green, current.g))
                            .child(channel("B", Slider::Blue, current.b)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap_1()
                            .child(
                                div()
                                    .size(px(36.))
                                    .rounded_md()
                                    .border_1()
                                    .border_color(theme::border())
                                    .bg(theme::to_hsla(current)),
                            )
                            .child(div().text_xs().text_color(theme::muted()).child(current.css())),
                    ),
            )
    }

    fn render_value_popover(&self, slider: Slider, cx: &Context<Self>) -> impl IntoElement {
        let (label, step): (String, f64) = match slider {
            Slider::Thickness => (format!("{} px", self.thickness), THICKNESS_RANGE.2),
            _ => (format!("{:.0}%", self.alpha * 100.0), ALPHA_STEP),
        };
        let nudge = move |delta: f64| {
            cx.listener(move |this: &mut Self, _: &gpui::ClickEvent, _, cx| match slider {
                Slider::Thickness => this.set_thickness(this.thickness + delta, cx),
                _ => this.set_alpha(this.alpha + delta, cx),
            })
        };

        div()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .child(button_base(ElementId::Name(format!("{slider:?}-minus").into())).on_click(nudge(-step)).child("−"))
            .child(self.render_slider(slider, cx))
            .child(button_base(ElementId::Name(format!("{slider:?}-plus").into())).on_click(nudge(step)).child("+"))
            .child(div().w(px(48.)).text_sm().text_color(theme::muted()).child(label))
    }

    fn slider_fraction(&self, slider: Slider) -> f64 {
        match slider {
            Slider::Thickness => {
                let (min, max, _) = THICKNESS_RANGE;
                (self.thickness - min) / (max - min)
            }
            Slider::Alpha => self.alpha,
            Slider::Red => f64::from(self.color.r) / 255.0,
            Slider::Green => f64::from(self.color.g) / 255.0,
            Slider::Blue => f64::from(self.color.b) / 255.0,
        }
    }

    fn set_slider(&mut self, slider: Slider, fraction: f64, cx: &mut Context<Self>) {
        let fraction = fraction.clamp(0.0, 1.0);
        let channel = (fraction * 255.0).round() as u8;
        match slider {
            Slider::Thickness => {
                let (min, max, _) = THICKNESS_RANGE;
                self.set_thickness(min + fraction * (max - min), cx);
            }
            Slider::Alpha => self.set_alpha(fraction, cx),
            Slider::Red => self.set_color(Color { r: channel, ..self.color }, cx),
            Slider::Green => self.set_color(Color { g: channel, ..self.color }, cx),
            Slider::Blue => self.set_color(Color { b: channel, ..self.color }, cx),
        }
    }

    fn render_slider(&self, slider: Slider, cx: &Context<Self>) -> impl IntoElement {
        const WIDTH: f32 = 170.0;
        const KNOB: f32 = 14.0;
        let travel = WIDTH - KNOB;
        let fraction = self.slider_fraction(slider) as f32;
        let fill_color = match slider {
            Slider::Red => gpui::rgb(0xe01b24),
            Slider::Green => gpui::rgb(0x33d17a),
            Slider::Blue => gpui::rgb(0x3584e4),
            _ => theme::accent(),
        };

        let entity = cx.entity();
        let value_at = move |bounds: Bounds<Pixels>, position: Point<Pixels>| {
            f64::from((f32::from(position.x - bounds.origin.x) - KNOB / 2.0) / travel)
        };

        div()
            .relative()
            .w(px(WIDTH))
            .h(px(20.))
            .cursor_pointer()
            .child(
                div()
                    .absolute()
                    .left(px(KNOB / 2.0))
                    .top(px(8.))
                    .w(px(travel))
                    .h(px(4.))
                    .rounded_full()
                    .bg(theme::pressed()),
            )
            .child(
                div()
                    .absolute()
                    .left(px(KNOB / 2.0))
                    .top(px(8.))
                    .w(px(travel * fraction))
                    .h(px(4.))
                    .rounded_full()
                    .bg(fill_color),
            )
            .child(
                div()
                    .absolute()
                    .left(px(travel * fraction))
                    .top(px(3.))
                    .size(px(KNOB))
                    .rounded_full()
                    .bg(theme::text())
                    .border_1()
                    .border_color(theme::border()),
            )
            // Mouse handling is registered as window-wide listeners so a drag
            // keeps working when the pointer leaves the slider.
            .child(
                canvas(
                    |bounds, _, _| bounds,
                    move |_, bounds, window, _| {
                        let this = entity.clone();
                        window.on_mouse_event(move |event: &MouseDownEvent, phase, _, cx| {
                            if phase == DispatchPhase::Bubble
                                && event.button == MouseButton::Left
                                && bounds.contains(&event.position)
                            {
                                this.update(cx, |this, cx| {
                                    this.dragging = Some(slider);
                                    this.set_slider(slider, value_at(bounds, event.position), cx);
                                });
                            }
                        });
                        let this = entity.clone();
                        window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                            if phase != DispatchPhase::Bubble {
                                return;
                            }
                            this.update(cx, |this, cx| {
                                if this.dragging != Some(slider) {
                                    return;
                                }
                                if event.pressed_button == Some(MouseButton::Left) {
                                    this.set_slider(slider, value_at(bounds, event.position), cx);
                                } else {
                                    this.dragging = None;
                                }
                            });
                        });
                        let this = entity.clone();
                        window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                            if phase == DispatchPhase::Bubble && event.button == MouseButton::Left {
                                this.update(cx, |this, _| {
                                    if this.dragging == Some(slider) {
                                        this.dragging = None;
                                    }
                                });
                            }
                        });
                    },
                )
                .absolute()
                .size_full(),
            )
    }

    fn render_about(&self, cx: &Context<Self>) -> impl IntoElement {
        let line = |text: &'static str| div().text_xs().text_color(theme::muted()).child(text);
        deferred(
            div()
                .id("about-scrim")
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .occlude()
                .bg(theme::scrim())
                .flex()
                .items_center()
                .justify_center()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.show_about = false;
                        cx.notify();
                    }),
                )
                .child(
                    div()
                        .id("about")
                        .occlude()
                        .w(px(420.))
                        .p_6()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap_2()
                        .rounded_xl()
                        .bg(theme::popover_bg())
                        .border_1()
                        .border_color(theme::border())
                        .shadow_lg()
                        .text_color(theme::text())
                        .child(img("pizarra128.png").size(px(96.)))
                        .child(div().text_xl().font_weight(FontWeight::BOLD).child("Pizarra"))
                        .child(div().text_sm().text_color(theme::muted()).child(concat!("Versión ", env!("CARGO_PKG_VERSION"))))
                        .child(div().mt_1().child("Como un pizarrón de gis, pero digital, vectorial e infinito."))
                        .child(
                            div()
                                .id("website")
                                .text_sm()
                                .text_color(theme::accent())
                                .cursor_pointer()
                                .hover(|s| s.underline())
                                .on_click(|_, _, cx| cx.open_url("https://pizarra.categulario.xyz"))
                                .child("pizarra.categulario.xyz"),
                        )
                        .child(div().mt_2().flex().flex_col().items_center()
                            .child(line("Copyright 2022 Abraham Toriz"))
                            .child(line("Contribuciones de Federico Mena, @tyoc213"))
                            .child(line("Interfaz GPUI basada en pizarra-gtk"))
                            .child(line("Licencia GPL 3.0 o posterior")))
                        .child(
                            button_base("about-close")
                                .mt_3()
                                .px_4()
                                .bg(theme::hover())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.show_about = false;
                                    cx.notify();
                                }))
                                .child("Cerrar"),
                        ),
                ),
        )
        .with_priority(2)
    }
}

impl Render for PizarraWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_window_title(window);

        let toolbar = self.render_toolbar(cx);
        let canvas = self.render_canvas(cx);
        let popover = self.popover.map(|p| self.render_popover(p, cx));
        let about = self.show_about.then(|| self.render_about(cx));

        div()
            .id("pizarra")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::new_drawing))
            .on_action(cx.listener(Self::open))
            .on_action(cx.listener(Self::save))
            .on_action(cx.listener(Self::save_as))
            .on_action(cx.listener(Self::export))
            .on_action(cx.listener(Self::quit))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(Self::toggle_fullscreen))
            .on_action(cx.listener(Self::about))
            .on_action(cx.listener(Self::zoom_in))
            .on_action(cx.listener(Self::zoom_out))
            .on_action(cx.listener(Self::zoom_home))
            .on_action(cx.listener(Self::choose_color))
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(|this, _: &ToolPen, _, cx| this.set_tool(Tool::Pen, cx)))
            .on_action(cx.listener(|this, _: &ToolRectangle, _, cx| this.set_tool(Tool::Rectangle, cx)))
            .on_action(cx.listener(|this, _: &ToolPolygon, _, cx| this.set_tool(Tool::Polygon, cx)))
            .on_action(cx.listener(|this, _: &ToolCircle, _, cx| this.set_tool(Tool::Circle, cx)))
            .on_action(cx.listener(|this, _: &ToolCircleThreePoints, _, cx| this.set_tool(Tool::CircleThreePoints, cx)))
            .on_action(cx.listener(|this, _: &ToolEllipse, _, cx| this.set_tool(Tool::Ellipse, cx)))
            .on_action(cx.listener(|this, _: &ToolGrid, _, cx| this.set_tool(Tool::Grid, cx)))
            .on_action(cx.listener(|this, _: &ToolFreeGrid, _, cx| this.set_tool(Tool::FreeGrid, cx)))
            .on_action(cx.listener(|this, _: &ToolEraser, _, cx| this.set_tool(Tool::Eraser, cx)))
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(theme::toolbar_bg())
            .text_color(theme::text())
            .text_sm()
            .child(toolbar)
            .child(canvas)
            .children(popover)
            .children(about)
    }
}

// -------------------------------------------------------------------------
// Small pieces

fn alpha_u8(alpha: f64) -> u8 {
    (alpha.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn to_pizarra_button(button: MouseButton) -> Option<pizarra::MouseButton> {
    match button {
        MouseButton::Left => Some(pizarra::MouseButton::Left),
        MouseButton::Middle => Some(pizarra::MouseButton::Middle),
        MouseButton::Right => Some(pizarra::MouseButton::Right),
        MouseButton::Navigate(_) => None,
    }
}

fn flags(modifiers: &Modifiers) -> Flags {
    Flags { shift: modifiers.shift, ctrl: modifiers.control, alt: modifiers.alt }
}

fn icon(path: &'static str, color: gpui::Rgba) -> impl IntoElement {
    svg().path(path).size(px(18.)).flex_none().text_color(color)
}

fn separator() -> impl IntoElement {
    div().mx_1().w(px(1.)).h(px(22.)).bg(theme::border())
}

fn button_base(id: impl Into<ElementId>) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .flex_row()
        .items_center()
        .justify_center()
        .h(px(32.))
        .min_w(px(32.))
        .px_1p5()
        .rounded_md()
        .cursor_pointer()
        .hover(|s| s.bg(theme::hover()))
        .active(|s| s.bg(theme::pressed()))
}

fn menu_row(id: impl Into<ElementId>) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex()
        .flex_row()
        .items_center()
        .gap_2()
        .px_2()
        .py_1p5()
        .rounded_md()
        .cursor_pointer()
        .hover(|s| s.bg(theme::hover()))
        .active(|s| s.bg(theme::pressed()))
}

struct Tooltip(SharedString);

impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .rounded_md()
            .bg(theme::popover_bg())
            .border_1()
            .border_color(theme::border())
            .shadow_md()
            .text_xs()
            .text_color(theme::text())
            .child(self.0.clone())
    }
}

fn tooltip(text: &'static str) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    move |_, cx| cx.new(|_| Tooltip(text.into())).into()
}
