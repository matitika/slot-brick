//! The TrimUI Brick's buttons read straight from the A133's GPIO pins, for Slot OS, which runs
//! no trimui_inputd to turn them into evdev. Every button is one pin, pressed when low. The
//! power key is not among them: it is an input device of its own, read as on stock.
//!
//! The pins and registers are those Mikhailzrick's open trimui_inputd reads
//! (github.com/Mikhailzrick/Trimui_Inputd, src/devices/brick), as LordNordyx's Slot-Brick
//! (github.com/LordNordyx/Slot-Brick, GPL-3.0) found them.
use slot_input::{Btn, RawEvent};

/// The pin controller's registers, physical; banks of 0x24 bytes, A at the start.
const PIO: libc::off_t = 0x0300_B000;
const BANK_STRIDE: usize = 0x24;
const CFG0: usize = 0x00;
const DATA: usize = 0x10;
const MAP_LEN: usize = 0x1000;

/// A pin is bank * 32 + its number in the bank: 109 is PD13.
pub const BRICK_PINS: [(u32, Btn); 17] = [
    (113, Btn::Up),
    (110, Btn::Down),
    (111, Btn::Left),
    (112, Btn::Right),
    (108, Btn::A),
    (109, Btn::B),
    (114, Btn::X),
    (115, Btn::Y),
    (36, Btn::L1),
    (241, Btn::R1),
    (34, Btn::L2),
    (37, Btn::R2),
    (239, Btn::Select),
    (240, Btn::Start),
    (238, Btn::Menu),
    (116, Btn::VolDown),
    (117, Btn::VolUp),
];

pub struct BrickPins {
    regs: *mut u8,
    /// Bit i set: BRICK_PINS[i] held.
    held: u32,
}

// The mapping is only read and written through volatile accesses from the one thread that
// polls input.
unsafe impl Send for BrickPins {}

impl BrickPins {
    pub fn open() -> Result<Self, String> {
        let regs = unsafe {
            let fd = libc::open(c"/dev/mem".as_ptr(), libc::O_RDWR | libc::O_SYNC);
            if fd < 0 {
                return Err(format!("/dev/mem: {}", std::io::Error::last_os_error()));
            }
            let map = libc::mmap(
                std::ptr::null_mut(),
                MAP_LEN,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd,
                PIO,
            );
            libc::close(fd);
            if map == libc::MAP_FAILED {
                return Err(format!("mmap: {}", std::io::Error::last_os_error()));
            }
            map as *mut u8
        };
        let pins = BrickPins { regs, held: 0 };
        // every button pin an input (function 0), as trimui_inputd sets them
        for &(pin, _) in &BRICK_PINS {
            pins.set_input(pin);
        }
        Ok(pins)
    }

    fn reg(&self, pin: u32, offset: usize) -> *mut u32 {
        let bank = (pin / 32) as usize;
        unsafe { self.regs.add(bank * BANK_STRIDE + offset) as *mut u32 }
    }

    fn set_input(&self, pin: u32) {
        let reg = self.reg(pin, CFG0 + ((pin % 32) / 8) as usize * 4);
        let shift = (pin % 8) * 4;
        unsafe { reg.write_volatile(reg.read_volatile() & !(0xf << shift)) };
    }

    fn pressed(&self, pin: u32) -> bool {
        let data = unsafe { self.reg(pin, DATA).read_volatile() };
        data & (1 << (pin % 32)) == 0
    }

    pub fn held_now(&self) -> Vec<Btn> {
        let held = held_of(|pin| self.pressed(pin));
        buttons(held).collect()
    }

    pub fn poll(&mut self) -> Vec<RawEvent> {
        let held = held_of(|pin| self.pressed(pin));
        let out = changes(self.held, held).collect();
        self.held = held;
        out
    }
}

impl Drop for BrickPins {
    fn drop(&mut self) {
        unsafe { libc::munmap(self.regs as *mut libc::c_void, MAP_LEN) };
    }
}

/// Which pins read pressed, bit i for BRICK_PINS[i].
pub fn held_of(pressed: impl Fn(u32) -> bool) -> u32 {
    BRICK_PINS
        .iter()
        .enumerate()
        .filter(|(_, &(pin, _))| pressed(pin))
        .fold(0, |held, (i, _)| held | 1 << i)
}

fn buttons(held: u32) -> impl Iterator<Item = Btn> {
    BRICK_PINS
        .iter()
        .enumerate()
        .filter(move |(i, _)| held & 1 << i != 0)
        .map(|(_, &(_, btn))| btn)
}

/// The presses and releases from `old` to `new`.
pub fn changes(old: u32, new: u32) -> impl Iterator<Item = RawEvent> {
    BRICK_PINS
        .iter()
        .enumerate()
        .filter_map(move |(i, &(_, btn))| {
            let bit = 1 << i;
            (old & bit != new & bit).then_some(match new & bit != 0 {
                true => RawEvent::Down(btn),
                false => RawEvent::Up(btn),
            })
        })
}
