// Adapted from LordNordyx's Slot-Brick (https://github.com/LordNordyx/Slot-Brick), GPL-3.0:
// adaptive CPU clocks for the TrimUI Brick, so mGBA's frames and audio keep up.
//! Conservative runtime CPU ceilings. No voltage, PLL or firmware changes.
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Game,
    Menu,
    Standby,
    Boost,
}
// The least a game is given on the Brick: traces of Kirby show mGBA falling behind at 1.0 GHz
// and keeping up from 1.2 GHz.
const GAME_MIN_KHZ: u32 = 1_200_000;
// A game played this long without trouble is tried one step lower next time.
const PROBE_DOWN_AFTER: u32 = 600; // half-second windows: 5 minutes
                                   // `at` is the ceiling (scaling_max_freq). In games `low` is also held as
                                   // scaling_min_freq: schedutil reads the frame-paced emulator as idle and parks
                                   // at 816 MHz, so a ceiling alone cannot stop audio underruns.
                                   // `learned` and `floor` are what this game needed last time (Config/cpu-levels.txt): the held
                                   // minimum never steps below `learned`, the ceiling never below `floor`.
struct Policy {
    levels: Vec<u32>,
    at: usize,
    floor: usize,
    clean: u32,
    mode: Mode,
    base: usize,
    low: usize,
    steady: u32,
    learned: usize,
    trouble: bool,
    windows: u32,
    last_bad: bool,
}
impl Policy {
    fn new(levels: Vec<u32>) -> Self {
        let at = levels.len() - 1;
        let floor = levels.iter().position(|&v| v >= GAME_MIN_KHZ).unwrap_or(at);
        Self {
            levels,
            at,
            floor,
            clean: 0,
            mode: Mode::Boost,
            base: floor,
            low: 0,
            steady: 0,
            learned: floor,
            trouble: false,
            windows: 0,
            last_bad: false,
        }
    }
    fn index_of(&self, khz: u32) -> usize {
        self.levels
            .iter()
            .position(|&v| v >= khz)
            .unwrap_or(self.levels.len() - 1)
            .max(self.base)
    }
    /// Start a game from what it needed last time, or from the defaults.
    fn load(&mut self, remembered: Option<(u32, u32)>) {
        let (min, ceiling) = remembered.map_or((self.base, self.base), |(m, c)| {
            (self.index_of(m), self.index_of(c))
        });
        self.learned = min;
        self.floor = ceiling.max(min);
        self.trouble = false;
        self.windows = 0;
        if self.mode == Mode::Game {
            self.low = self.learned.min(self.at);
        }
    }
    /// What to remember for the game: its held minimum and ceiling floor, in kHz. A long
    /// session without trouble lets both try one step lower next time.
    fn remember(&self) -> (u32, u32) {
        let down = usize::from(!self.trouble && self.windows >= PROBE_DOWN_AFTER);
        let min = self.learned.saturating_sub(down).max(self.base);
        let ceiling = self.floor.saturating_sub(down).max(min);
        (self.levels[min], self.levels[ceiling])
    }
    fn change_mode(&mut self, mode: Mode) -> u32 {
        if self.mode != mode {
            self.clean = 0;
            self.at = match mode {
                Mode::Menu => self
                    .levels
                    .iter()
                    .position(|&v| v >= 816_000)
                    .unwrap_or(self.levels.len() - 1),
                Mode::Standby => 0,
                _ => self.levels.len() - 1,
            };
            self.low = if mode == Mode::Game {
                self.learned.min(self.at)
            } else {
                0
            };
            self.steady = 0;
            self.mode = mode;
        }
        self.levels[self.at]
    }
    // Called per half-second window, never per rendered frame.
    fn sample(&mut self, bad: bool, spare: bool) -> u32 {
        if self.mode != Mode::Game {
            return self.levels[self.at];
        }
        self.windows = self.windows.saturating_add(1);
        if bad {
            self.trouble = true;
            // Late even at what it needed before: it needs more, this time and next. Once per
            // spell of trouble, so one long hiccup doesn't raise it twice.
            if self.low == self.learned && !self.last_bad {
                self.learned = (self.learned + 1).min(self.levels.len() - 1);
            }
            if self.at + 1 < self.levels.len() {
                self.floor = self.floor.max(self.at + 1);
            }
            self.at = (self.at + 2).min(self.levels.len() - 1);
            self.clean = 0;
            self.low = (self.low + 1).min(self.at);
            self.steady = 0;
        } else if spare {
            self.clean += 1;
            if self.clean >= 20 {
                self.at = self.at.saturating_sub(1).max(self.floor).max(self.learned);
                self.clean = 0;
            }
        } else {
            self.clean = 0;
        }
        if !bad {
            // A minute without trouble lets the held minimum step back down.
            self.steady += 1;
            if self.steady >= 120 {
                self.low = self.low.saturating_sub(1).max(self.learned);
                self.steady = 0;
            }
        }
        self.last_bad = bad;
        self.low = self.low.min(self.at);
        self.levels[self.at]
    }
    fn min_khz(&self) -> u32 {
        self.levels[self.low]
    }
}
pub struct CpuPower {
    sys: PathBuf,
    log: PathBuf,
    original: Vec<(String, String)>,
    policy: Option<Policy>,
    userspace: bool,
    written: Option<u32>,
    written_min: Option<u32>,
    window: Instant,
    frames: u32,
    late: u32,
    busy: u32,
    underruns: u64,
    grace: Instant,
    telemetry_at: Instant,
    /// Config/cpu-levels.txt: what each game needed, so the next session starts there.
    levels_file: PathBuf,
    game: Option<String>,
}
const LEVELS_FILE: &str = "Config/cpu-levels.txt";
/// Config/cpu-levels.txt: one game a line, `key<TAB>floor kHz<TAB>ceiling floor kHz`.
fn remembered(file: &Path) -> std::collections::BTreeMap<String, (u32, u32)> {
    fs::read_to_string(file)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let mut f = line.split('\t');
            let key = f.next()?.to_string();
            let min = f.next()?.trim().parse().ok()?;
            let ceiling = f.next()?.trim().parse().ok()?;
            Some((key, (min, ceiling)))
        })
        .collect()
}
fn read(sys: &Path, name: &str) -> io::Result<String> {
    Ok(fs::read_to_string(sys.join(name))?.trim().into())
}
fn write(sys: &Path, name: &str, value: &str) -> io::Result<()> {
    let mut f = fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(sys.join(name))?;
    let command = format!("{value}\n");
    if f.write(command.as_bytes())? != command.len() {
        return Err(io::Error::new(
            io::ErrorKind::WriteZero,
            "short cpufreq write",
        ));
    }
    Ok(())
}
impl CpuPower {
    pub fn open(root: &Path) -> Self {
        Self::open_in(root, Path::new("/sys/devices/system/cpu/cpu0/cpufreq"))
    }
    fn open_in(root: &Path, sys: &Path) -> Self {
        let now = Instant::now();
        let mut s = Self {
            sys: sys.into(),
            log: root.join("Logs/power.log"),
            levels_file: root.join(LEVELS_FILE),
            game: None,
            original: vec![],
            policy: None,
            userspace: false,
            written: None,
            written_min: None,
            window: now,
            frames: 0,
            late: 0,
            busy: 0,
            underruns: 0,
            grace: now + Duration::from_secs(3),
            telemetry_at: now,
        };
        if root.join("DISABLE_POWER_SAVING").exists() {
            s.log("adaptive clocks disabled");
            return s;
        }
        let result = (|| -> io::Result<()> {
            let min = read(sys, "scaling_min_freq")?
                .parse::<u32>()
                .map_err(io::Error::other)?;
            let max = read(sys, "scaling_max_freq")?
                .parse::<u32>()
                .map_err(io::Error::other)?;
            let gov = read(sys, "scaling_governor")?;
            let governors =
                read(sys, "scaling_available_governors").unwrap_or_else(|_| gov.clone());
            let selected = ["schedutil", "ondemand", "conservative", "userspace"]
                .into_iter()
                .find(|g| governors.split_whitespace().any(|v| v == *g))
                .ok_or_else(|| io::Error::other("no supported governor"))?;
            let mut levels: Vec<u32> = read(sys, "scaling_available_frequencies")?
                .split_whitespace()
                .filter_map(|v| v.parse().ok())
                .filter(|&v| v >= 408_000 && v <= max.min(1_800_000))
                .collect();
            levels.sort_unstable();
            levels.dedup();
            if levels.is_empty() {
                return Err(io::Error::other("no verified frequency table"));
            }
            s.original = vec![
                ("scaling_min_freq".into(), min.to_string()),
                ("scaling_max_freq".into(), max.to_string()),
                ("scaling_governor".into(), gov.clone()),
            ];
            if gov == "userspace" {
                let speed = read(sys, "scaling_setspeed")?;
                speed.parse::<u32>().map_err(io::Error::other)?;
                s.original.push(("scaling_setspeed".into(), speed));
            }
            // Lower min first so a menu/standby ceiling never crosses it.
            write(sys, "scaling_min_freq", &levels[0].to_string())?;
            write(sys, "scaling_governor", selected)?;
            s.userspace = selected == "userspace";
            s.policy = Some(Policy::new(levels));
            s.log(&format!(
                "enabled governor={selected}; original={:?}",
                s.original
            ));
            Ok(())
        })();
        if let Err(e) = result {
            s.log(&format!("disabled: {e}"));
            s.restore();
        }
        s
    }
    fn log(&self, message: &str) {
        if let Some(dir) = self.log.parent() {
            let _ = fs::create_dir_all(dir);
        }
        if fs::metadata(&self.log).is_ok_and(|m| m.len() > 131072) {
            let _ = fs::rename(&self.log, self.log.with_extension("previous.log"));
        }
        if let Ok(mut f) = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log)
        {
            let _ = writeln!(f, "{:?} {message}", std::time::SystemTime::now());
        }
    }
    fn apply(&mut self, value: u32) {
        let low = self.policy.as_ref().map_or(0, |p| p.min_khz()).min(value);
        if self.written == Some(value) && (self.userspace || self.written_min == Some(low)) {
            return;
        }
        let max = |s: &Self| write(&s.sys, "scaling_max_freq", &value.to_string());
        let min = |s: &Self| {
            if s.userspace || s.written_min == Some(low) {
                Ok(())
            } else {
                write(&s.sys, "scaling_min_freq", &low.to_string())
            }
        };
        // Keep min <= max at every step: raise the ceiling before the floor, lower the floor before the ceiling.
        let raising = self.written.is_none_or(|w| value >= w);
        let result = if raising {
            max(self).and_then(|_| min(self))
        } else {
            min(self).and_then(|_| max(self))
        }
        .and_then(|_| {
            if self.userspace {
                write(&self.sys, "scaling_setspeed", &value.to_string())
            } else {
                Ok(())
            }
        });
        if let Err(e) = result {
            self.log(&format!("write failed: {e}; restoring"));
            self.restore();
            return;
        }
        let moved =
            self.written != Some(value) || (!self.userspace && self.written_min != Some(low));
        self.written = Some(value);
        if !self.userspace {
            self.written_min = Some(low);
        }
        if moved {
            self.log(&format!(
                "mode={:?} ceiling_khz={value} floor_khz={low}",
                self.policy.as_ref().map(|p| p.mode)
            ));
        }
    }
    pub fn mode(&mut self, mode: Mode, underruns: u64) {
        if self.telemetry_at.elapsed() >= Duration::from_secs(60) {
            self.telemetry_at = Instant::now();
            let battery = Path::new("/sys/class/power_supply/axp2202-battery");
            self.log(&format!("sample mode={mode:?} actual_khz={} capacity={} voltage_uv={} charge_counter_raw={} audio_underruns={underruns}",
                read(&self.sys,"scaling_cur_freq").unwrap_or_default(),read(battery,"capacity").unwrap_or_default(),read(battery,"voltage_now").unwrap_or_default(),read(battery,"charge_counter").unwrap_or_default()));
        }
        let Some(p) = &mut self.policy else { return };
        let changed = p.mode != mode;
        let value = p.change_mode(mode);
        if changed {
            self.window = Instant::now();
            self.grace = self.window + Duration::from_secs(3);
            self.frames = 0;
            self.late = 0;
            self.busy = 0;
            self.underruns = underruns;
        }
        self.apply(value);
    }
    pub fn sample(&mut self, frame_ready: bool, work: Duration, budget: Duration, underruns: u64) {
        if self.policy.is_none() {
            return;
        }
        self.frames += 1;
        self.late += u32::from(!frame_ready || work > budget);
        self.busy += u32::from(work > budget.mul_f32(0.65));
        if self.window.elapsed() < Duration::from_millis(500) {
            return;
        }
        let audio_bad = underruns > self.underruns;
        let bad = audio_bad || self.late * 20 > self.frames;
        let spare = self.busy * 20 < self.frames && self.late == 0 && !audio_bad;
        if Instant::now() >= self.grace {
            let value = self.policy.as_mut().unwrap().sample(bad, spare);
            self.apply(value);
        }
        self.frames = 0;
        self.late = 0;
        self.busy = 0;
        self.underruns = underruns;
        self.window = Instant::now();
    }
    /// The game now in the slot (a key that names it), or None on the shelf. A change saves what
    /// the last game needed and starts this one from what it needed before.
    pub fn game(&mut self, key: Option<&str>) {
        if self.game.as_deref() == key {
            return;
        }
        self.save_game();
        self.game = key.map(str::to_string);
        let remembered = key.and_then(|k| remembered(&self.levels_file).remove(k));
        if let Some(p) = &mut self.policy {
            p.load(remembered);
        }
        if let (Some(k), Some((min, ceiling))) = (key, remembered) {
            self.log(&format!(
                "{k}: remembered floor_khz={min} ceiling_floor_khz={ceiling}"
            ));
        }
    }
    fn save_game(&mut self) {
        let (Some(game), Some(p)) = (self.game.take(), &self.policy) else {
            return;
        };
        if p.windows == 0 {
            return; // never played: nothing learned
        }
        let (min, ceiling) = p.remember();
        let mut all = remembered(&self.levels_file);
        if all.get(&game) == Some(&(min, ceiling)) {
            return;
        }
        all.insert(game.clone(), (min, ceiling));
        let text: String = all
            .iter()
            .map(|(k, (m, c))| format!("{k}\t{m}\t{c}\n"))
            .collect();
        let tmp = self.levels_file.with_extension("tmp");
        let saved = self
            .levels_file
            .parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|_| fs::write(&tmp, text))
            .and_then(|_| fs::rename(&tmp, &self.levels_file));
        match saved {
            Ok(()) => self.log(&format!(
                "{game}: saved floor_khz={min} ceiling_floor_khz={ceiling}"
            )),
            Err(e) => self.log(&format!("{game}: can't save levels: {e}")),
        }
    }
    pub fn restore(&mut self) {
        self.save_game();
        self.policy = None;
        // Original max first: our min was only lowered; original min is then valid.
        for key in [
            "scaling_max_freq",
            "scaling_min_freq",
            "scaling_governor",
            "scaling_setspeed",
        ] {
            if let Some((_, v)) = self.original.iter().find(|(k, _)| k == key) {
                if let Err(e) = write(&self.sys, key, v) {
                    self.log(&format!("restore {key}: {e}"));
                }
            }
        }
        self.original.clear();
        self.written = None;
        self.written_min = None;
    }
}
impl Drop for CpuPower {
    fn drop(&mut self) {
        self.restore();
    }
}
// Linux CLOCK_BOOTTIME includes suspend; CLOCK_MONOTONIC excludes it.
pub fn suspended_seconds() -> Option<f64> {
    #[repr(C)]
    struct Timespec {
        sec: std::ffi::c_long,
        nsec: std::ffi::c_long,
    }
    unsafe extern "C" {
        fn clock_gettime(clock: std::ffi::c_int, tp: *mut Timespec) -> std::ffi::c_int;
    }
    let mut boot = Timespec { sec: 0, nsec: 0 };
    let mut mono = Timespec { sec: 0, nsec: 0 };
    unsafe {
        if clock_gettime(7, &mut boot) != 0 || clock_gettime(1, &mut mono) != 0 {
            return None;
        }
    }
    Some((boot.sec - mono.sec) as f64 + (boot.nsec - mono.nsec) as f64 / 1e9)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn policy() -> Policy {
        Policy::new(vec![
            408000, 816000, 1008000, 1200000, 1416000, 1608000, 1800000,
        ])
    }
    #[test]
    fn slow_down_fast_up_and_remember_failed_level() {
        let mut p = policy();
        p.change_mode(Mode::Game);
        for _ in 0..19 {
            assert_eq!(p.sample(false, true), 1800000)
        }
        assert_eq!(p.sample(false, true), 1608000);
        assert_eq!(p.sample(true, false), 1800000);
        for _ in 0..100 {
            p.sample(false, true);
        }
        assert_eq!(p.levels[p.at], 1800000);
    }
    #[test]
    fn games_hold_a_minimum_that_rises_on_trouble_and_decays() {
        let mut p = policy();
        p.change_mode(Mode::Game);
        assert_eq!(p.min_khz(), 1200000);
        p.sample(true, false);
        assert_eq!(p.min_khz(), 1416000);
        p.sample(true, false);
        assert_eq!(p.min_khz(), 1608000);
        for _ in 0..119 {
            p.sample(false, false);
        }
        assert_eq!(p.min_khz(), 1608000);
        p.sample(false, false);
        assert_eq!(p.min_khz(), 1416000);
        // It was late at 1.2 GHz, so it stays at what it needed instead of dipping back.
        for _ in 0..1000 {
            p.sample(false, true);
        }
        assert_eq!(p.min_khz(), 1416000);
        assert!(p.min_khz() <= p.levels[p.at]);
        assert_eq!(p.change_mode(Mode::Menu), 816000);
        assert_eq!(p.min_khz(), 408000);
    }
    #[test]
    fn floor_written_in_games_and_released_in_menu() {
        let (t, sys) = fixture();
        fs::write(sys.join("scaling_available_governors"), "schedutil").unwrap();
        let mut cpu2 = CpuPower::open_in(t.path(), &sys);
        cpu2.mode(Mode::Game, 0);
        assert_eq!(read(&sys, "scaling_min_freq").unwrap(), "1200000");
        assert_eq!(read(&sys, "scaling_max_freq").unwrap(), "1800000");
        cpu2.mode(Mode::Menu, 0);
        assert_eq!(read(&sys, "scaling_min_freq").unwrap(), "408000");
        assert_eq!(read(&sys, "scaling_max_freq").unwrap(), "816000");
        cpu2.mode(Mode::Standby, 0);
        assert_eq!(read(&sys, "scaling_max_freq").unwrap(), "408000");
        cpu2.mode(Mode::Game, 0);
        assert_eq!(read(&sys, "scaling_min_freq").unwrap(), "1200000");
        drop(cpu2);
        assert_eq!(read(&sys, "scaling_min_freq").unwrap(), "1200000");
        assert_eq!(read(&sys, "scaling_max_freq").unwrap(), "2000000");
    }
    #[test]
    fn floors_and_transitions_protect_gameplay() {
        let mut p = policy();
        assert_eq!(p.change_mode(Mode::Standby), 408000);
        assert_eq!(p.change_mode(Mode::Menu), 816000);
        assert_eq!(p.change_mode(Mode::Game), 1800000);
        for _ in 0..400 {
            p.sample(false, true);
        }
        assert_eq!(p.levels[p.at], 1200000);
        assert_eq!(p.change_mode(Mode::Boost), 1800000);
        for _ in 0..100 {
            assert_eq!(p.sample(false, true), 1800000);
        }
    }
    #[test]
    fn a_game_learns_up_at_once_and_down_after_a_clean_session() {
        let mut p = policy();
        p.load(None);
        p.change_mode(Mode::Game);
        p.sample(true, false); // late at 1.2 GHz
        p.sample(true, false); // the same hiccup: no second step
        for _ in 0..10 {
            p.sample(false, false);
        }
        assert_eq!(p.remember().0, 1416000);
        // Next time it starts there, and doesn't dip below it.
        let saved = p.remember();
        let mut p = policy();
        p.load(Some(saved));
        p.change_mode(Mode::Game);
        assert_eq!(p.min_khz(), 1416000);
        for _ in 0..PROBE_DOWN_AFTER {
            assert!(p.min_khz() >= 1416000);
            p.sample(false, true);
        }
        // Five clean minutes: try one step lower next time, never below the Brick's minimum.
        assert_eq!(p.remember().0, 1200000);
        let mut p = policy();
        p.load(Some((408000, 408000)));
        p.change_mode(Mode::Game);
        assert_eq!(p.min_khz(), 1200000);
    }
    #[test]
    fn levels_are_saved_per_game_on_the_card() {
        let (t, sys) = fixture();
        let mut cpu = CpuPower::open_in(t.path(), &sys);
        cpu.game(Some("GBA/Kirby"));
        cpu.mode(Mode::Game, 0);
        let p = cpu.policy.as_mut().unwrap();
        p.sample(true, false);
        cpu.game(None);
        let file = fs::read_to_string(t.path().join(LEVELS_FILE)).unwrap();
        // the fixture has no 1.416 GHz step: one up from 1.2 is 1.608
        assert_eq!(file, "GBA/Kirby\t1608000\t1608000\n");
        cpu.game(Some("GB/Tetris")); // never played: nothing written
        cpu.game(None);
        cpu.game(Some("GBA/Kirby"));
        cpu.mode(Mode::Game, 0);
        assert_eq!(read(&sys, "scaling_min_freq").unwrap(), "1608000");
    }
    fn fixture() -> (tempfile::TempDir, PathBuf) {
        let t = tempfile::tempdir().unwrap();
        let sys = t.path().join("cpu");
        fs::create_dir(&sys).unwrap();
        for (n, v) in [
            ("scaling_governor", "userspace"),
            ("scaling_available_governors", "userspace schedutil"),
            ("scaling_min_freq", "1200000"),
            ("scaling_max_freq", "2000000"),
            ("scaling_setspeed", "1608000"),
            (
                "scaling_available_frequencies",
                "2000000 1800000 1608000 1200000 1008000 816000 408000",
            ),
        ] {
            fs::write(sys.join(n), v).unwrap();
        }
        (t, sys)
    }
    #[test]
    fn restores_original_policy_and_does_not_overclock() {
        let (t, sys) = fixture();
        let mut cpu = CpuPower::open_in(t.path(), &sys);
        cpu.mode(Mode::Game, 0);
        assert_eq!(read(&sys, "scaling_max_freq").unwrap(), "1800000");
        cpu.mode(Mode::Standby, 0);
        assert_eq!(read(&sys, "scaling_max_freq").unwrap(), "408000");
        drop(cpu);
        for (n, v) in [
            ("scaling_min_freq", "1200000"),
            ("scaling_max_freq", "2000000"),
            ("scaling_governor", "userspace"),
            ("scaling_setspeed", "1608000"),
        ] {
            assert_eq!(read(&sys, n).unwrap(), v);
        }
    }
    #[test]
    fn missing_table_and_optout_leave_hardware_unchanged() {
        let (t, sys) = fixture();
        fs::remove_file(sys.join("scaling_available_frequencies")).unwrap();
        let cpu = CpuPower::open_in(t.path(), &sys);
        assert!(cpu.policy.is_none());
        assert_eq!(read(&sys, "scaling_min_freq").unwrap(), "1200000");
        fs::write(t.path().join("DISABLE_POWER_SAVING"), "").unwrap();
        let cpu = CpuPower::open_in(t.path(), &sys);
        assert!(cpu.policy.is_none());
    }
    #[test]
    fn userspace_only_firmware_gets_explicit_speed() {
        let (t, sys) = fixture();
        fs::write(sys.join("scaling_available_governors"), "userspace").unwrap();
        let mut cpu = CpuPower::open_in(t.path(), &sys);
        cpu.mode(Mode::Menu, 0);
        assert_eq!(read(&sys, "scaling_setspeed").unwrap(), "816000");
        cpu.restore();
        assert_eq!(read(&sys, "scaling_setspeed").unwrap(), "1608000");
    }
    #[test]
    fn failed_write_disables_controller_and_rolls_back() {
        let (t, sys) = fixture();
        let mut cpu = CpuPower::open_in(t.path(), &sys);
        fs::remove_file(sys.join("scaling_max_freq")).unwrap();
        fs::create_dir(sys.join("scaling_max_freq")).unwrap();
        cpu.mode(Mode::Standby, 0);
        assert!(cpu.policy.is_none());
        assert_eq!(read(&sys, "scaling_min_freq").unwrap(), "1200000");
        assert_eq!(read(&sys, "scaling_governor").unwrap(), "userspace");
    }
}
