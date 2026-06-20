use crate::config::GroupConfig;
use crate::ids::{AccountId, TimestampMs};
use crate::state::{AccountRuntime, StateView};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountChoice {
    Available(AccountId),
    RetryAt(TimestampMs),
    Unavailable,
}

pub fn choose_account(
    state: &StateView,
    group_config: Option<&GroupConfig>,
    now_ms: TimestampMs,
) -> AccountChoice {
    let Some(group_config) = group_config else {
        return AccountChoice::Unavailable;
    };
    if group_config.accounts.is_empty() {
        return AccountChoice::Unavailable;
    }

    let mut best: Option<(AccountId, Option<TimestampMs>)> = None;
    let mut earliest_cooldown: Option<TimestampMs> = None;

    for account_id in &group_config.accounts {
        let runtime = state.accounts.get(account_id);
        if !is_enabled(runtime) {
            continue;
        }
        if let Some(cooldown_until) = cooldown_until(runtime) {
            if cooldown_until > now_ms {
                earliest_cooldown = Some(match earliest_cooldown {
                    Some(current) => current.min(cooldown_until),
                    None => cooldown_until,
                });
                continue;
            }
        }
        let last_send = runtime.and_then(|r| r.last_send_ms);
        match &best {
            Some((_, best_last)) => {
                if last_send.is_none() || best_last.is_some_and(|b| last_send < Some(b)) {
                    best = Some((account_id.clone(), last_send));
                }
            }
            None => best = Some((account_id.clone(), last_send)),
        }
    }

    if let Some((account_id, _)) = best {
        return AccountChoice::Available(account_id);
    }

    if let Some(cooldown) = earliest_cooldown {
        return AccountChoice::RetryAt(cooldown);
    }

    AccountChoice::Unavailable
}

fn is_enabled(runtime: Option<&AccountRuntime>) -> bool {
    runtime.map_or(true, |r| r.enabled)
}

fn cooldown_until(runtime: Option<&AccountRuntime>) -> Option<TimestampMs> {
    runtime.and_then(|r| r.cooldown_until_ms)
}

#[allow(dead_code)]
pub fn backoff_delay_ms(
    base_ms: TimestampMs,
    attempt: u32,
    max_delay_ms: TimestampMs,
) -> TimestampMs {
    if attempt == 0 {
        return base_ms.min(max_delay_ms);
    }
    let shift = attempt.saturating_sub(1).min(60);
    let delay = base_ms.saturating_mul(1i64 << shift);
    delay.min(max_delay_ms)
}
