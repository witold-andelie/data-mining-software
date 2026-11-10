//! `dms`: the desktop application (phase A spike, docs/gui/DESIGN.md). Wires eframe (wgpu,
//! Metal on macOS), the native macOS menu bar, file dialogs and state persistence to the
//! `dms-ui` workbench, which does everything else.
//!
//! ```text
//! dms [FILE.str]            open a stream
//! dms --synthetic N         show a synthetic stream of N nodes
//! dms --bench N [--vsync]   draw N synthetic nodes while panning, print frame times, exit
//! dms --frame-time          show the CPU time of each frame in the status bar
//! dms --data-dir DIR ...    also search DIR for data the stream names (File > Add Data Folder…)
//! dms FILE.str --run [--report OUT]
//!                           run the stream once opened; with --report, write a summary of the
//!                           run (counts and result kinds, no data) to OUT and quit (smoke test)
//! ```
//!
//! On macOS, files double-clicked in Finder, chosen with "Open With" or dropped on the Dock icon
//! arrive through `dms-macos-open`, installed before the event loop runs.

#[cfg(target_os = "macos")]
mod macos_menu;

use dms_engine::session::Document;
use dms_ui::i18n::{self, tr, tr_fmt};
use dms_ui::workbench::{Clipboard, Pick};
use dms_ui::{Platform, ShellState, Workbench, synthetic};
use eframe::egui;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

const APP_NAME: &str = "Data Mining";
const STATE_KEY: &str = "shell";
/// Largest synthetic stream the command line accepts.
const MAX_SYNTHETIC: usize = 5_000;
const BENCH_WARMUP: usize = 30;
const BENCH_FRAMES: usize = 600;

struct SystemClipboard(Result<arboard::Clipboard, String>);

impl Clipboard for SystemClipboard {
    fn get_text(&mut self) -> Result<String, String> {
        self.0
            .as_mut()
            .map_err(|e| e.clone())?
            .get_text()
            .map_err(|e| {
                tr_fmt(
                    "Cannot read clipboard: {error}",
                    &[("error", &e.to_string())],
                )
            })
    }

    fn set_text(&mut self, text: &str) -> Result<(), String> {
        self.0
            .as_mut()
            .map_err(|e| e.clone())?
            .set_text(text)
            .map_err(|e| {
                tr_fmt(
                    "Cannot write clipboard: {error}",
                    &[("error", &e.to_string())],
                )
            })
    }
}

fn autosave_directory() -> Option<PathBuf> {
    if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(|p| PathBuf::from(p).join("Data Mining").join("Autosave"))
    } else {
        std::env::var_os("HOME")
            .map(|p| PathBuf::from(p).join("Library/Application Support/Data Mining/Autosave"))
    }
}

#[derive(Debug, Default)]
struct Options {
    file: Option<PathBuf>,
    synthetic: Option<usize>,
    bench: Option<usize>,
    vsync: bool,
    frame_time: bool,
    run: bool,
    report: Option<PathBuf>,
    data_dirs: Vec<PathBuf>,
}

fn parse_args() -> Result<Options, String> {
    let mut options = Options::default();
    let mut args = std::env::args_os().skip(1);
    let count = |value: Option<std::ffi::OsString>, flag: &str| -> Result<usize, String> {
        value
            .and_then(|v| v.into_string().ok())
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|n| (1..=MAX_SYNTHETIC).contains(n))
            .ok_or_else(|| format!("{flag} takes a node count from 1 to {MAX_SYNTHETIC}"))
    };
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--synthetic") => options.synthetic = Some(count(args.next(), "--synthetic")?),
            Some("--bench") => options.bench = Some(count(args.next(), "--bench")?),
            Some("--vsync") => options.vsync = true,
            Some("--frame-time") => options.frame_time = true,
            Some("--run") => options.run = true,
            Some("--data-dir") => options.data_dirs.push(PathBuf::from(
                args.next().ok_or("--data-dir takes a folder")?,
            )),
            Some("--report") => {
                options.report = Some(PathBuf::from(
                    args.next().ok_or("--report takes an output file")?,
                ))
            }
            Some(flag) if flag.starts_with("--") => return Err(format!("unknown option {flag}")),
            _ => options.file = Some(PathBuf::from(arg)),
        }
    }
    Ok(options)
}

fn main() -> ExitCode {
    let launched = Instant::now();
    let options = match parse_args() {
        Ok(options) => options,
        Err(message) => {
            eprintln!("dms: {message}");
            return ExitCode::from(2);
        }
    };
    let native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(APP_NAME)
            .with_app_id("dms")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([640.0, 400.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    let mut native = native;
    if options.bench.is_some() && !options.vsync {
        // Measure throughput rather than the display's refresh rate.
        native.wgpu_options.surface.present_mode = eframe::wgpu::PresentMode::AutoNoVsync;
    }
    let event_loop =
        match winit::event_loop::EventLoop::<eframe::UserEvent>::with_user_event().build() {
            Ok(event_loop) => event_loop,
            Err(error) => {
                eprintln!("dms: cannot create the event loop: {error}");
                return ExitCode::FAILURE;
            }
        };
    // Before the loop runs: files that launch the app arrive while it finishes launching.
    if cfg!(target_os = "macos") && !dms_macos_open::install() {
        eprintln!("dms: files opened from Finder or the Dock will not reach this window");
    }
    let mut app = eframe::create_native(
        APP_NAME,
        native,
        Box::new(move |cc| Ok(Box::new(DmsApp::new(cc, options, launched)))),
        &event_loop,
    );
    match event_loop.run_app(&mut app) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("dms: {error}");
            ExitCode::FAILURE
        }
    }
}

struct Bench {
    launched: Instant,
    frame: usize,
    started: Option<Instant>,
    cpu_ms: Vec<f32>,
}

/// `--run [--report OUT]`: run the stream once it is open, then optionally report and quit.
struct Smoke {
    started: bool,
    report: Option<PathBuf>,
}

struct DmsApp {
    workbench: Workbench,
    smoke: Option<Smoke>,
    platform: Platform,
    show_frame_time: bool,
    bench: Option<Bench>,
    #[cfg(target_os = "macos")]
    menu: Option<macos_menu::NativeMenu>,
}

impl DmsApp {
    fn new(cc: &eframe::CreationContext<'_>, options: Options, launched: Instant) -> Self {
        let mut state: ShellState = cc
            .storage
            .and_then(|storage| eframe::get_value(storage, STATE_KEY))
            .unwrap_or_default();
        state.sanitize();
        i18n::set_system_language(dms_macos_open::preferred_language().as_deref());
        i18n::set_language(state.language);

        #[cfg(target_os = "macos")]
        let menu = match macos_menu::NativeMenu::install(&cc.egui_ctx) {
            Ok(menu) => Some(menu),
            Err(error) => {
                eprintln!("dms: native menu unavailable, using the in-window menu: {error}");
                None
            }
        };
        let native_menu = {
            #[cfg(target_os = "macos")]
            {
                menu.is_some()
            }
            #[cfg(not(target_os = "macos"))]
            {
                false
            }
        };

        // Folders given on the command line join the saved ones for this session.
        state.data_folders.extend(options.data_dirs.iter().cloned());
        let synthetic_nodes = options.bench.or(options.synthetic);
        let mut workbench = match synthetic_nodes {
            Some(nodes) => Workbench::with_document(
                Document::from_stream(
                    PathBuf::from(format!("synthetic-{nodes}.str")),
                    synthetic::grid(nodes, 20),
                ),
                state,
            ),
            None => Workbench::new(state),
        };
        workbench.set_clipboard(Box::new(SystemClipboard(
            arboard::Clipboard::new().map_err(|e| {
                tr_fmt(
                    "System clipboard is unavailable: {error}",
                    &[("error", &e.to_string())],
                )
            }),
        )));
        workbench.set_autosave_dir(autosave_directory());
        if let Some(file) = options.file {
            workbench.open_path(file);
        }
        workbench.connect_repaint(&cc.egui_ctx);
        let repaint = cc.egui_ctx.clone();
        dms_macos_open::set_waker(Some(Box::new(move || repaint.request_repaint())));
        DmsApp {
            workbench,
            smoke: options.run.then_some(Smoke {
                started: false,
                report: options.report,
            }),
            platform: Platform {
                mac: cfg!(target_os = "macos"),
                native_menu,
                frame_cpu_ms: None,
            },
            show_frame_time: options.frame_time || options.bench.is_some(),
            bench: options.bench.map(|_| Bench {
                launched,
                frame: 0,
                started: None,
                cpu_ms: Vec::with_capacity(BENCH_FRAMES),
            }),
            #[cfg(target_os = "macos")]
            menu,
        }
    }

    fn smoke_step(&mut self, ctx: &egui::Context) {
        let Some(smoke) = &mut self.smoke else {
            return;
        };
        if !smoke.started {
            if self.workbench.session.document().is_some() {
                smoke.started = true;
                self.workbench.dispatch(ctx, "run.stream", &mut |_| None);
            }
            return;
        }
        if !matches!(
            self.workbench.session.run_state(),
            dms_engine::session::RunState::Idle
        ) {
            return;
        }
        let Some(report_path) = smoke.report.take() else {
            self.smoke = None;
            return;
        };
        let text = match self.workbench.session.results() {
            Some(results) => run_summary(&results.report),
            None => "no results\n".to_string(),
        };
        if let Err(error) = std::fs::write(&report_path, text) {
            eprintln!("dms: cannot write {}: {error}", report_path.display());
        }
        self.smoke = None;
        self.workbench.quit_without_saving(ctx);
    }

    fn bench_step(&mut self, ctx: &egui::Context, cpu_ms: Option<f32>) {
        let Some(bench) = &mut self.bench else {
            return;
        };
        bench.frame += 1;
        if bench.frame == 1 {
            println!(
                "dms bench: first frame {:.0} ms after launch",
                bench.launched.elapsed().as_secs_f64() * 1000.0
            );
        }
        if bench.frame == BENCH_WARMUP {
            bench.started = Some(Instant::now());
        } else if bench.frame > BENCH_WARMUP
            && let Some(ms) = cpu_ms
        {
            bench.cpu_ms.push(ms);
        }
        if bench.frame == 2 {
            // One zoom step in from "fit": all 200 nodes stay on screen and labels are drawn.
            self.workbench.dispatch(ctx, "view.zoom_in", &mut |_| None);
        }
        let Some(bench) = &mut self.bench else {
            return;
        };
        // Pan back and forth across the stream.
        let phase = (bench.frame as f32 / 90.0).sin();
        let canvas = &mut self.workbench.state.canvas;
        canvas.pan = [canvas.pan[0] + phase * 6.0, canvas.pan[1] + phase * 2.0];
        if bench.frame >= BENCH_WARMUP + BENCH_FRAMES {
            let seconds = bench
                .started
                .map(|s| s.elapsed().as_secs_f64())
                .unwrap_or(f64::NAN);
            let fps = BENCH_FRAMES as f64 / seconds;
            let mut cpu = bench.cpu_ms.clone();
            cpu.sort_by(f32::total_cmp);
            let mean = cpu.iter().sum::<f32>() / cpu.len().max(1) as f32;
            let p95 = cpu.get(cpu.len() * 95 / 100).copied().unwrap_or(f32::NAN);
            let nodes = self.workbench.graph().map_or(0, |g| g.nodes.len());
            println!(
                "dms bench: {nodes} nodes, {BENCH_FRAMES} frames in {seconds:.2} s = {fps:.0} fps; \
                 CPU per frame mean {mean:.2} ms, p95 {p95:.2} ms; zoom {:.2}",
                self.workbench.state.canvas.zoom
            );
            self.bench = None;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        ctx.request_repaint();
    }
}

impl eframe::App for DmsApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let cpu_ms = frame.info().cpu_usage.map(|s| s * 1000.0);
        self.platform.frame_cpu_ms = if self.show_frame_time { cpu_ms } else { None };
        let ctx = ui.ctx().clone();

        // Files from Finder or the Dock: this app shows one stream, so the last one wins.
        if let Some(path) = dms_macos_open::take_paths().pop() {
            eprintln!("dms: asked to open {}", path.display());
            self.workbench.open_path(path);
        }
        #[cfg(target_os = "macos")]
        if let Some(menu) = &mut self.menu {
            for id in menu.take_commands() {
                let filename = self.workbench.save_filename();
                self.workbench
                    .dispatch(&ctx, id, &mut |kind| pick(kind, &filename));
            }
        }
        let requested = self.workbench.ui(ui, &self.platform);
        for id in requested {
            let filename = self.workbench.save_filename();
            self.workbench
                .dispatch(&ctx, id, &mut |kind| pick(kind, &filename));
        }
        #[cfg(target_os = "macos")]
        if let Some(menu) = &mut self.menu {
            if let Err(error) = menu.rebuild_if_language_changed() {
                eprintln!("dms: cannot rebuild native menu: {error}");
            }
            menu.sync_enabled(&self.workbench.session, ctx.egui_wants_keyboard_input());
        }
        self.bench_step(&ctx, cpu_ms);
        self.smoke_step(&ctx);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, STATE_KEY, &self.workbench.state);
    }
}

/// Counts and result kinds of a run, without any data values.
fn run_summary(report: &dms_engine::run::RunReport) -> String {
    let mut lines = vec![
        format!("targets {}", report.targets.len()),
        format!("evaluated {}", report.results.len()),
        format!("root_causes {}", report.root_causes.len()),
        format!("elapsed_ms {}", report.elapsed.as_millis()),
    ];
    for uid in &report.targets {
        let kind = report
            .results
            .get(uid)
            .map_or("missing", dms_ui::results::result_kind);
        lines.push(format!("target {uid} {kind}"));
    }
    for error in &report.root_causes {
        lines.push(format!("root_cause {error}"));
    }
    lines.join("\n") + "\n"
}

/// The native open and folder dialogs (run on the main thread, as macOS requires).
fn pick(kind: Pick, filename: &str) -> Option<PathBuf> {
    match kind {
        Pick::Stream => {
            let dialog = rfd::FileDialog::new().set_title(tr("Open Stream"));
            // No type filter on macOS: the open panel maps an extension to the type this app
            // declares, while `.str` files on a Mac can carry another app's type (Modeler's
            // `com.ibm.spssmac.str`), and then they show greyed out and cannot be chosen. Any
            // file can be picked; one that is not a stream is reported as an open error.
            let dialog = if cfg!(target_os = "macos") {
                dialog
            } else {
                dialog.add_filter(tr("Streams"), &["dms", "str"])
            };
            dialog.pick_file()
        }
        Pick::DataFile { extensions } => {
            let dialog = rfd::FileDialog::new().set_title(tr("Choose Data File"));
            let dialog = if cfg!(target_os = "macos") || extensions.is_empty() {
                dialog
            } else {
                dialog.add_filter(tr("Data files"), extensions)
            };
            dialog.pick_file()
        }
        Pick::Folder => rfd::FileDialog::new()
            .set_title(tr("Add Data Folder"))
            .pick_folder(),
        Pick::Save => rfd::FileDialog::new()
            .set_title(tr("Save Stream"))
            .add_filter(tr("Data Mining Stream"), &["dms"])
            .set_file_name(filename)
            .save_file(),
    }
}
