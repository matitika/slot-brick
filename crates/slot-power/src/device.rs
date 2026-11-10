#[cfg(unix)]
use std::ffi::{c_int, c_ulong, c_void};
use std::fs;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::{Battery, Charge, LedState, Platform, RepairResult};

const TOP_STEP: u32 = 10;

const BACKLIGHT_FLOOR: f64 = 8.0 / 255.0;

const BACKLIGHT_CURVE: f64 = 1.75;

const DEV_INPUT: &str = "/dev/input";

const RUN_DIR: &str = "/run";

const CARD_LOG: &str = "/data/slot-card.log";

const REPAIR_SCRIPT: &str = "repair.sh";

const FSCK_EXFAT: &str = "fsck.exfat";

const REPAIR_STAGE: &str = "/tmp/slot-repair";

const REPAIR_RESULT: &str = "/data/slot-repair.result";

const TIMEDATECTL: &str = "/usr/bin/timedatectl";

const WIFI_MODULE: &str = "8821cs";

const WIFI_UNLOAD_WAIT: Duration = Duration::from_secs(5);

const KERNEL_LINES: usize = 40;

pub fn uptime_seconds(s: &str) -> Option<String> {
    let first = s.split_whitespace().next()?;
    if !first.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return None;
    }
    if !first.chars().any(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(first.to_string())
}

pub fn record_first_frame(run: &Path, uptime: &str) {
    let marker = run.join("boot-first-frame");
    let again = marker.exists();
    if !again {
        let _ = fs::write(&marker, format!("{uptime}\n"));
    }
    let label = if again {
        "first-frame-again"
    } else {
        "first-frame"
    };
    if let Ok(mut f) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(run.join("boot-trace"))
    {
        let _ = writeln!(f, "{uptime} {label}");
    }
}

pub fn trace_first_frame() {
    let Ok(raw) = fs::read_to_string("/proc/uptime") else {
        return;
    };
    let Some(up) = uptime_seconds(&raw) else {
        return;
    };
    record_first_frame(Path::new(RUN_DIR), &up);
}

const EV_FF: u16 = 0x15;
const FF_RUMBLE: u16 = 0x50;
#[cfg(unix)]
const EVIOCSFF: c_ulong = 0x4030_4580;

const DISPDBG_MAX: u32 = 255;

const DISP_MAX: u32 = 255;
/// As NextUI does on the Brick: a panel brought back from 0 goes through this
/// level first, or it can stay dark.
const DISP_WAKE: u32 = 8;
const DISP_LCD_SET_BRIGHTNESS: c_ulong = 0x102;

const TRIMUI_MOTOR_GPIO: &str = "class/gpio/gpio227/value";
const TRIMUI_MOTOR_VOLTAGE: &str = "class/motor/voltage";
const TRIMUI_LED_ANIM: &str = "class/led_anim";
const TRIMUI_SLEEP: &str = "System/brick-sleep.sh";

pub const BOARD_VAR: &str = "SLOT_DEVICE";

#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum Board {
    #[default]
    Anbernic,
    TrimUiBrick,
    TrimUiBrickPro,
    TrimUiSmartPro,
}

impl Board {
    pub fn detect() -> Board {
        let device = std::env::var(BOARD_VAR).ok();
        let cpuinfo = fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
        Board::from_hints(device.as_deref(), &cpuinfo)
    }

    pub fn from_hints(device: Option<&str>, cpuinfo: &str) -> Board {
        let named = device.map(|d| d.trim().to_ascii_lowercase());
        match named.as_deref() {
            Some("brickpro" | "brick-pro" | "tg4040") => return Board::TrimUiBrickPro,
            Some("brick" | "tg3040") => return Board::TrimUiBrick,
            Some("smartpro" | "smart-pro" | "tg5040") => return Board::TrimUiSmartPro,
            Some("anbernic" | "h700") => return Board::Anbernic,
            _ => {}
        }
        if cpuinfo.contains("TG4040") {
            Board::TrimUiBrickPro
        } else if cpuinfo.contains("TG3040") {
            Board::TrimUiBrick
        } else if cpuinfo.contains("TG5040") {
            Board::TrimUiSmartPro
        } else {
            Board::Anbernic
        }
    }

    pub fn is_trimui(self) -> bool {
        self != Board::Anbernic
    }

    fn motor_volts(self) -> u32 {
        match self {
            Board::TrimUiBrickPro => 2_500_000,
            _ => 3_300_000,
        }
    }
}

enum Backlight {
    Class(Vec<(PathBuf, u32)>),
    Dispdbg(PathBuf),
    Disp(PathBuf),
}

enum Led {
    Multi(PathBuf),
    Mono { dir: PathBuf, max: u32 },
    Anim(PathBuf),
}

pub struct DevicePlatform {
    root: PathBuf,
    sysfs: PathBuf,
    backlight: Option<Backlight>,
    max_brightness: u32,
    battery: Option<PathBuf>,
    rtc: Option<PathBuf>,
    motor: Option<Motor>,
    led: Option<Led>,
    sleep_command: Option<PathBuf>,
    sleeping: Option<Child>,
    board: Board,
    disp_dark: bool,
}

impl DevicePlatform {
    pub fn new(root: PathBuf) -> Self {
        let platform = DevicePlatform::probe_board(
            Path::new("/sys"),
            Path::new("/dev"),
            root,
            Board::detect(),
        );
        match std::env::var_os("DSOS_SLEEP") {
            Some(cmd) if !cmd.is_empty() => platform.with_sleep(PathBuf::from(cmd)),
            _ => platform,
        }
    }

    pub fn with_sleep(mut self, command: PathBuf) -> Self {
        self.sleep_command = Some(command);
        self
    }

    pub fn probe(sysfs: &Path, root: PathBuf) -> Self {
        DevicePlatform::probe_board(sysfs, Path::new("/dev"), root, Board::Anbernic)
    }

    pub fn probe_board(sysfs: &Path, dev: &Path, root: PathBuf, board: Board) -> Self {
        if board.is_trimui() {
            return DevicePlatform::probe_trimui(sysfs, dev, root, board);
        }
        let class = class_backlights(sysfs);
        let max_brightness = class.first().map_or(DISPDBG_MAX, |(_, max)| *max);
        let backlight = match class.is_empty() {
            false => Some(Backlight::Class(class)),
            true => {
                let dbg = sysfs.join("kernel/debug/dispdbg");
                dbg.join("param")
                    .is_file()
                    .then_some(Backlight::Dispdbg(dbg))
            }
        };
        let battery = first_dir(&sysfs.join("class/power_supply"), is_battery);
        let rtc = first_dir(&sysfs.join("class/rtc"), |_| true);
        let motor = Motor::open(sysfs);
        let led = first_dir(&sysfs.join("class/leds"), |d| {
            let charging = d
                .file_name()
                .is_some_and(|n| n.to_string_lossy().contains("charging"));
            !charging && (d.join("multi_intensity").is_file() || d.join("brightness").is_file())
        })
        .map(|dir| {
            if dir.join("multi_intensity").is_file() {
                Led::Multi(dir)
            } else {
                let max = read_number(&dir.join("max_brightness")).unwrap_or(255);
                Led::Mono { dir, max }
            }
        });
        DevicePlatform {
            root,
            sysfs: sysfs.to_path_buf(),
            backlight,
            max_brightness,
            battery,
            rtc,
            motor,
            led,
            sleep_command: None,
            sleeping: None,
            board,
            disp_dark: false,
        }
    }

    fn probe_trimui(sysfs: &Path, dev: &Path, root: PathBuf, board: Board) -> Self {
        let disp = dev.join("disp");
        let class = class_backlights(sysfs);
        let (backlight, max_brightness) = match (disp.exists(), class.first()) {
            (true, _) => (Some(Backlight::Disp(disp)), DISP_MAX),
            (false, Some(&(_, max))) => (Some(Backlight::Class(class)), max),
            (false, None) => (None, DISP_MAX),
        };
        let motor = Motor::open(sysfs).or_else(|| Motor::gpio(sysfs, board));
        let anim = sysfs.join(TRIMUI_LED_ANIM);
        let led = anim.join("max_scale").is_file().then_some(Led::Anim(anim));
        // suspend-to-RAM while dozing, on stock and on Slot OS
        let sleep = root.join(TRIMUI_SLEEP);
        let sleep_command = sleep.is_file().then_some(sleep);
        DevicePlatform {
            root,
            sysfs: sysfs.to_path_buf(),
            backlight,
            max_brightness,
            battery: first_dir(&sysfs.join("class/power_supply"), is_battery),
            rtc: first_dir(&sysfs.join("class/rtc"), |_| true),
            motor,
            led,
            sleep_command,
            sleeping: None,
            board,
            disp_dark: false,
        }
    }

    /// On Slot OS init powers off or restarts once slot has ended: it stops what slot started
    /// (Wi-Fi) and closes the card first.
    fn hand_to_init(&self, what: &str, code: i32) -> ! {
        self.breadcrumb(&format!("{what}: handing to Slot OS init ({code})"));
        let _ = Command::new("sync").status();
        std::process::exit(code)
    }

    pub fn board(&self) -> Board {
        self.board
    }

    fn breadcrumb(&self, line: &str) {
        use std::io::Write;
        let dir = self.root.join("Logs");
        let _ = fs::create_dir_all(&dir);
        if let Ok(mut f) = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("ags-shutdown-trace.log"))
        {
            let _ = writeln!(f, "{} {line}", self.now());
            let _ = f.sync_all();
        }
    }

    pub fn trace_boot(&self) {
        self.breadcrumb("boot: platform up, trace working");
    }

    pub fn report(&self) -> String {
        let leaf = |p: &Path| {
            p.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.display().to_string())
        };
        let name = |p: &Option<PathBuf>, absent: &str| match p {
            Some(p) => leaf(p),
            None => absent.to_string(),
        };
        let backlight = match &self.backlight {
            Some(Backlight::Class(dirs)) => dirs
                .iter()
                .map(|(dir, _)| leaf(dir))
                .collect::<Vec<_>>()
                .join(" and "),
            Some(Backlight::Dispdbg(_)) => "dispdbg".to_string(),
            Some(Backlight::Disp(_)) => "disp".to_string(),
            None => "no backlight".to_string(),
        };
        let motor = match &self.motor {
            Some(m) => format!("motor {}", m.name),
            None => "no motor".to_string(),
        };
        format!(
            "{:?}: backlight {} (0 to {}), battery {}, {}, {motor}",
            self.board,
            backlight,
            self.max_brightness,
            name(&self.battery, "no battery"),
            name(&self.rtc, "no rtc"),
        )
    }
}

fn ran(what: &str, result: std::io::Result<std::process::ExitStatus>) {
    match result {
        Ok(status) if status.success() => eprintln!("slot: {what} ok"),
        Ok(status) => eprintln!("slot: {what} {status}"),
        Err(e) => eprintln!("slot: {what}: {e}"),
    }
}

pub fn has_bit(mask: &str, bit: u16) -> bool {
    let words: Vec<&str> = mask.split_whitespace().collect();
    let from_end = usize::from(bit) / 64;
    let Some(word) = words.len().checked_sub(from_end + 1).map(|i| words[i]) else {
        return false;
    };
    u64::from_str_radix(word, 16).is_ok_and(|w| w >> (bit % 64) & 1 == 1)
}

pub fn rumble_node(sysfs: &Path) -> Option<String> {
    let dir = first_dir(&sysfs.join("class/input"), |node| {
        fs::read_to_string(node.join("device/capabilities/ff"))
            .is_ok_and(|ff| has_bit(&ff, FF_RUMBLE))
    })?;
    Some(dir.file_name()?.to_string_lossy().into_owned())
}

pub fn motor_change(strength: u16, running: bool) -> Option<bool> {
    match (strength > 0, running) {
        (true, false) => Some(true),
        (false, true) => Some(false),
        _ => None,
    }
}

struct Motor {
    drive: Drive,
    name: String,
    running: bool,
}

enum Drive {
    Ff {
        node: fs::File,
        id: i16,
    },
    Gpio {
        value: PathBuf,
        voltage: PathBuf,
        volts: u32,
    },
}

#[cfg(unix)]
#[repr(C)]
#[derive(Default)]
struct FfTrigger {
    button: u16,
    interval: u16,
}

#[cfg(unix)]
#[repr(C)]
#[derive(Default)]
struct FfReplay {
    length: u16,
    delay: u16,
}

#[cfg(unix)]
#[repr(C)]
#[derive(Default)]
struct FfRumble {
    strong: u16,
    weak: u16,
}

#[cfg(unix)]
#[repr(C)]
struct FfEffect {
    kind: u16,
    id: i16,
    direction: u16,
    trigger: FfTrigger,
    replay: FfReplay,
    _align: u16,
    rumble: FfRumble,
    _tail: [u8; 28],
}

#[repr(C)]
struct FfEvent {
    sec: i64,
    usec: i64,
    kind: u16,
    code: u16,
    value: i32,
}

#[cfg(unix)]
extern "C" {
    fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
    fn fcntl(fd: c_int, cmd: c_int, ...) -> c_int;
    fn dup2(old: c_int, new: c_int) -> c_int;
}

const F_GETFD: c_int = 1;
const F_SETFD: c_int = 2;
const FD_CLOEXEC: c_int = 1;

fn unload_wifi() {
    let Ok(mut rmmod) = Command::new("rmmod").arg(WIFI_MODULE).spawn() else {
        return;
    };
    let until = Instant::now() + WIFI_UNLOAD_WAIT;
    while Instant::now() < until {
        if !matches!(rmmod.try_wait(), Ok(None)) {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn release_logs() {
    #[cfg(unix)]
    if let Ok(null) = fs::OpenOptions::new().write(true).open("/dev/null") {
        unsafe {
            dup2(null.as_raw_fd(), 1);
            dup2(null.as_raw_fd(), 2);
        }
    }
}

fn close_on_exec() {
    let Ok(fds) = fs::read_dir("/proc/self/fd") else {
        return;
    };
    let fds: Vec<c_int> = fds
        .filter_map(|e| e.ok()?.file_name().to_str()?.parse().ok())
        .filter(|fd| *fd > 2)
        .collect();
    for fd in fds {
        unsafe {
            let flags = fcntl(fd, F_GETFD);
            if flags >= 0 {
                fcntl(fd, F_SETFD, flags | FD_CLOEXEC);
            }
        }
    }
}

fn upload(node: &fs::File, id: i16) -> Option<i16> {
    #[cfg(not(unix))]
    {
        let _ = (node, id);
        None
    }
    #[cfg(unix)]
    {
        let mut effect = FfEffect {
            kind: FF_RUMBLE,
            id,
            direction: 0,
            trigger: FfTrigger::default(),
            replay: FfReplay::default(),
            _align: 0,
            rumble: FfRumble {
                strong: u16::MAX,
                weak: u16::MAX,
            },
            _tail: [0; 28],
        };
        let rc = unsafe {
            ioctl(
                node.as_raw_fd(),
                EVIOCSFF,
                &mut effect as *mut FfEffect as *mut c_void,
            )
        };
        (rc >= 0).then_some(effect.id)
    }
}

impl Motor {
    fn open(sysfs: &Path) -> Option<Motor> {
        let name = rumble_node(sysfs)?;
        let node = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(Path::new(DEV_INPUT).join(&name))
            .map_err(|e| eprintln!("slot: rumble {name}: {e}"))
            .ok()?;
        let Some(id) = upload(&node, -1) else {
            eprintln!("slot: rumble {name}: {}", std::io::Error::last_os_error());
            return None;
        };
        Some(Motor {
            drive: Drive::Ff { node, id },
            name,
            running: false,
        })
    }

    fn gpio(sysfs: &Path, board: Board) -> Option<Motor> {
        let value = sysfs.join(TRIMUI_MOTOR_GPIO);
        if !value.is_file() {
            return None;
        }
        Some(Motor {
            drive: Drive::Gpio {
                value,
                voltage: sysfs.join(TRIMUI_MOTOR_VOLTAGE),
                volts: board.motor_volts(),
            },
            name: "gpio227".to_string(),
            running: false,
        })
    }

    fn play(&mut self, on: bool) {
        match &mut self.drive {
            Drive::Ff { node, id } => {
                if on {
                    match upload(node, *id).or_else(|| upload(node, -1)) {
                        Some(new) => *id = new,
                        None => eprintln!(
                            "slot: rumble {}: {}",
                            self.name,
                            std::io::Error::last_os_error()
                        ),
                    }
                }
                let ev = FfEvent {
                    sec: 0,
                    usec: 0,
                    kind: EV_FF,
                    code: *id as u16,
                    value: i32::from(on),
                };
                let bytes = unsafe {
                    std::slice::from_raw_parts(
                        &ev as *const FfEvent as *const u8,
                        size_of::<FfEvent>(),
                    )
                };
                match (&*node).write_all(bytes) {
                    Ok(()) => self.running = on,
                    Err(e) => eprintln!("slot: rumble: {e}"),
                }
            }
            Drive::Gpio {
                value,
                voltage,
                volts,
            } => {
                if on && voltage.is_file() {
                    let _ = fs::write(&*voltage, volts.to_string());
                }
                match fs::write(&*value, if on { "1" } else { "0" }) {
                    Ok(()) => self.running = on,
                    Err(e) => eprintln!("slot: rumble {}: {e}", value.display()),
                }
            }
        }
    }
}

impl Drop for Motor {
    fn drop(&mut self) {
        if self.running {
            self.play(false);
        }
    }
}

const ANIM_STATIC: u32 = 4;
const ANIM_SCALE: u32 = 30;

#[cfg(not(unix))]
fn disp_brightness(_node: &Path, _value: u32) -> std::io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn disp_brightness(node: &Path, value: u32) -> std::io::Result<()> {
    let disp = fs::OpenOptions::new().read(true).write(true).open(node)?;
    let mut param: [c_ulong; 4] = [0, c_ulong::from(value), 0, 0];
    let rc = unsafe {
        ioctl(
            disp.as_raw_fd(),
            DISP_LCD_SET_BRIGHTNESS,
            param.as_mut_ptr() as *mut c_void,
        )
    };
    if rc < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

fn all_dirs(parent: &Path) -> Vec<PathBuf> {
    let mut names: Vec<PathBuf> = fs::read_dir(parent)
        .map(|d| d.filter_map(|e| e.ok()).map(|e| e.path()).collect())
        .unwrap_or_default();
    names.sort();
    names
}

fn first_dir(parent: &Path, keep: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    all_dirs(parent).into_iter().find(|p| keep(p))
}

fn class_backlights(sysfs: &Path) -> Vec<(PathBuf, u32)> {
    all_dirs(&sysfs.join("class/backlight"))
        .into_iter()
        .filter(|d| d.join("brightness").is_file())
        .map(|d| {
            let max = read_number(&d.join("max_brightness")).unwrap_or(DISPDBG_MAX);
            (d, max)
        })
        .collect()
}

fn is_battery(dir: &Path) -> bool {
    fs::read_to_string(dir.join("type")).is_ok_and(|t| t.trim() == "Battery")
        && dir.join("capacity").is_file()
}

fn read_number(path: &Path) -> Option<u32> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn charge_at(dir: &Path) -> Charge {
    let Ok(text) = fs::read_to_string(dir.join("status")) else {
        return Charge::Unknown;
    };
    match text.trim() {
        "Charging" => Charge::Charging,
        "Discharging" => Charge::Discharging,
        "Full" => Charge::Full,
        _ => Charge::Unknown,
    }
}

fn headphones_in(gpio: &str) -> bool {
    gpio.lines()
        .find(|l| l.contains("|Headphone detection"))
        .and_then(|l| l.rsplit_once(')'))
        .is_some_and(|(_, state)| state.split_whitespace().nth(1) == Some("hi"))
}

fn backlight_value(step: u8, max: u32) -> u32 {
    let step = u32::from(step).min(TOP_STEP);
    if step == 0 {
        return 0;
    }
    let t = f64::from(step - 1) / f64::from(TOP_STEP - 1);
    let floor = (f64::from(max) * BACKLIGHT_FLOOR).round().max(1.0);
    let span = f64::from(max) - floor;
    (floor + span * t.powf(BACKLIGHT_CURVE)).round() as u32
}

impl Platform for DevicePlatform {
    fn sleep(&mut self) {
        let Some(command) = &self.sleep_command else {
            return;
        };
        if let Some(child) = &mut self.sleeping {
            if matches!(child.try_wait(), Ok(None)) {
                return;
            }
        }
        // a script on the card runs through sh: FAT and exFAT may not keep its execute bit
        let spawned = match command.extension().is_some_and(|e| e == "sh") {
            true => Command::new("/bin/sh")
                .arg(command)
                .env("SLOT_ROOT", &self.root)
                .spawn(),
            false => Command::new(command).spawn(),
        };
        match spawned {
            Ok(child) => self.sleeping = Some(child),
            Err(e) => eprintln!("slot: sleep {}: {e}", command.display()),
        }
    }

    fn headphones(&self) -> bool {
        fs::read_to_string(self.sysfs.join("kernel/debug/gpio")).is_ok_and(|g| headphones_in(&g))
    }

    fn card_read_only(&self) -> bool {
        read_only(&self.root)
    }

    fn card_log(&self) -> Option<PathBuf> {
        Some(PathBuf::from(CARD_LOG))
    }

    fn card_kernel_lines(&self) -> Vec<String> {
        Command::new("dmesg")
            .output()
            .map(|o| card_kernel_lines(&String::from_utf8_lossy(&o.stdout)))
            .unwrap_or_default()
    }

    fn set_backlight(&mut self, step: u8) {
        let Some(backlight) = &self.backlight else {
            return;
        };
        let value = backlight_value(step, self.max_brightness);
        let was_dark = self.disp_dark;
        let write = |file: PathBuf, body: String| {
            if let Err(e) = fs::write(&file, body) {
                eprintln!("slot: {}: {e}", file.display());
            }
        };
        match backlight {
            Backlight::Class(dirs) => {
                for (dir, max) in dirs {
                    write(
                        dir.join("brightness"),
                        backlight_value(step, *max).to_string(),
                    );
                }
            }
            Backlight::Dispdbg(dir) => {
                let value = backlight_value(step, self.max_brightness);
                write(dir.join("name"), "lcd0".to_string());
                write(dir.join("command"), "setbl".to_string());
                write(dir.join("param"), value.to_string());
                write(dir.join("start"), "1".to_string());
            }
            Backlight::Disp(node) => {
                if was_dark && value > 0 {
                    let _ = disp_brightness(node, DISP_WAKE);
                }
                if let Err(e) = disp_brightness(node, value) {
                    eprintln!("slot: {}: {e}", node.display());
                }
            }
        }
        self.disp_dark = value == 0;
    }

    fn battery(&self) -> Option<Battery> {
        let dir = self.battery.as_ref()?;
        let percent = read_number(&dir.join("capacity"))?;
        Some(Battery {
            percent: percent.min(100) as u8,
            charge: charge_at(dir),
        })
    }

    fn charge(&self) -> Charge {
        self.battery
            .as_ref()
            .map_or(Charge::Unknown, |dir| charge_at(dir))
    }

    fn set_led(&mut self, state: LedState) {
        let Some(led) = &self.led else {
            return;
        };
        match led {
            Led::Multi(dir) => {
                let (r, g, b) = match state {
                    LedState::Off => (0, 0, 0),
                    LedState::Running | LedState::Charged => (0, 255, 0),
                    LedState::Low => (255, 0, 0),
                    LedState::Charging => (255, 140, 0),
                };
                let _ = fs::write(dir.join("multi_intensity"), format!("{r} {g} {b}\n"));
                let _ = fs::write(dir.join("brightness"), "255\n");
            }
            Led::Mono { dir, max } => {
                let on = !matches!(state, LedState::Off | LedState::Low);
                let value = if on { *max } else { 0 };
                let _ = fs::write(dir.join("brightness"), format!("{value}\n"));
            }
            Led::Anim(dir) => {
                let rgb = match state {
                    LedState::Off | LedState::Running => None,
                    LedState::Low => Some("FF0000"),
                    LedState::Charging => Some("FF8C00"),
                    LedState::Charged => Some("00FF00"),
                };
                let Some(rgb) = rgb else {
                    let _ = fs::write(dir.join("max_scale"), "0\n");
                    return;
                };
                let _ = fs::write(dir.join("effect_rgb_hex_m"), format!("{rgb}\n"));
                let _ = fs::write(dir.join("effect_duration_m"), "1000\n");
                let _ = fs::write(dir.join("effect_m"), format!("{ANIM_STATIC}\n"));
                let _ = fs::write(dir.join("max_scale"), format!("{ANIM_SCALE}\n"));
            }
        }
    }

    fn restart(&mut self) -> ! {
        if crate::slot_os() {
            self.hand_to_init("restart", crate::SLOT_OS_RESTART);
        }
        self.breadcrumb("restart: reached slot, syncing before the WiFi driver unloads");
        let _ = Command::new("sync").status();
        unload_wifi();
        self.breadcrumb("restart: WiFi driver unloaded or given up on, about to sync");
        let _ = Command::new("sync").status();
        self.breadcrumb("restart: sync returned, about to signal init");
        release_logs();
        let _ = Command::new("reboot").status();
        std::thread::sleep(Duration::from_secs(10));
        std::process::exit(0)
    }

    fn repair_card(&mut self) -> ! {
        // Slot OS's init checks the card at every start it finds it left in use
        if crate::slot_os() {
            self.hand_to_init("repair", crate::SLOT_OS_RESTART);
        }
        let mount = fs::read_to_string("/proc/mounts")
            .ok()
            .and_then(|m| card_mount_in(&m, &self.root));
        match (mount, stage_repair(&self.root, Path::new(REPAIR_STAGE))) {
            (Some(m), Ok(script)) => {
                close_on_exec();
                #[cfg(unix)]
                {
                    use std::os::unix::process::CommandExt;
                    let e = Command::new("/bin/sh")
                        .arg(&script)
                        .arg(&m.device)
                        .arg(&m.point)
                        .arg(&m.fs)
                        .exec();
                    eprintln!("slot: repair: {e}");
                }
            }
            (None, _) => eprintln!(
                "slot: repair: no block device under {}",
                self.root.display()
            ),
            (_, Err(e)) => eprintln!("slot: repair: {e}"),
        }
        self.restart()
    }

    fn take_repair_result(&mut self) -> Option<RepairResult> {
        take_repair_result(Path::new(REPAIR_RESULT))
    }

    fn poweroff(&mut self) -> ! {
        if crate::slot_os() {
            self.hand_to_init("poweroff", crate::SLOT_OS_POWEROFF);
        }
        self.breadcrumb("poweroff: reached slot, syncing before the WiFi driver unloads");
        let _ = Command::new("sync").status();
        unload_wifi();
        self.breadcrumb("poweroff: WiFi driver unloaded or given up on, about to sync");
        let _ = Command::new("sync").status();
        self.breadcrumb("poweroff: sync returned, about to signal init");
        release_logs();
        let _ = Command::new("poweroff").status();
        std::thread::sleep(Duration::from_secs(10));
        std::process::exit(0)
    }

    fn root(&self) -> &Path {
        &self.root
    }

    fn now(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }

    fn set_clock(&mut self, secs: i64) {
        write_clock(secs);
    }

    fn set_network_time(&mut self, on: bool) {
        std::thread::spawn(move || set_ntp(on));
    }

    fn relink_adb(&mut self) -> bool {
        let gadget = self.sysfs.join("kernel/config/usb_gadget/g1/UDC");
        if !gadget.is_file() {
            return false;
        }
        let Some(udc) = first_dir(&self.sysfs.join("class/udc"), |_| true) else {
            return false;
        };
        let Some(name) = udc.file_name().map(|n| n.to_string_lossy().into_owned()) else {
            return false;
        };
        let _ = fs::write(&gadget, "\n");
        match fs::write(&gadget, &name) {
            Ok(()) => true,
            Err(e) => {
                eprintln!("slot: adb {}: {e}", gadget.display());
                false
            }
        }
    }

    fn set_rumble(&mut self, strength: u16) {
        let Some(motor) = &mut self.motor else {
            return;
        };
        if let Some(on) = motor_change(strength, motor.running) {
            motor.play(on);
        }
    }
}

pub fn read_only(path: &Path) -> bool {
    fs::read_to_string("/proc/mounts").is_ok_and(|m| read_only_in(&m, path))
}

pub fn card_kernel_lines(dmesg: &str) -> Vec<String> {
    let lines: Vec<String> = dmesg
        .lines()
        .filter(|l| {
            let l = l.to_ascii_lowercase();
            l.contains("fat-fs")
                || l.contains("exfat")
                || l.contains("remount")
                || l.contains("i/o error")
                || (l.contains("mmcblk") && (l.contains("error") || l.contains("fail")))
        })
        .map(str::to_owned)
        .collect();
    let skip = lines.len().saturating_sub(KERNEL_LINES);
    lines.into_iter().skip(skip).collect()
}

struct MountLine<'a> {
    device: &'a str,
    point: &'a Path,
    fs: &'a str,
    options: &'a str,
}

fn closest_mount<'a>(mounts: &'a str, path: &Path) -> Option<MountLine<'a>> {
    mounts
        .lines()
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            let device = f.next()?;
            let point = Path::new(f.next()?);
            let fs = f.next()?;
            let options = f.next()?;
            path.starts_with(point).then_some(MountLine {
                device,
                point,
                fs,
                options,
            })
        })
        .max_by_key(|m| m.point.as_os_str().len())
}

pub fn read_only_in(mounts: &str, path: &Path) -> bool {
    closest_mount(mounts, path).is_some_and(|m| m.options.split(',').any(|o| o == "ro"))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CardMount {
    pub device: String,
    pub point: PathBuf,
    pub fs: String,
}

pub fn card_mount_in(mounts: &str, path: &Path) -> Option<CardMount> {
    let m = closest_mount(mounts, path)?;
    m.device.starts_with("/dev/").then(|| CardMount {
        device: m.device.to_owned(),
        point: m.point.to_path_buf(),
        fs: m.fs.to_owned(),
    })
}

pub fn stage_repair(root: &Path, stage: &Path) -> std::io::Result<PathBuf> {
    let system = root.join("System");
    fs::create_dir_all(stage)?;
    let script = stage.join(REPAIR_SCRIPT);
    fs::copy(system.join(REPAIR_SCRIPT), &script)?;
    let fsck = system.join(FSCK_EXFAT);
    if fsck.exists() {
        let staged = stage.join(FSCK_EXFAT);
        fs::copy(&fsck, &staged)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&staged, fs::Permissions::from_mode(0o755))?;
        }
    }
    Ok(script)
}

pub fn take_repair_result(file: &Path) -> Option<RepairResult> {
    let text = fs::read_to_string(file).ok()?;
    let _ = fs::remove_file(file);
    Some(match text.trim() {
        "repaired" => RepairResult::Repaired,
        "clean" => RepairResult::Clean,
        _ => RepairResult::Failed,
    })
}

fn set_ntp(on: bool) {
    let want = if on { "yes" } else { "no" };
    let now = Command::new(TIMEDATECTL)
        .args(["show", "-p", "NTP", "--value"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
    if now.as_deref() == Some(want) {
        return;
    }
    ran(
        "timedatectl set-ntp",
        Command::new(TIMEDATECTL).args(["set-ntp", want]).status(),
    );
}

fn write_clock(secs: i64) {
    ran(
        "date",
        Command::new("date").arg(format!("-s@{secs}")).status(),
    );
    ran(
        "hwclock",
        Command::new("hwclock").args(["-w", "-u"]).status(),
    );
}
