// The Brick's GBA layout is a process-wide switch, so it is tested in a binary of its own.
use slot_gfx::{set_brick_layout, table_placement, Sizing};

const GBA: (u32, u32) = (240, 160);
const GB: (u32, u32) = (160, 144);
const PANEL: (u32, u32) = (1024, 768);

#[test]
fn gba_on_the_brick_fits_the_width_keeps_4x_or_fills_the_panel() {
    set_brick_layout(true);
    let at = |s| table_placement(PANEL, GBA, s);
    assert_eq!(at(Sizing::Band), Some([0.0, 42.0, 1024.0, 683.0]), "Fit, the default");
    assert_eq!(at(Sizing::Integer), Some([32.0, 64.0, 960.0, 640.0]), "a sharp 4x");
    assert_eq!(at(Sizing::Panel), Some([0.0, 0.0, 1024.0, 768.0]), "Full Screen");
    // the width fill keeps the GBA's 3:2 shape
    let [_, _, w, h] = at(Sizing::Band).unwrap();
    assert!((w / h - 1.5).abs() < 0.01);
    // Game Boy keeps upstream's values
    assert_eq!(table_placement(PANEL, GB, Sizing::Panel), Some([0.0, 0.0, 1024.0, 768.0]));
    set_brick_layout(false);
    assert_eq!(table_placement(PANEL, GBA, Sizing::Panel), Some([32.0, 64.0, 960.0, 640.0]), "the RG DS+ keeps 4x");
}
