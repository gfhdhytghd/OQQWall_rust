use oqqwall_rust_core::IngressMessage;
use oqqwall_rust_core::anonymous::detect_anonymous;

fn msg(text: &str) -> IngressMessage {
    IngressMessage {
        text: text.to_string(),
        attachments: Vec::new(),
    }
}

#[test]
fn anonymous_positive_signal() {
    let messages = vec![msg("帮我匿名一下")];
    assert!(detect_anonymous(&messages));
}

#[test]
fn anonymous_negative_signal() {
    let messages = vec![msg("不匿名，直接发")];
    assert!(!detect_anonymous(&messages));
}

#[test]
fn anonymous_recent_precedence_negative_wins() {
    let messages = vec![msg("匿名一下"), msg("算了不匿名")];
    assert!(!detect_anonymous(&messages));
}

#[test]
fn anonymous_recent_precedence_positive_wins() {
    let messages = vec![msg("不匿名"), msg("还是匿一下吧")];
    assert!(detect_anonymous(&messages));
}
