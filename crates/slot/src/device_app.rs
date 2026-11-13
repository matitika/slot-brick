use std::path::PathBuf;
use std::time::{Duration, Instant};

use slot::frontend::Frontend;
use slot::app::BottomSource;
use slot::pacer::Pacer;
use slot::screen::Screen;
use slot::input::DeviceInput;
use slot::panel_turn::{panel_turn, PanelTurn};
use slot_gfx::{Compositor, DsPass, DsScreen, FbdevSurface, KmsSurface, Surface};
use slot_power::{trace_first_frame, Board, DevicePlatform};

const CARD: &str = "/mnt/sdcard";

const TRIMUI_CARD: &str = "/mnt/SDCARD";

const MIN_FRAME: Duration = Duration::from_millis(12);

const STEP_TIMEOUT: Duration = Duration::from_millis(12);


const LOG_SYNC: Duration = Duration::from_secs(1);

/// Slot OS's init loads the GPU driver beside slot's own start and makes the file
/// SLOT_GPU_READY names once it has: the screen waits for it, the fonts and sound do not.
const GPU_READY_VAR: &str = "SLOT_GPU_READY";
const GPU_WAIT: Duration = Duration::from_secs(10);

/// The CPU's clock policy and temperature, for the Brick's bug reports: on Slot OS nothing
/// but the kernel's defaults sets them.
fn log_cpu() {
    let read = |p: &str| {
        std::fs::read_to_string(p)
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| "?".to_string())
    };
    let cpu = "/sys/devices/system/cpu/cpu0/cpufreq";
    eprintln!(
        "slot: cpu governor {} at {} kHz ({}..{}), temp {}",
        read(&format!("{cpu}/scaling_governor")),
        read(&format!("{cpu}/scaling_cur_freq")),
        read(&format!("{cpu}/scaling_min_freq")),
        read(&format!("{cpu}/scaling_max_freq")),
        read("/sys/class/thermal/thermal_zone0/temp"),
    );
}

/// The CPU's clock now and the warmest thermal zone, for the pace trace.
fn cpu_now() -> String {
    let khz = std::fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/scaling_cur_freq")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "?".into());
    let temp = (0..8)
        .filter_map(|i| {
            std::fs::read_to_string(format!("/sys/class/thermal/thermal_zone{i}/temp")).ok()
        })
        .filter_map(|s| s.trim().parse::<i64>().ok())
        .max()
        .map(|t| format!("{:.0} C", if t > 1000 { t as f64 / 1000.0 } else { t as f64 }))
        .unwrap_or_else(|| "?".into());
    format!("cpu {khz} kHz, {temp}")
}

fn wait_for_gpu() {
    let Some(ready) = std::env::var_os(GPU_READY_VAR).map(PathBuf::from) else {
        return;
    };
    let since = Instant::now();
    while !ready.exists() && since.elapsed() < GPU_WAIT {
        std::thread::sleep(Duration::from_millis(5));
    }
    eprintln!("slot: GPU driver ready after {} ms", since.elapsed().as_millis());
    slot::boot_time::mark("gpu");
}

pub fn run() {
    slot::boot_time::mark("start");
    let _ = std::thread::Builder::new()
        .name("slot-fonts".into())
        .spawn(slot_ui::warm_fonts);
    let audio = std::thread::Builder::new()
        .name("slot-audio-open".into())
        .spawn(slot::audio::opened_sink);
    let build = slot::build_info::Build::current();
    eprintln!("slot: build {} {} ({})", build.version, build.hash, build.date);
    let board = Board::detect();
    eprintln!("slot: board {board:?}");
    let root = std::env::var_os("SLOT_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(if board.is_trimui() { TRIMUI_CARD } else { CARD }));
    // Less sound queued on the Brick: measured, the default trailed the picture by about 110 ms
    // (75 in slot's ring, 36 in the device), which a rhythm game feels. Its frames now keep up, so
    // 4 frames of ring and 20 ms of device buffer; Config/audio-large-buffer keeps the defaults.
    if board.is_trimui() && !root.join(AUDIO_LARGE_BUFFER).is_file() {
        slot::audio::set_ring_frames(4);
        slot::audio::set_device_latency_ms(20);
        eprintln!("slot: audio: 4-frame ring, 20 ms device buffer");
    }
    if board.is_trimui() {
        log_cpu();
    }
    if board.is_trimui() && std::env::var_os("AGS_LINK_TRACE").is_none() {
        // System/slotlink.sh, which brings Wi-Fi up, logs each step to Logs/slotlink.log:
        // the Brick's Wi-Fi is new, and a failure is otherwise silent
        std::env::set_var("AGS_LINK_TRACE", root.join("Logs/slotlink.log"));
    }
    wait_for_gpu();
    let opened: Result<Box<dyn Surface>, _> = match slot::baseos::var("BASEOS_PLATFORM").as_deref()
    {
        Some("rk3568") => KmsSurface::new().map(|s| Box::new(s) as Box<dyn Surface>),
        // the Brick's PowerVR takes no native fbdev window
        _ => FbdevSurface::with_window(!board.is_trimui())
            .map(|s| Box::new(s) as Box<dyn Surface>),
    };
    let mut surface = match opened {
        Ok(s) => s,
        Err(e) => {
            eprintln!("slot: {e}");
            return;
        }
    };
    slot::boot_time::mark("surface");
    let mut compositor = match Compositor::new(&*surface) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("slot: {e}");
            return;
        }
    };
    slot::boot_time::mark("compositor");
    let panel = surface.window_size();
    let env = |key| std::env::var(key).ok();
    let turn = panel_turn(
        panel,
        slot::baseos::var("BASEOS_PANEL_ROTATION_CCW").as_deref(),
        env("SLOT_ROTATE").as_deref(),
    );
    let turned = turn != PanelTurn::None;
    let window = if turned { (panel.1, panel.0) } else { panel };
    eprintln!(
        "slot: baseos target {}, model {}, rotation ccw {}",
        slot::baseos::var("BASEOS_TARGET").unwrap_or_default(),
        slot::baseos::var("BASEOS_MODEL").unwrap_or_default(),
        slot::baseos::var("BASEOS_PANEL_ROTATION_CCW").unwrap_or_default()
    );
    eprintln!(
        "slot: panel {}x{}, canvas {}x{}, turn {turn:?}",
        panel.0, panel.1, window.0, window.1
    );
    if turned {
        if let Err(e) = compositor.set_panel_turn(panel, turn == PanelTurn::Ccw) {
            eprintln!("slot: {e}");
            return;
        }
    }
    let platform = DevicePlatform::probe_board(
        std::path::Path::new("/sys"),
        std::path::Path::new("/dev"),
        root.clone(),
        board,
    );
    eprintln!("slot: {}", platform.report());
    platform.trace_boot();
    slot::boot_time::mark("platform");
    let sink = match audio.map(|h| h.join()) {
        Ok(Ok(sink)) => sink,
        _ => slot::audio::opened_sink(),
    };
    slot::boot_time::mark("audio");
    let mut frontend = Frontend::boot_with(Box::new(platform), sink);
    slot::boot_time::mark("frontend");
    let target = slot::baseos::var("BASEOS_TARGET").unwrap_or_default();
    let keymap = slot::input::evdev::Keymap::of_system();
    frontend.set_left_stick(keymap.has_stick(&target));
    // releases are built for the Anbernics and applied by their launcher
    frontend.set_self_updates(!board.is_trimui());
    // upstream marks where 1024x768 frame art will go in lime; on the Brick, black
    frontend.set_placeholder_bezels(!board.is_trimui());
    // the Brick's LEDs: Settings → General → LEDs
    frontend.set_led_modes(board.is_trimui());
    // GBA's Scale setting picks fit, 4x or full screen on the Brick's panel
    slot_gfx::set_brick_layout(board.is_trimui());
    frontend.upload_boot_faces(&mut compositor);
    slot::boot_time::mark("faces");
    frontend.upload_bezel(&mut compositor, window);
    slot::boot_time::mark("bezel");
    let mut input = DeviceInput::open(keymap, &root);
    slot::boot_time::mark("input");
    if slot::kid::unlock_chord(input.held_at_open()) {
        frontend.app_mut().unlock_kid_mode();
    }
    let card = root.clone();
    let _ = std::thread::Builder::new()
        .name("slot-bootlogo".into())
        .spawn(move || {
            // the Anbernics' boot logo lives on its own partition; on Slot OS it is the card's
            // bootlogo.bmp; stock TrimUI firmware keeps its own, left alone
            if !board.is_trimui() {
                slot::bootlogo::refresh(&card);
            } else if slot_power::slot_os() {
                match slot::bootlogo::refresh_slot_os(&card) {
                    Ok(slot::bootlogo::Outcome::Installed) => {
                        eprintln!("slot: bootlogo: set, shown from the next start")
                    }
                    Ok(slot::bootlogo::Outcome::Invalid) => eprintln!(
                        "slot: bootlogo: Config/bootlogo.bmp is not a 24- or 32-bit bmp, left alone"
                    ),
                    Ok(_) => {}
                    Err(e) => eprintln!("slot: bootlogo: {e}"),
                }
            }
            slot::root::sweep_temps(&card);
        });
    let _ = std::thread::Builder::new()
        .name("slot-logsync".into())
        .spawn(sync_log_writes);
    let mut drawn = false;
    let mut ds = match surface.panel_count() > 1 {
        true => DsPass::new().map_err(|e| eprintln!("slot: ds pass: {e}")).ok(),
        false => None,
    };
    frontend.set_two_panels(ds.is_some());
    let show_fps = root.join(SHOW_FPS).is_file();
    let mut ds_overlay = DsOverlayTextures::default();
    let mut fps = Fps::default();
    frontend.drive_emulator();
    let mut pacer = match surface.refresh_period() {
        Some(period) => {
            eprintln!("slot: pace: fixed at the panel's {:.3} ms", period.as_secs_f64() * 1e3);
            Pacer::fixed(period)
        }
        None => Pacer::new(),
    };
    let (mut frames, mut missed, mut early, mut stale) = (0u32, 0u32, 0u32, 0u32);
    // sound waiting in slot's queue and in the device, while a game plays: (sum, max, count)
    let (mut queued, mut device) = ((0.0f64, 0.0f64, 0u32), (0.0f64, 0.0f64, 0u32));
    let mut stages = Stages::default();
    // frames since a game frame capture was asked for (Logs/capture-frame), or 0
    let mut capture_shown = 0u32;
    // The Brick's adaptive CPU clocks, from LordNordyx's Slot-Brick (Logs/power.log; a card
    // file DISABLE_POWER_SAVING leaves the kernel's governor alone).
    let mut cpu = board
        .is_trimui()
        .then(|| slot::brick_power::CpuPower::open(&root));
    loop {
        pacer.wait();
        let began = Instant::now();
        input.steer_with_stick(frontend.app().left_stick_dpad());
        frontend.advance(&mut input);
        frontend.set_ds_touch(input.touch());
        if frontend.restarting() {
        if let Some(cpu) = cpu.as_mut() {
            cpu.restore();
        }
            frontend.restart();
        }
        if frontend.relaunching() {
        if let Some(cpu) = cpu.as_mut() {
            cpu.restore();
        }
            frontend.relaunch();
            return;
        }
        if frontend.repairing() {
            frontend.repair();
        }
        if frontend.powering_off() {
        if let Some(cpu) = cpu.as_mut() {
            cpu.restore();
        }
            frontend.poweroff();
            return;
        }
        if slot::crash::stopping() {
            eprintln!("slot: told to stop, saving first");
            frontend.stop();
            return;
        }
        if let Some(cpu) = cpu.as_mut() {
            // The in-game menu doesn't name the cart: keep the game until its core closes.
            let game = frontend.power_game();
            if game.is_some() || !frontend.has_game_core() {
                cpu.game(game.as_deref());
            }
            cpu.mode(frontend.power_mode(), frontend.audio_underruns());
        }
        // An idle shelf on the Brick: nothing to redraw, so don't (input is still read every 25 ms)
        if cpu.is_some() && !frontend.needs_render() {
            pacer = Pacer::new();
            std::thread::sleep(Duration::from_millis(25));
            continue;
        }
        let late = frontend.step_emulator(pacer.period(), STEP_TIMEOUT);
        let stepped = Instant::now();
        frontend.render(&mut compositor, window);
        if capture_shown >= 30 {
            capture_shown = 0;
            slot::frame_capture::done(&root);
            let (fw, fh) = compositor.frame_size();
            slot::frame_capture::save_rgba(
                &root.join("Logs/frame-canvas.png"),
                fw,
                fh,
                &compositor.read_frame(),
            );
            slot::frame_capture::save_rgba(
                &root.join("Logs/frame-window.png"),
                window.0,
                window.1,
                &compositor.read_window(window),
            );
            slot::frame_capture::save_fb0(&root);
        } else if frontend.app().shows_game() && frames % 60 == 0 && capture_shown == 0 {
            capture_shown = u32::from(slot::frame_capture::wanted(&root));
        } else if capture_shown > 0 {
            capture_shown += 1;
        }
        let rendered = Instant::now();
        let ds_frame = frontend.ds_frame();
        if let (Some(pass), Some(frame)) = (ds.as_mut(), ds_frame) {
            pass.set_grade(compositor.grade());
            ds_overlay.sync(pass, frontend.ds_overlay());
            pass.upload(frame);
            if frontend.ds_direct() && show_fps {
                let shown = fps.sample(frontend.frames_published(), Instant::now());
                pass.draw_fps(shown, surface.window_size());
            }
        }
        let swap = Instant::now();
        let work = swap - began;
        if let Some(cpu) = cpu.as_mut() {
            cpu.sample(!late, work, pacer.period(), frontend.audio_underruns());
        }
        if let Err(e) = surface.swap() {
            eprintln!("slot: {e}");
            frontend.stop();
            return;
        }
        let swap_took = swap.elapsed();
        let bottom_began = Instant::now();
        if let Some(pass) = ds.as_ref() {
            let power = frontend.ds_frame().map(|_| frontend.screen_power());
            let drawn = match frontend.bottom_source() {
                BottomSource::Compositor => draw_menu_below(&mut *surface, &mut frontend, &mut compositor, window),
                BottomSource::DsScreen => draw_bottom(&mut *surface, pass, power),
                BottomSource::Black => draw_bottom(&mut *surface, pass, None),
            };
            if let Err(e) = drawn {
                eprintln!("slot: bottom panel: {e}");
                ds = None;
                frontend.set_two_panels(false);
            }
        }
        let bottom_took = bottom_began.elapsed();
        slot::latency::swapped(swap_took.as_secs_f64() * 1000.0);
        let dropped = pacer.swapped(work, swap_took > Duration::from_millis(1));
        if slot::latency::tracing() {
            frames += 1;
            missed += u32::from(dropped);
            early += u32::from(swap_took > pacer.period() / 2);
            stale += u32::from(late);
            stages.add([stepped - began, rendered - stepped, swap - rendered, swap_took, bottom_took]);
            if frontend.has_game_core() {
                for (acc, ms) in [
                    (&mut queued, frontend.audio_queued_ms()),
                    (&mut device, slot::audio::device_delay_ms()),
                ] {
                    *acc = (acc.0 + ms, acc.1.max(ms), acc.2 + 1);
                }
            }
            if frames == 600 {
                let avg = |a: (f64, f64, u32)| a.0 / f64::from(a.2.max(1));
                let sound = match queued.2 {
                    0 => String::new(),
                    _ => format!(
                        ", sound queued {:.0}/{:.0} ms, in device {:.0}/{:.0} ms",
                        avg(queued),
                        queued.1,
                        avg(device),
                        device.1
                    ),
                };
                eprintln!(
                    "slot: pace: 600 frames, {missed} missed a latch, {early} drawn early, {stale} stale, work {:.1} ms, period {:.2} ms, {}, {}{sound}",
                    pacer.work().as_secs_f64() * 1e3,
                    pacer.period().as_secs_f64() * 1e3,
                    stages.report(),
                    cpu_now()
                );
                (frames, missed, early, stale) = (0, 0, 0, 0);
                (queued, device) = ((0.0, 0.0, 0), (0.0, 0.0, 0));
                stages = Stages::default();
            }
        }
        if !drawn {
            drawn = true;
            slot::boot_time::mark("first frame");
            trace_first_frame();
        }
        if let Some(left) = MIN_FRAME.checked_sub(began.elapsed()) {
            std::thread::sleep(left);
        }
    }
}

/// A card file that keeps slot's default (larger) sound buffers on the Brick.
const AUDIO_LARGE_BUFFER: &str = "Config/audio-large-buffer";

const SHOW_FPS: &str = "System/show-fps.on";
const FPS_EVERY: Duration = Duration::from_millis(500);

#[derive(Default)]
struct Fps {
    since: Option<(Instant, u64)>,
    shown: f32,
}

impl Fps {
    fn sample(&mut self, published: u64, now: Instant) -> f32 {
        match self.since {
            Some((at, from)) if now - at >= FPS_EVERY => {
                self.shown = published.saturating_sub(from) as f32 / (now - at).as_secs_f32();
                self.since = Some((now, published));
            }
            Some(_) => {}
            None => self.since = Some((now, published)),
        }
        self.shown
    }
}

#[derive(Default)]
struct Stages {
    sum: [Duration; 5],
    max: [Duration; 5],
    n: u32,
}

impl Stages {
    fn add(&mut self, took: [Duration; 5]) {
        for (i, t) in took.into_iter().enumerate() {
            self.sum[i] += t;
            self.max[i] = self.max[i].max(t);
        }
        self.n += 1;
    }

    fn report(&self) -> String {
        let names = ["emu wait", "render", "ds draw", "top swap", "bottom"];
        let n = self.n.max(1);
        names
            .iter()
            .enumerate()
            .map(|(i, name)| {
                format!(
                    "{name} {:.1}/{:.1}",
                    (self.sum[i] / n).as_secs_f64() * 1e3,
                    self.max[i].as_secs_f64() * 1e3
                )
            })
            .collect::<Vec<_>>()
            .join(", ")
    }
}

#[derive(Default)]
struct DsOverlayTextures {
    serial: u64,
    textures: Vec<u32>,
}

impl DsOverlayTextures {
    fn sync(&mut self, pass: &mut DsPass, decoded: Option<&slot::overlay::DecodedOverlay>) {
        let Some(decoded) = decoded.filter(|d| d.serial != self.serial) else {
            return;
        };
        self.serial = decoded.serial;
        for tex in self.textures.drain(..) {
            pass.delete_overlay_texture(tex);
        }
        let mut upload = |image: &Option<slot::overlay::Rgba>| {
            image.as_ref().map(|(w, h, rgba)| {
                let tex = pass.create_overlay_texture(*w, *h, rgba);
                self.textures.push(tex);
                (tex, *w, *h)
            })
        };
        let tile = upload(&decoded.tile);
        let both = upload(&decoded.both).map(|(tex, _, _)| tex);
        let top = upload(&decoded.top).map(|(tex, _, _)| tex).or(both);
        let bottom = upload(&decoded.bottom).map(|(tex, _, _)| tex).or(both);
        let shown = tile.is_some() || top.is_some() || bottom.is_some();
        pass.set_overlay(shown.then_some(slot_gfx::DsOverlay { tile, top, bottom }));
    }
}

fn draw_menu_below(
    surface: &mut dyn Surface,
    frontend: &mut Frontend,
    compositor: &mut Compositor,
    window: (u32, u32),
) -> Result<(), slot_gfx::GfxError> {
    surface.select_panel(1)?;
    frontend.render_screen(compositor, window, Screen::Bottom);
    let swapped = surface.swap();
    surface.select_panel(0)?;
    swapped
}

fn draw_bottom(surface: &mut dyn Surface, pass: &DsPass, power: Option<f32>) -> Result<(), slot_gfx::GfxError> {
    surface.select_panel(1)?;
    let panel = surface.window_size();
    match power {
        Some(t) => pass.draw_screen_powered(DsScreen::Bottom, panel, t),
        None => pass.clear(panel),
    }
    let swapped = surface.swap();
    surface.select_panel(0)?;
    swapped
}

fn sync_log_writes() {
    let mut synced = 0;
    loop {
        std::thread::sleep(LOG_SYNC);
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe { libc::fstat(2, &mut st) } != 0 {
            return;
        }
        if st.st_size != synced {
            #[cfg(target_os = "linux")]
            unsafe {
                libc::syncfs(2);
            }
            synced = st.st_size;
        }
    }
}
