use slot::input::evdev::{decode, Ev, Hat, Keymap, Triggers, ABS_RZ, ABS_Z, EVENT_BYTES, EV_ABS};
use slot_input::{Btn, RawEvent};

const EV_KEY: u16 = 0x01;
const EV_SW: u16 = 0x05;
const SW_LID: u16 = 0x00;

fn packet(kind: u16, code: u16, value: i32) -> [u8; EVENT_BYTES] {
    let mut b = [0u8; EVENT_BYTES];
    b[16..18].copy_from_slice(&kind.to_le_bytes());
    b[18..20].copy_from_slice(&code.to_le_bytes());
    b[20..24].copy_from_slice(&value.to_le_bytes());
    b
}

fn abs(code: u16, value: i32) -> Ev {
    Ev {
        kind: EV_ABS,
        code,
        value,
    }
}

#[test]
fn the_trimui_face_buttons_land_where_they_are_printed() {
    let expected = [
        (0x130, Btn::B),
        (0x131, Btn::A),
        (0x133, Btn::Y),
        (0x134, Btn::X),
        (0x136, Btn::L1),
        (0x137, Btn::R1),
        (0x13a, Btn::Select),
        (0x13b, Btn::Start),
        (0x13c, Btn::Menu),
        (115, Btn::VolUp),
        (114, Btn::VolDown),
        (116, Btn::Power),
    ];
    for (code, want) in expected {
        assert_eq!(Keymap::TrimUi.btn(code), Some(want), "code {code:#x}");
    }
    let down = decode(&packet(EV_KEY, 0x131, 1)).and_then(|ev| Keymap::TrimUi.to_raw(ev));
    assert_eq!(down, Some(RawEvent::Down(Btn::A)));
}

#[test]
fn trimui_triggers_press_past_half_travel_and_release_below_it() {
    let mut t = Triggers::new(128);
    assert_eq!(t.feed(abs(ABS_Z, 40)), None);
    assert_eq!(t.feed(abs(ABS_Z, 255)), Some(RawEvent::Down(Btn::L2)));
    assert_eq!(t.feed(abs(ABS_Z, 200)), None);
    assert_eq!(t.feed(abs(ABS_Z, 0)), Some(RawEvent::Up(Btn::L2)));
    assert_eq!(t.feed(abs(ABS_RZ, 255)), Some(RawEvent::Down(Btn::R2)));
    assert_eq!(t.feed(abs(0x10, 1)), None, "a hat is not a trigger");
}

#[test]
fn each_trimui_trigger_uses_its_own_halfway_point() {
    let mut t = Triggers::axes(128, 2000);
    assert_eq!(t.feed(abs(ABS_Z, 128)), Some(RawEvent::Down(Btn::L2)));
    assert_eq!(t.feed(abs(ABS_RZ, 1999)), None);
    assert_eq!(t.feed(abs(ABS_RZ, 2000)), Some(RawEvent::Down(Btn::R2)));
}

#[test]
fn digital_trimui_l2_r2_are_ignored_so_the_analog_axes_own_them() {
    let key = |code| decode(&packet(EV_KEY, code, 1)).and_then(|ev| Keymap::TrimUi.to_raw(ev));
    assert_eq!(key(0x138), None);
    assert_eq!(key(0x139), None);
}

#[test]
fn a_trimui_switch_event_is_never_taken_for_a_lid() {
    let shut = decode(&packet(EV_SW, SW_LID, 1)).unwrap();
    assert_eq!(Keymap::TrimUi.to_raw(shut), None, "the Brick has no lid");
    assert_eq!(Keymap::BaseOs.to_raw(shut), Some(RawEvent::Down(Btn::Lid)));
}

#[test]
fn a_trimui_trigger_is_not_taken_for_the_d_pad() {
    let hat = Hat::new(Keymap::TrimUi);
    assert!(!hat.takes(abs(ABS_Z, 255)));
    assert!(!hat.takes(abs(ABS_RZ, 255)));
    assert!(hat.takes(abs(0x10, 1)), "the hat is still the d-pad");
    let line = slot::input::trace::event_line(Keymap::TrimUi, "event1", 0, abs(ABS_Z, 255));
    assert!(line.ends_with("-> L2"), "{line}");
}

#[test]
fn the_keymap_can_be_named() {
    assert_eq!(Keymap::parse("trimui"), Some(Keymap::TrimUi));
    assert_eq!(Keymap::parse(" Brick "), Some(Keymap::TrimUi));
    assert_eq!(Keymap::parse("dsos"), Some(Keymap::DsOs));
    assert_eq!(Keymap::parse("nope"), None);
}
