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
