/// A failed product-version save fails the step instead of vanishing.
#[test]
fn a_failed_product_version_save_is_an_error() {
    let failed = super::persisted_version(Err(otto_core::Error::Internal("disk full".into())));
    assert!(failed.unwrap_err().to_string().contains("disk full"));
}

#[test]
fn a_canvas_step_id_cannot_leave_the_canvas_dir() {
    assert_eq!(
        super::canvas_rel("r1", "canvas_1", "mmd"),
        "workflow-canvas/r1/canvas_1.mmd"
    );
    let escaped = super::canvas_rel("r1", "../../../../tmp/x", "mmd");
    assert_eq!(escaped, "workflow-canvas/r1/tmp-x.mmd");
    assert_eq!(
        super::canvas_rel("r1", "..", "d2"),
        "workflow-canvas/r1/step.d2"
    );
}
