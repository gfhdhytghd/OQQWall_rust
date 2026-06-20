use oqqwall_rust_core::event::{DraftEvent, Event, IngressEvent};
use tokio::sync::broadcast::error::RecvError;
use tokio::task::JoinHandle;

use crate::engine::EngineHandle;

pub fn spawn_status_logger(handle: &EngineHandle) -> JoinHandle<()> {
    let mut rx = handle.subscribe();
    tokio::spawn(async move {
        loop {
            let env = match rx.recv().await {
                Ok(env) => env,
                Err(RecvError::Closed) => break,
                Err(RecvError::Lagged(_)) => continue,
            };

            match env.event {
                Event::Ingress(IngressEvent::MessageAccepted {
                    group_id,
                    user_id,
                    message,
                    ..
                }) => {
                    let attachment_count = message.attachments.len();
                    let preview = preview_text(&message.text, 80);
                    let text = if preview.is_empty() {
                        "(无文本)".to_string()
                    } else {
                        preview
                    };
                    println!(
                        "收到新消息 group={} user={} attachments={} text={}",
                        group_id, user_id, attachment_count, text
                    );
                }
                Event::Ingress(IngressEvent::MessageSynced { .. }) => {}
                Event::Draft(DraftEvent::PostDraftCreated {
                    post_id,
                    group_id,
                    ingress_ids,
                    ..
                }) => {
                    println!(
                        "创建新投稿 post_id={} group={} 消息数={}",
                        post_id.0,
                        group_id,
                        ingress_ids.len()
                    );
                }
                _ => {}
            }
        }
    })
}

fn preview_text(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut out = String::new();
    for (idx, ch) in text.chars().enumerate() {
        if idx >= max_chars {
            break;
        }
        out.push(ch);
    }
    out.push_str("...");
    out
}
