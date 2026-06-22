use oqqwall_rust_core::{GlobalAction, ReviewAction, ReviewCode, ShortcutScope};

pub const RAW_BUILTIN_PREFIX: &str = "原始";

const REVIEW_DEFER_MS: i64 = 180_000;

#[derive(Debug, Clone)]
pub struct ShortcutTemplateContext<'a> {
    pub args: &'a str,
    pub review_code: Option<ReviewCode>,
    pub sender_id: Option<&'a str>,
    pub group_id: &'a str,
}

pub fn validate_shortcut_name(name: &str) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("快捷指令名不能为空".to_string());
    }
    if trimmed == RAW_BUILTIN_PREFIX {
        return Err(format!("快捷指令名不能为 {}", RAW_BUILTIN_PREFIX));
    }
    if trimmed.chars().any(char::is_whitespace) {
        return Err("快捷指令名不能包含空白字符".to_string());
    }
    Ok(trimmed.to_string())
}

pub fn shortcut_scope_label(scope: ShortcutScope) -> &'static str {
    match scope {
        ShortcutScope::Review => "审核",
        ShortcutScope::Global => "全局",
    }
}

pub fn shortcut_field_name(scope: ShortcutScope) -> &'static str {
    match scope {
        ShortcutScope::Review => "review_shortcuts",
        ShortcutScope::Global => "global_shortcuts",
    }
}

pub fn parse_shortcut_scope_token(token: &str) -> Option<ShortcutScope> {
    match token.trim() {
        "审核" | "review" => Some(ShortcutScope::Review),
        "全局" | "global" => Some(ShortcutScope::Global),
        _ => None,
    }
}

pub fn is_builtin_review_command_name(name: &str) -> bool {
    matches!(
        name.trim(),
        "是" | "否"
            | "等"
            | "删"
            | "拒"
            | "立即"
            | "刷新"
            | "重渲染"
            | "消息全选"
            | "匿"
            | "扩列审查"
            | "扩列"
            | "查"
            | "查成分"
            | "展示"
            | "评论"
            | "回复"
            | "合并"
            | "拉黑"
    )
}

pub fn is_builtin_global_command_name(name: &str) -> bool {
    matches!(
        name.trim(),
        "帮助"
            | "调出"
            | "撤回"
            | "信息"
            | "手动重新登录"
            | "自动重新登录"
            | "待处理"
            | "删除待处理"
            | "删除暂存区"
            | "发送暂存区"
            | "清理发送中"
            | "列出拉黑"
            | "取消拉黑"
            | "设定编号"
            | "快捷回复"
            | "快捷指令"
            | "自检"
            | "系统修复"
    )
}

pub fn split_shortcut_definition(definition: &str) -> Result<Vec<String>, String> {
    let normalized = definition.replace("\r\n", "\n");
    let mut out = Vec::new();
    let mut current = String::new();
    for ch in normalized.chars() {
        if ch == '|' || ch == '\n' {
            push_shortcut_step(&mut out, &mut current)?;
            continue;
        }
        current.push(ch);
    }
    push_shortcut_step(&mut out, &mut current)?;
    if out.is_empty() {
        return Err("快捷指令至少需要一个步骤".to_string());
    }
    Ok(out)
}

fn push_shortcut_step(out: &mut Vec<String>, current: &mut String) -> Result<(), String> {
    let trimmed = current.trim();
    if trimmed.is_empty() {
        if !current.is_empty() {
            current.clear();
            return Err("快捷指令包含空步骤".to_string());
        }
        current.clear();
        return Ok(());
    }
    out.push(trimmed.to_string());
    current.clear();
    Ok(())
}

pub fn validate_review_shortcut_definition(definition: &str) -> Result<(), String> {
    let ctx = ShortcutTemplateContext {
        args: "1",
        review_code: Some(1),
        sender_id: Some("10001"),
        group_id: "group",
    };
    parse_review_shortcut_actions(definition, &ctx).map(|_| ())
}

pub fn validate_global_shortcut_definition(definition: &str) -> Result<(), String> {
    let ctx = ShortcutTemplateContext {
        args: "1",
        review_code: None,
        sender_id: None,
        group_id: "group",
    };
    parse_global_shortcut_actions(definition, &ctx).map(|_| ())
}

pub fn parse_review_shortcut_actions(
    definition: &str,
    ctx: &ShortcutTemplateContext<'_>,
) -> Result<Vec<ReviewAction>, String> {
    split_shortcut_definition(definition)?
        .into_iter()
        .map(|step| {
            let rendered = render_shortcut_step(ShortcutScope::Review, &step, ctx)?;
            parse_builtin_review_shortcut_step(&rendered)
                .ok_or_else(|| format!("无效的审核快捷指令步骤：{}", step))
        })
        .collect()
}

pub fn parse_global_shortcut_actions(
    definition: &str,
    ctx: &ShortcutTemplateContext<'_>,
) -> Result<Vec<GlobalAction>, String> {
    split_shortcut_definition(definition)?
        .into_iter()
        .map(|step| {
            let rendered = render_shortcut_step(ShortcutScope::Global, &step, ctx)?;
            parse_builtin_global_shortcut_step(&rendered)
                .ok_or_else(|| format!("无效的全局快捷指令步骤：{}", step))
        })
        .collect()
}

fn render_shortcut_step(
    scope: ShortcutScope,
    step: &str,
    ctx: &ShortcutTemplateContext<'_>,
) -> Result<String, String> {
    let mut out = String::with_capacity(step.len());
    let mut idx = 0usize;
    while idx < step.len() {
        let Some(ch) = step[idx..].chars().next() else {
            break;
        };
        if ch != '{' {
            out.push(ch);
            idx += ch.len_utf8();
            continue;
        }
        let start = idx + ch.len_utf8();
        let Some(close_rel) = step[start..].find('}') else {
            return Err("快捷指令占位符缺少 '}'".to_string());
        };
        let close = start + close_rel;
        let name = step[start..close].trim();
        let replacement = match (scope, name) {
            (_, "args") => ctx.args.to_string(),
            (_, "group_id") => ctx.group_id.to_string(),
            (ShortcutScope::Review, "review_code") => ctx
                .review_code
                .map(|value| value.to_string())
                .ok_or_else(|| "当前上下文不支持 {review_code}".to_string())?,
            (ShortcutScope::Review, "sender_id") => ctx
                .sender_id
                .map(str::to_string)
                .ok_or_else(|| "当前上下文不支持 {sender_id}".to_string())?,
            (ShortcutScope::Global, "review_code" | "sender_id") => {
                return Err(format!("全局快捷指令不支持占位符 {{{}}}", name));
            }
            _ => return Err(format!("未知占位符 {{{}}}", name)),
        };
        out.push_str(&replacement);
        idx = close + 1;
    }
    Ok(out)
}

pub fn parse_builtin_review_action(
    command: &str,
    rest: &str,
    allow_quick_reply: bool,
) -> Option<ReviewAction> {
    let rest = rest.trim();
    let action = match command {
        "是" => ReviewAction::Approve,
        "否" => ReviewAction::Skip,
        "等" => ReviewAction::Defer {
            delay_ms: REVIEW_DEFER_MS,
        },
        "删" => ReviewAction::Delete,
        "拒" => ReviewAction::Reject,
        "立即" => ReviewAction::Immediate,
        "刷新" => ReviewAction::Refresh,
        "重渲染" => ReviewAction::Rerender,
        "消息全选" => ReviewAction::SelectAllMessages,
        "匿" => ReviewAction::ToggleAnonymous,
        "扩列审查" | "扩列" | "查" | "查成分" => ReviewAction::ExpandAudit,
        "展示" => ReviewAction::Show,
        "评论" => ReviewAction::Comment {
            text: rest.to_string(),
        },
        "回复" => ReviewAction::Reply {
            text: rest.to_string(),
        },
        "合并" => {
            let target = rest.split_whitespace().next()?;
            let review_code = target.parse::<ReviewCode>().ok()?;
            ReviewAction::Merge { review_code }
        }
        "拉黑" => ReviewAction::Blacklist {
            reason: if rest.is_empty() {
                None
            } else {
                Some(rest.to_string())
            },
        },
        _ => {
            if allow_quick_reply {
                ReviewAction::QuickReply {
                    key: command.to_string(),
                }
            } else {
                return None;
            }
        }
    };
    Some(action)
}

pub fn parse_builtin_global_action(command: &str, rest: &str) -> Option<GlobalAction> {
    if command.eq_ignore_ascii_case("help") {
        return Some(GlobalAction::Help);
    }
    let rest = rest.trim();
    match command {
        "帮助" => Some(GlobalAction::Help),
        "调出" => parse_review_code(rest).map(|review_code| GlobalAction::Recall { review_code }),
        "撤回" => {
            parse_review_code(rest).map(|review_code| GlobalAction::Withdraw { review_code })
        }
        "信息" => parse_review_code(rest).map(|review_code| GlobalAction::Info { review_code }),
        "手动重新登录" => Some(GlobalAction::ManualRelogin),
        "自动重新登录" => Some(GlobalAction::AutoRelogin),
        "待处理" => Some(GlobalAction::PendingList),
        "删除待处理" => Some(GlobalAction::PendingClear),
        "删除暂存区" => Some(GlobalAction::SendQueueClear),
        "发送暂存区" => Some(GlobalAction::SendQueueFlush),
        "清理发送中" => Some(GlobalAction::SendInFlightClear),
        "列出拉黑" => Some(GlobalAction::BlacklistList),
        "取消拉黑" => {
            parse_first_token(rest).map(|sender_id| GlobalAction::BlacklistRemove { sender_id })
        }
        "设定编号" => parse_u64(rest).map(|value| GlobalAction::SetExternalNumber { value }),
        "快捷回复" => parse_quick_reply_action(rest),
        "快捷指令" => parse_shortcut_action(rest),
        "自检" => Some(GlobalAction::SelfCheck),
        "系统修复" => Some(GlobalAction::SystemRepair),
        _ => None,
    }
}

fn parse_builtin_review_shortcut_step(step: &str) -> Option<ReviewAction> {
    let (command, rest) = split_first_token_with_rest(step)?;
    parse_builtin_review_action(command, rest.trim_start(), false)
}

fn parse_builtin_global_shortcut_step(step: &str) -> Option<GlobalAction> {
    let (command, rest) = split_first_token_with_rest(step)?;
    let action = parse_builtin_global_action(command, rest)?;
    matches!(
        action,
        GlobalAction::Recall { .. }
            | GlobalAction::Withdraw { .. }
            | GlobalAction::PendingClear
            | GlobalAction::SendQueueClear
            | GlobalAction::SendQueueFlush
            | GlobalAction::SendInFlightClear
            | GlobalAction::BlacklistRemove { .. }
            | GlobalAction::SetExternalNumber { .. }
    )
    .then_some(action)
}

fn parse_quick_reply_action(rest: &str) -> Option<GlobalAction> {
    let mut tokens = rest.split_whitespace();
    let sub = tokens.next();
    match sub {
        None => Some(GlobalAction::QuickReplyList),
        Some("添加") => {
            let payload = tokens.collect::<Vec<_>>().join(" ");
            let (key, text) = payload.split_once('=')?;
            let key = key.trim();
            let text = text.trim();
            if key.is_empty() || text.is_empty() {
                return None;
            }
            Some(GlobalAction::QuickReplyAdd {
                key: key.to_string(),
                text: text.to_string(),
            })
        }
        Some("删除") => {
            let key = tokens.collect::<Vec<_>>().join(" ");
            let key = key.trim();
            if key.is_empty() {
                return None;
            }
            Some(GlobalAction::QuickReplyDelete {
                key: key.to_string(),
            })
        }
        _ => None,
    }
}

fn parse_shortcut_action(rest: &str) -> Option<GlobalAction> {
    let rest = rest.trim();
    if rest.is_empty() {
        return Some(GlobalAction::ShortcutList);
    }
    let (sub, rest) = split_first_token_with_rest(rest)?;
    match sub {
        "添加" => {
            let (scope_token, payload) = split_first_token_with_rest(rest)?;
            let scope = parse_shortcut_scope_token(scope_token)?;
            let (key, definition) = payload.split_once('=')?;
            let key = validate_shortcut_name(key).ok()?;
            let definition = definition.trim();
            if definition.is_empty() {
                return None;
            }
            Some(GlobalAction::ShortcutAdd {
                scope,
                key,
                definition: definition.to_string(),
            })
        }
        "删除" => {
            let (scope_token, key) = split_first_token_with_rest(rest)?;
            let scope = parse_shortcut_scope_token(scope_token)?;
            let key = validate_shortcut_name(key).ok()?;
            Some(GlobalAction::ShortcutDelete { scope, key })
        }
        _ => None,
    }
}

fn split_first_token_with_rest(input: &str) -> Option<(&str, &str)> {
    let input = input.trim_start();
    if input.is_empty() {
        return None;
    }
    let mut iter = input.splitn(2, char::is_whitespace);
    let first = iter.next().unwrap_or("");
    let rest = iter.next().unwrap_or("");
    if first.is_empty() {
        None
    } else {
        Some((first, rest))
    }
}

fn parse_review_code(text: &str) -> Option<ReviewCode> {
    let token = text.split_whitespace().next()?;
    let trimmed = token.strip_prefix('#').unwrap_or(token);
    trimmed.parse::<ReviewCode>().ok()
}

fn parse_first_token(text: &str) -> Option<String> {
    Some(text.split_whitespace().next()?.to_string())
}

fn parse_u64(text: &str) -> Option<u64> {
    text.split_whitespace().next()?.parse::<u64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_shortcut_definition_supports_bar_and_newline() {
        let steps = split_shortcut_definition("匿 | 是\n拒").expect("steps");
        assert_eq!(steps, vec!["匿", "是", "拒"]);
    }

    #[test]
    fn review_shortcut_supports_system_placeholders() {
        let actions = parse_review_shortcut_actions(
            "回复 投稿人 {sender_id} #{review_code} {args} | 拉黑 {group_id}",
            &ShortcutTemplateContext {
                args: "请重发",
                review_code: Some(42),
                sender_id: Some("10001"),
                group_id: "wall-a",
            },
        )
        .expect("actions");
        assert_eq!(
            actions,
            vec![
                ReviewAction::Reply {
                    text: "投稿人 10001 #42 请重发".to_string(),
                },
                ReviewAction::Blacklist {
                    reason: Some("wall-a".to_string()),
                }
            ]
        );
    }

    #[test]
    fn global_shortcut_rejects_review_only_placeholders() {
        let err = parse_global_shortcut_actions(
            "撤回 {review_code}",
            &ShortcutTemplateContext {
                args: "",
                review_code: None,
                sender_id: None,
                group_id: "wall-a",
            },
        )
        .expect_err("should fail");
        assert!(err.contains("{review_code}"));
    }
}
