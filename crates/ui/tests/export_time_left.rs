//! The time an export has left, as the status bar words it.

use bettercut_ui::panels::export_time_left;

#[test]
fn nothing_is_guessed_in_the_first_seconds() {
    assert_eq!(export_time_left(1.0, 0.5), None);
    assert_eq!(export_time_left(30.0, 0.01), None);
}

#[test]
fn the_time_left_is_rounded_so_it_settles() {
    // Half done after 20 s: 20 s to go.
    assert_eq!(
        export_time_left(20.0, 0.5).as_deref(),
        Some("about 20 s left")
    );
    // A quarter done after 60 s: three minutes to go.
    assert_eq!(
        export_time_left(60.0, 0.25).as_deref(),
        Some("about 3 min left")
    );
    // A tenth done after 15 min: over two hours to go.
    assert_eq!(
        export_time_left(900.0, 0.1).as_deref(),
        Some("about 2 h 15 min left")
    );
    assert_eq!(export_time_left(50.0, 1.0).as_deref(), Some("finishing"));
}
