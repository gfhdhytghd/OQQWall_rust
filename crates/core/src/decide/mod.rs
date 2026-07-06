mod driver;
mod flush;
mod global;
mod ingress;
mod post_action;
mod review;
mod scheduler;
mod sender;
mod tick;

pub mod builder;

use crate::command::Command;
use crate::config::CoreConfig;
use crate::event::Event;
use crate::state::StateView;

pub fn decide(state: &StateView, command: &Command, config: &CoreConfig) -> Vec<Event> {
    match command {
        Command::Ingress(cmd) => ingress::decide_ingress(state, cmd, config),
        Command::Tick(cmd) => tick::decide_tick(state, cmd, config),
        Command::ReviewAction(cmd) => review::decide_review_action(state, cmd, config),
        Command::ReviewActionBatch(cmd) => review::decide_review_action_batch(state, cmd, config),
        Command::GlobalAction(cmd) => global::decide_global_action(state, cmd, config),
        Command::GlobalActionBatch(cmd) => global::decide_global_action_batch(state, cmd, config),
        Command::PostAction(cmd) => post_action::decide_post_action(state, cmd),
        Command::DriverEvent(event) => driver::decide_driver_event(state, event, config),
    }
}
