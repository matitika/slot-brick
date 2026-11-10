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
