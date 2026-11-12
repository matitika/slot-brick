use slot::bootlogo::{decode_any, encode, encode_v5, fit_to, refresh_slot_os, Bmp, Outcome, BRICK_PANEL};

fn solid(w: usize, h: usize, c: [u8; 3]) -> Bmp {
    Bmp { width: w, height: h, rgb: vec![c; w * h] }
}

#[test]
fn a_logo_round_trips_through_the_v5_form() {
    let mut img = solid(4, 3, [10, 20, 30]);
    img.rgb[5] = [200, 100, 50];
    let back = decode_any(&encode_v5(&img)).expect("reads its own V5");
    assert_eq!((back.width, back.height), (4, 3));
    assert_eq!(back.rgb, img.rgb);
    // and the 24-bit form Boot Logo Studio writes
    let back = decode_any(&encode(&img)).expect("reads 24-bit");
    assert_eq!(back.rgb, img.rgb);
}

#[test]
fn a_logo_of_another_size_is_fitted_and_centred() {
    let mut img = solid(720, 480, [0, 0, 0]);
    img.rgb[0] = [9, 9, 9]; // the top-left colour fills the rest
    img.rgb[720 * 240 + 360] = [255, 0, 0];
    let fitted = fit_to(&img, BRICK_PANEL).unwrap();
    assert_eq!((fitted.width, fitted.height), BRICK_PANEL);
    // 720x480 fits as 1024x683, 42 rows above it
    assert_eq!(fitted.rgb[10 * 1024 + 500], [9, 9, 9], "the band above");
    let red = fitted.rgb.iter().position(|c| *c == [255, 0, 0]).unwrap();
    let (x, y) = (red % 1024, red / 1024);
    assert!((508..=516).contains(&x) && (382..=390).contains(&y), "the centre stays the centre: {x},{y}");
}

#[test]
fn the_chosen_logo_is_installed_then_the_image_s_own_comes_back() {
    let card = tempfile::tempdir().unwrap();
    let root = card.path();
    let own = encode_v5(&solid(1024, 768, [1, 2, 3]));
    std::fs::write(root.join("bootlogo.bmp"), &own).unwrap();
    // nothing chosen: nothing to do
    assert_eq!(refresh_slot_os(root).unwrap(), Outcome::Same);
    // a 24-bit logo from Boot Logo Studio, another size
    std::fs::create_dir_all(root.join("Config")).unwrap();
    std::fs::write(root.join("Config/bootlogo.bmp"), encode(&solid(640, 480, [200, 0, 0]))).unwrap();
    assert_eq!(refresh_slot_os(root).unwrap(), Outcome::Installed);
    let shown = decode_any(&std::fs::read(root.join("bootlogo.bmp")).unwrap()).unwrap();
    assert_eq!((shown.width, shown.height), BRICK_PANEL);
    assert_eq!(shown.rgb[384 * 1024 + 512], [200, 0, 0]);
    assert_eq!(refresh_slot_os(root).unwrap(), Outcome::Same, "already shown");
    // Gamebit's Restore deletes the chosen logo: the image's own comes back
    std::fs::remove_file(root.join("Config/bootlogo.bmp")).unwrap();
    assert_eq!(refresh_slot_os(root).unwrap(), Outcome::Installed);
    assert_eq!(std::fs::read(root.join("bootlogo.bmp")).unwrap(), own);
    assert!(!root.join("bootlogo.slotos.bmp").exists());
}

#[test]
fn a_logo_slot_cannot_read_is_left_alone() {
    let card = tempfile::tempdir().unwrap();
    let root = card.path();
    std::fs::write(root.join("bootlogo.bmp"), b"theirs").unwrap();
    std::fs::create_dir_all(root.join("Config")).unwrap();
    std::fs::write(root.join("Config/bootlogo.bmp"), b"not a bmp").unwrap();
    assert_eq!(refresh_slot_os(root).unwrap(), Outcome::Invalid);
    assert_eq!(std::fs::read(root.join("bootlogo.bmp")).unwrap(), b"theirs");
}
