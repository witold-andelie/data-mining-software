//! `dms`: the desktop application (phase A spike, docs/gui/DESIGN.md). Wires eframe (wgpu,
//! Metal on macOS), the native macOS menu bar, file dialogs and state persistence to the
//! `dms-ui` workbench, which does everything else.
//!
//! ```text
//! dms [FILE.str]            open a stream
//! dms --synthetic N         show a synthetic stream of N nodes
//! dms --bench N [--vsync]   draw N synthetic nodes while panning, print frame times, exit
//! dms --frame-time          show the CPU time of each frame in the status bar
//! ```

#[cfg(target_os = "macos")]
mod macos_menu;

use dms_engine::session::Document;
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

#[derive(Debug, Default)]
struct Options {
    file: Option<PathBuf>,
    synthetic: Option<usize>,
    bench: Option<usize>,
    vsync: bool,
    frame_time: bool,
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
    let result = eframe::run_native(
        APP_NAME,
        native,
        Box::new(move |cc| Ok(Box::new(DmsApp::new(cc, options, launched)))),
    );
    match result {
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

struct DmsApp {
    workbench: Workbench,
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

        let synthetic_nodes = options.bench.or(options.synthetic);
        let mut workbench = match synthetic_nodes {
            Some(nodes) => Workbench::with_document(
                Document {
                    path: PathBuf::from(format!("synthetic-{nodes}.str")),
                    stream: synthetic::grid(nodes, 20),
                },
                state,
            ),
            None => Workbench::new(state),
        };
        if let Some(file) = options.file {
            workbench.open_path(file);
        }
        DmsApp {
            workbench,
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
            self.workbench.dispatch(ctx, "view.zoom_in", &mut || None);
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

        #[allow(unused_mut)]
        let mut requested = self.workbench.ui(ui, &self.platform);
        #[cfg(target_os = "macos")]
        if let Some(menu) = &mut self.menu {
            requested.extend(menu.take_commands());
            menu.sync_enabled(&self.workbench.session);
        }
        for id in requested {
            self.workbench.dispatch(&ctx, id, &mut pick_stream);
        }
        self.bench_step(&ctx, cpu_ms);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, STATE_KEY, &self.workbench.state);
    }
}

/// The native open dialog (runs on the main thread, as macOS requires).
fn pick_stream() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Open Stream")
        .add_filter("Streams", &["str"])
        .pick_file()
}
