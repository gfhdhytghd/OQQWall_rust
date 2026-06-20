use oqqwall_rust_core::IngressMessage;
use oqqwall_rust_core::safety::detect_safe;

fn msg(text: &str) -> IngressMessage {
    IngressMessage {
        text: text.to_string(),
        attachments: Vec::new(),
    }
}

#[test]
fn safety_detects_unsafe() {
    let messages = vec![msg("你就是个傻逼")];
    assert!(!detect_safe(&messages));
}

#[test]
fn safety_passes_clean_text() {
    let messages = vec![msg("今天心情不错")];
    assert!(detect_safe(&messages));
}
