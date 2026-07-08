use oqqwall_rust_core::decide::decide;
use oqqwall_rust_core::event::{Event, IngressEvent, IngressIgnoreReason};
use oqqwall_rust_core::{Command, CoreConfig, IngressCommand, IngressMessage, StateView};

#[test]
fn ingress_blacklisted_is_ignored() {
    let mut state = StateView::default();
    state
        .blacklist
        .entry("group-a".to_string())
        .or_default()
        .insert("user-1".to_string(), Some("spam".to_string()));

    let ingress = IngressCommand {
        profile_id: "bot".to_string(),
        chat_id: "user-1".to_string(),
        user_id: "user-1".to_string(),
        sender_name: None,
        group_id: "group-a".to_string(),
        platform_msg_id: "msg-1".to_string(),
        message: IngressMessage {
            text: "hello".to_string(),
            attachments: Vec::new(),
        },
        received_at_ms: 1_000,
        route_meta: None,
        close_immediately: false,
    };

    let events = decide(&state, &Command::Ingress(ingress), &CoreConfig::default());
    assert_eq!(events.len(), 1);
    match &events[0] {
        Event::Ingress(IngressEvent::MessageIgnored { reason, .. }) => {
            assert!(matches!(reason, IngressIgnoreReason::Blacklisted));
        }
        other => panic!("unexpected event: {other:?}"),
    }
}
