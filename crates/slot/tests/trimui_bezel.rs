mod common;

use common::{tmp_root_with_carts, tmp_root_with_gb_carts, tmp_root_with_gbc_carts, StubSnapshot};
use slot::app::{App, Phase};
use slot_store::{write_slot_state, BezelPick, Platform, Scale, SlotState};
use slot_ui::BezelArt;
use tempfile::TempDir;

/// Playing a game on the Brick's panel, laid out as device_app lays out TrimUI.
fn playing(platform: Platform, panel: (u32, u32), state: SlotState) -> (TempDir, App) {
    slot_gfx::set_brick_layout(true);
    let d = match platform {
        Platform::Gb => tmp_root_with_gb_carts(&["Game"]),
        Platform::Gbc => tmp_root_with_gbc_carts(&["Game"]),
        _ => tmp_root_with_carts(&["Game"]),
    };
    write_slot_state(
        d.path(),
        &SlotState {
            cart: Some("Game".into()),
            cart_platform: Some(platform),
            clock_set: true,
            ..state
        },
    )
    .unwrap();
    let mut a = App::boot(d.path());
    a.set_panel(panel);
    a.set_snapshot(StubSnapshot::boxed());
    a.on_core_ready();
    for _ in 0..120 {
        a.update(1.0 / 60.0);
    }
    assert!(matches!(a.phase(), Phase::Playing { .. }));
    (d, a)
}

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
    assert!(
        app.hidden_rows().contains(&QuickRow::LidOff),
        "the Brick has no lid"
    );
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

#[test]
fn the_brick_frames_its_integer_and_fit_pictures() {
    for (platform, scale, want) in [
        // the SP frame is not offered on the Brick: the stored default shows as Integer
        (Platform::Gb, Scale::SpFrame, Some(BezelArt::Integer4x3)),
        (Platform::Gb, Scale::Integer, Some(BezelArt::Integer4x3)),
        (Platform::Gb, Scale::Fit, Some(BezelArt::Fit4x3)),
        (Platform::Gbc, Scale::Integer, Some(BezelArt::Integer4x3)),
        (Platform::Gba, Scale::Integer, Some(BezelArt::Integer4x3)),
        (Platform::Gba, Scale::Fit, Some(BezelArt::Fit4x3)),
    ] {
        let state = match platform {
            // GBA left at its stored default, the AGS-102 strip, which the Brick cannot show
            Platform::Gba => SlotState {
                scale_gba: scale,
                ..SlotState::default()
            },
            Platform::Gbc => SlotState {
                bezel_gbc: BezelPick::Default,
                scale_gbc: scale,
                ..SlotState::default()
            },
            _ => SlotState {
                bezel_gb: BezelPick::Default,
                scale_gb: scale,
                ..SlotState::default()
            },
        };
        let (_d, mut a) = playing(platform, (1024, 768), state);
        a.set_brick_frames(true);
        assert_eq!(a.bezel_art(), want, "{platform:?} {scale:?}");
        assert_eq!(a.resolved_bezel(platform), BezelPick::Default);
        assert_eq!(a.game_bezel_size(), (1024, 768));
        if let Some(kind) = want {
            let face = slot_ui::brick_frame_face(platform, kind).expect("a 1024x768 frame");
            assert_eq!(face.len(), 1024 * 768 * 4);
        }
    }
}

#[test]
fn the_brick_offers_game_boy_integer_and_fit_only() {
    let (_d, mut a) = playing(Platform::Gbc, (1024, 768), SlotState::default());
    a.set_brick_frames(true);
    assert_eq!(a.scale_choices(Platform::Gbc), [Scale::Integer, Scale::Fit]);
    assert_eq!(a.resolved_scale(Platform::Gbc), Scale::Integer);
    assert_eq!(a.bezel_art(), Some(BezelArt::Integer4x3));
    a.set_brick_frames(false);
    assert!(a.scale_choices(Platform::Gbc).contains(&Scale::SpFrame));
}
