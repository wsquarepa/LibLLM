#![cfg(feature = "test-support")]

use crossterm::event::KeyCode;
use libllm_core::session::Session;
use libllm_tui::harness::Harness;

#[tokio::test]
async fn boots_and_renders_input_focus() {
    let mut session = Session::default();
    let h = Harness::builder()
        .size(100, 30)
        .no_db()
        .no_api()
        .build(&mut session)
        .await
        .unwrap();

    let screen = h.screen();
    assert!(
        screen.contains("Input"),
        "expected the input box label on screen, got:\n{screen}"
    );

    // Focus::Input has no associated dialog, so active_dialog is None.
    assert_eq!(
        h.observe().active_dialog,
        None,
        "expected no active dialog at startup (Input focus)"
    );
}

#[tokio::test]
async fn streams_a_completion_into_the_head_message() {
    let mut session = Session::default();
    let mut h = Harness::builder()
        .size(100, 30)
        .temp_db()
        .mock_api()
        .build(&mut session)
        .await
        .unwrap();

    h.enqueue_completion(&["Hello", ", ", "world"]);
    h.type_text("hi").await;
    h.key(KeyCode::Enter).await;
    h.pump().await;

    let obs = h.observe();
    assert!(!obs.is_streaming, "stream should be finished after pump");
    assert!(
        obs.head_text
            .as_deref()
            .unwrap_or("")
            .contains("Hello, world"),
        "expected streamed text in head message, got head_text={:?}\nscreen:\n{}",
        obs.head_text,
        h.screen()
    );
}
