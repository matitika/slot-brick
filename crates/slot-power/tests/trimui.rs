use std::fs;
use std::path::PathBuf;

use slot_power::{Board, DevicePlatform, LedState, Platform};
use tempfile::TempDir;

fn brick_pro_tree() -> (TempDir, TempDir) {
    let sys = tempfile::tempdir().unwrap();
    let dev = tempfile::tempdir().unwrap();
    let battery = sys.path().join("class/power_supply/axp2202-battery");
    fs::create_dir_all(&battery).unwrap();
    fs::write(battery.join("type"), "Battery\n").unwrap();
    fs::write(battery.join("capacity"), "64\n").unwrap();
    fs::write(battery.join("status"), "Discharging\n").unwrap();
    let gpio = sys.path().join("class/gpio/gpio227");
    fs::create_dir_all(&gpio).unwrap();
    fs::write(gpio.join("value"), "0").unwrap();
    let motor = sys.path().join("class/motor");
    fs::create_dir_all(&motor).unwrap();
    fs::write(motor.join("voltage"), "0").unwrap();
    let anim = sys.path().join("class/led_anim");
    fs::create_dir_all(&anim).unwrap();
    for f in ["max_scale", "max_scale_f1f2", "max_scale_lr", "effect_m", "effect_f1", "effect_f2", "effect_lr"] {
        fs::write(anim.join(f), "0").unwrap();
    }
    (sys, dev)
}

fn brick_pro(sys: &TempDir, dev: &TempDir) -> DevicePlatform {
    DevicePlatform::probe_board(
        sys.path(),
        dev.path(),
        PathBuf::from("/mnt/SDCARD"),
        Board::TrimUiBrickPro,
    )
}

fn read(sys: &TempDir, file: &str) -> String {
    fs::read_to_string(sys.path().join(file))
        .unwrap()
        .trim()
        .to_string()
}

#[test]
fn the_board_comes_from_the_launcher_before_cpuinfo() {
    assert_eq!(
        Board::from_hints(Some("brickpro"), "Hardware : TG3040"),
        Board::TrimUiBrickPro
    );
    assert_eq!(
        Board::from_hints(None, "Hardware\t: TG4040"),
        Board::TrimUiBrickPro
    );
    assert_eq!(
        Board::from_hints(None, "Hardware\t: TG3040"),
        Board::TrimUiBrick
    );
    assert_eq!(Board::from_hints(Some(""), "sun50iw9"), Board::Anbernic);
    assert!(Board::TrimUiBrickPro.is_trimui());
    assert!(!Board::Anbernic.is_trimui());
}

#[test]
fn the_brick_pro_rumbles_through_its_gpio_capped_at_two_and_a_half_volts() {
    let (sys, dev) = brick_pro_tree();
    let mut p = brick_pro(&sys, &dev);
    assert!(p.report().contains("motor gpio227"), "{}", p.report());
    p.set_rumble(u16::MAX);
    assert_eq!(read(&sys, "class/gpio/gpio227/value"), "1");
    assert_eq!(read(&sys, "class/motor/voltage"), "2500000");
    p.set_rumble(0);
    assert_eq!(read(&sys, "class/gpio/gpio227/value"), "0");
}

#[test]
fn the_top_bar_lights_by_state_and_goes_dark_when_off() {
    // the app passes Off when the LEDs setting wants them dark (Battery mode while running)
    let (sys, dev) = brick_pro_tree();
    let mut p = brick_pro(&sys, &dev);
    p.set_led(LedState::Running);
    assert_eq!(read(&sys, "class/led_anim/effect_rgb_hex_m"), "00FF00");
    p.set_led(LedState::Low);
    assert_eq!(read(&sys, "class/led_anim/effect_rgb_hex_m"), "FF0000");
    assert_ne!(read(&sys, "class/led_anim/max_scale"), "0");
    p.set_led(LedState::Off);
    assert_eq!(read(&sys, "class/led_anim/max_scale"), "0");
}

#[test]
fn the_brick_pro_reads_its_battery_like_the_anbernics() {
    let (sys, dev) = brick_pro_tree();
    let p = brick_pro(&sys, &dev);
    assert_eq!(p.battery().map(|b| b.percent), Some(64));
    assert!(p.report().starts_with("TrimUiBrickPro"), "{}", p.report());
}

#[test]
fn without_dev_disp_a_backlight_class_still_dims_the_panel() {
    let (sys, dev) = brick_pro_tree();
    let bl = sys.path().join("class/backlight/backlight0");
    fs::create_dir_all(&bl).unwrap();
    fs::write(bl.join("max_brightness"), "255\n").unwrap();
    fs::write(bl.join("brightness"), "0\n").unwrap();
    let mut p = brick_pro(&sys, &dev);
    p.set_backlight(u8::MAX);
    assert_eq!(read(&sys, "class/backlight/backlight0/brightness"), "255");
    p.set_backlight(0);
    assert_eq!(read(&sys, "class/backlight/backlight0/brightness"), "0");
}

fn brick_on(sys: &TempDir, dev: &TempDir, card: &TempDir) -> DevicePlatform {
    DevicePlatform::probe_board(
        sys.path(),
        dev.path(),
        card.path().to_path_buf(),
        Board::TrimUiBrick,
    )
}

#[test]
fn the_brick_sleeps_through_the_cards_sleep_script() {
    let (sys, dev) = brick_pro_tree();
    let card = tempfile::tempdir().unwrap();
    fs::create_dir_all(card.path().join("System")).unwrap();
    // no execute bit, as on a FAT card: run through sh
    fs::write(
        card.path().join("System/brick-sleep.sh"),
        "touch \"$SLOT_ROOT/slept\"\n",
    )
    .unwrap();
    let mut brick = brick_on(&sys, &dev, &card);
    brick.sleep();
    let slept = card.path().join("slept");
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !slept.exists() && std::time::Instant::now() < until {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(slept.exists(), "the sleep script ran");
}

#[test]
fn a_brick_card_without_the_sleep_script_only_dozes() {
    let (sys, dev) = brick_pro_tree();
    let card = tempfile::tempdir().unwrap();
    let mut brick = brick_on(&sys, &dev, &card);
    brick.sleep();
    assert!(fs::read_dir(card.path()).unwrap().next().is_none());
}

#[test]
fn every_led_zone_of_the_brick_follows_the_state() {
    let (sys, dev) = brick_pro_tree();
    let mut brick = brick_pro(&sys, &dev);
    // the firmware leaves F1/F2 lit: off must darken them too
    fs::write(sys.path().join("class/led_anim/max_scale_f1f2"), "60").unwrap();
    brick.set_led(LedState::Off);
    for scale in ["max_scale", "max_scale_f1f2", "max_scale_lr"] {
        assert_eq!(read(&sys, &format!("class/led_anim/{scale}")), "0", "{scale}");
    }
    brick.set_led(LedState::Running);
    for zone in ["m", "f1", "f2", "lr"] {
        assert_eq!(read(&sys, &format!("class/led_anim/effect_rgb_hex_{zone}")), "00FF00", "{zone}");
    }
    for scale in ["max_scale", "max_scale_f1f2", "max_scale_lr"] {
        assert_ne!(read(&sys, &format!("class/led_anim/{scale}")), "0", "{scale}");
    }
}
