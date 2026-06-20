use std::path::Path;
use std::sync::Arc;
use std::sync::RwLock;
use std::time::{SystemTime, UNIX_EPOCH};

use oqqwall_rust_core::decide::decide;
use oqqwall_rust_core::{ActorId, Command, CoreConfig, Event, EventEnvelope, Id128, StateView};
use oqqwall_rust_infra::{JournalCursor, LocalJournal, Snapshot, SnapshotStore};
use tokio::sync::{broadcast, mpsc};

#[cfg(debug_assertions)]
macro_rules! debug_log {
    ($($arg:tt)*) => {
        oqqwall_rust_infra::debug_log::log(format_args!($($arg)*));
    };
}

#[cfg(not(debug_assertions))]
macro_rules! debug_log {
    ($($arg:tt)*) => {};
}

const SNAPSHOT_EVERY_EVENTS: u64 = 1000;
const SNAPSHOT_EVERY_MS: i64 = 5 * 60 * 1000;

pub struct Engine {
    state: StateView,
    config: CoreConfig,
    cmd_rx: mpsc::Receiver<Command>,
    bus: broadcast::Sender<EventEnvelope>,
    next_event_id: u128,
    actor: ActorId,
    journal: LocalJournal,
    snapshot: SnapshotStore,
    last_cursor: JournalCursor,
    events_since_snapshot: u64,
    last_snapshot_ms: i64,
    shared_state: Arc<RwLock<StateView>>,
}

#[derive(Clone)]
#[allow(dead_code)]
pub struct EngineHandle {
    pub cmd_tx: mpsc::Sender<Command>,
    bus: broadcast::Sender<EventEnvelope>,
    state: Arc<RwLock<StateView>>,
}

impl EngineHandle {
    #[allow(dead_code)]
    pub fn subscribe(&self) -> broadcast::Receiver<EventEnvelope> {
        self.bus.subscribe()
    }

    pub fn state(&self) -> Arc<RwLock<StateView>> {
        self.state.clone()
    }
}

impl Engine {
    pub fn new(
        config: CoreConfig,
        data_dir: impl AsRef<Path>,
    ) -> Result<(Self, EngineHandle), String> {
        let (cmd_tx, cmd_rx) = mpsc::channel(1024);
        let (bus, _) = broadcast::channel(1024);
        let shared_state = Arc::new(RwLock::new(StateView::default()));
        let handle = EngineHandle {
            cmd_tx,
            bus: bus.clone(),
            state: shared_state,
        };
        let mut journal = LocalJournal::open(data_dir.as_ref())
            .map_err(|err| format!("journal init: {}", err))?;
        let snapshot = SnapshotStore::open(data_dir.as_ref())
            .map_err(|err| format!("snapshot init: {}", err))?;
        let (state, last_cursor, last_snapshot_ms) = Self::restore_state(&mut journal, &snapshot)?;
        {
            if let Ok(mut guard) = handle.state.write() {
                *guard = state.clone();
            }
        }
        let next_event_id = state
            .last_event_id
            .map(|id| id.0.saturating_add(1))
            .unwrap_or(1);
        let engine = Self {
            state,
            config,
            cmd_rx,
            bus,
            next_event_id,
            actor: Id128(1),
            journal,
            snapshot,
            last_cursor,
            events_since_snapshot: 0,
            last_snapshot_ms,
            shared_state: handle.state.clone(),
        };
        debug_log!("engine init: groups={}", engine.config.groups.len());
        Ok((engine, handle))
    }

    pub async fn run(mut self) {
        debug_log!("engine loop started");
        while let Some(cmd) = self.cmd_rx.recv().await {
            if !matches!(cmd, Command::Tick(_)) {
                debug_log!("engine cmd: {:?}", cmd);
            }
            let events = decide(&self.state, &cmd, &self.config);
            if !events.is_empty() {
                debug_log!("engine produced {} event(s)", events.len());
                for _event in &events {
                    debug_log!("engine event: {:?}", _event);
                }
            }
            for event in events {
                let env = self.envelope(event);
                let cursor = match self.journal.append(&env) {
                    Ok(cursor) => cursor,
                    Err(_err) => {
                        debug_log!("journal append failed: {}", _err);
                        return;
                    }
                };
                self.last_cursor = cursor;
                self.events_since_snapshot = self.events_since_snapshot.saturating_add(1);
                self.state = self.state.reduce(&env);
                {
                    if let Ok(mut guard) = self.shared_state.write() {
                        *guard = self.state.clone();
                    }
                }
                let send_result = self.bus.send(env);
                if send_result.is_err() {
                    debug_log!("engine event bus send failed");
                }
                self.maybe_snapshot();
            }
        }
        debug_log!("engine loop ended");
    }

    fn restore_state(
        journal: &mut LocalJournal,
        snapshot: &SnapshotStore,
    ) -> Result<(StateView, JournalCursor, i64), String> {
        let mut state = StateView::default();
        let mut cursor = None;
        let mut last_snapshot_ms = 0;

        match snapshot.load() {
            Ok(Some(loaded)) => {
                last_snapshot_ms = loaded.taken_at_ms;
                cursor = loaded.journal_cursor;
                state = loaded.state;
                debug_log!("snapshot loaded: last_event_id={:?}", state.last_event_id);
            }
            Ok(None) => {
                debug_log!("snapshot missing");
            }
            Err(_err) => {
                debug_log!("snapshot load failed: {}", _err);
            }
        }

        let replay = journal.replay(cursor, |env| {
            state = state.reduce(env);
        });
        let outcome = match replay {
            Ok(outcome) => outcome,
            Err(_err) if cursor.is_some() => {
                debug_log!("journal replay from snapshot cursor failed: {}", _err);
                state = StateView::default();
                last_snapshot_ms = 0;
                journal
                    .replay(None, |env| {
                        state = state.reduce(env);
                    })
                    .map_err(|err| format!("journal replay failed: {}", err))?
            }
            Err(_err) => return Err(format!("journal replay failed: {}", _err)),
        };

        if let Some(_corruption) = &outcome.corruption {
            debug_log!(
                "journal corruption at segment {} offset {}: {}",
                _corruption.segment,
                _corruption.offset,
                _corruption.reason
            );
            journal
                .truncate_tail(outcome.last_cursor)
                .map_err(|err| format!("journal repair failed: {}", err))?;
        }

        debug_log!("journal replayed {} event(s)", outcome.events);
        let last_cursor = outcome.last_cursor;
        let last_snapshot_ms = if last_snapshot_ms == 0 {
            now_ms()
        } else {
            last_snapshot_ms
        };
        Ok((state, last_cursor, last_snapshot_ms))
    }

    fn envelope(&mut self, event: Event) -> EventEnvelope {
        let env = EventEnvelope {
            id: Id128(self.next_event_id),
            ts_ms: now_ms(),
            actor: self.actor,
            correlation_id: None,
            event,
        };
        self.next_event_id = self.next_event_id.saturating_add(1);
        env
    }

    fn maybe_snapshot(&mut self) {
        let now = now_ms();
        if self.events_since_snapshot < SNAPSHOT_EVERY_EVENTS
            && now.saturating_sub(self.last_snapshot_ms) < SNAPSHOT_EVERY_MS
        {
            return;
        }
        let snapshot = Snapshot::new(now, Some(self.last_cursor), self.state.clone());
        match self.snapshot.write(&snapshot) {
            Ok(()) => {
                self.last_snapshot_ms = now;
                self.events_since_snapshot = 0;
                debug_log!("snapshot saved: ts_ms={}", now);
            }
            Err(_err) => {
                debug_log!("snapshot write failed: {}", _err);
            }
        }
    }
}

fn now_ms() -> i64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    now.as_millis() as i64
}
