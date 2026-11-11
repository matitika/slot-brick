use slot::app::App;

#[test]
fn a_1024_by_768_panel_shows_the_lime_placeholder_unless_it_is_turned_off() {
    let d = tempfile::tempdir().unwrap();
    let mut app = App::boot(d.path());
    app.set_panel((1024, 768));
    assert!(app.placeholder_bezel().is_some(), "upstream's placeholder");
    // the TrimUI Brick: black around the game
    app.set_placeholder_bezels(false);
    assert_eq!(app.placeholder_bezel(), None);
}

#[test]
fn gba_on_a_1024_by_768_panel_offers_fit_integer_and_full_screen() {
    use slot_store::{Platform, Scale};
    // as device_app sets it on TrimUI (process-wide: this binary's tests all run as a Brick)
    slot_gfx::set_brick_layout(true);
    let d = tempfile::tempdir().unwrap();
    let mut app = App::boot(d.path());
    app.set_panel((1024, 768));
    assert_eq!(
        app.scale_choices(Platform::Gba),
        [Scale::Fit, Scale::Integer, Scale::FullScreen]
    );
}

#[test]
fn the_leds_setting_picks_the_brick_s_led_state() {
    use slot_power::LedState;
    use slot_store::LedMode;
    use slot_ui::QuickRow;
    let d = tempfile::tempdir().unwrap();
    let mut app = App::boot(d.path());
    // not offered, and the Anbernics' LEDs as upstream has them, until the device says so
    assert!(app.hidden_rows().contains(&QuickRow::Leds));
    app.set_led_modes(true);
    assert!(!app.hidden_rows().contains(&QuickRow::Leds));
    assert!(app.hidden_rows().contains(&QuickRow::LidOff), "the Brick has no lid");
    // Battery, the default: dark while running (no battery read yet counts as running)
    assert_eq!(app.led_state(), LedState::Off);
    let mut s = slot_store::read_slot_state(d.path());
    s.leds = LedMode::Always;
    slot_store::write_slot_state(d.path(), &s).unwrap();
    let mut app = App::boot(d.path());
    app.set_led_modes(true);
    assert_eq!(app.led_state(), LedState::Running);
    s.leds = LedMode::Off;
    slot_store::write_slot_state(d.path(), &s).unwrap();
    let mut app = App::boot(d.path());
    app.set_led_modes(true);
    assert_eq!(app.led_state(), LedState::Off);
}
