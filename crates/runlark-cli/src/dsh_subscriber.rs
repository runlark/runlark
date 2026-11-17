use crate::dsh_events::{DshWireEvent, process_dsh_wire_event};
use crate::state::AppState;
use futures_util::{SinkExt, StreamExt};
use reqwest::Client;
use reqwest::header::{COOKIE, ORIGIN};
use reqwest::redirect::Policy;
use runlark_domain::{SessionId, SessionStatus, Timestamp};
use runlark_local::{SessionStore, open_sqlite};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::{Notify, watch};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum MuxServerMessage {
    #[serde(rename = "item")]
    Item {
        #[serde(rename = "streamId")]
        stream_id: String,
        value: Option<FollowFrame>,
    },
    #[serde(rename = "end")]
    End {
        #[serde(rename = "streamId")]
        stream_id: String,
    },
    #[serde(rename = "error")]
    Error {
        #[serde(rename = "streamId")]
        stream_id: String,
        error: Value,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum FollowFrame {
    #[serde(rename = "snapshot")]
    Snapshot {
        #[serde(default)]
        records: Vec<HistoryRecord>,
    },
    #[serde(rename = "event")]
    Event { event: DshWireEvent },
    #[serde(rename = "assistant-stream")]
    AssistantStream {},
}

#[derive(Debug, Deserialize)]
struct HistoryRecord {
    event: DshWireEvent,
}

/// Parsed DSH base origin, WebSocket mux URL, and optional launch token.
#[derive(Debug, Clone)]
pub struct DshEndpoint {
    pub origin: String,
    pub ws_mux_url: String,
    pub token: Option<String>,
}

impl DshEndpoint {
    /// Parses a DSH web URL (e.g. `http://127.0.0.1:4321/?token=...`) into HTTP origin,
    /// `/api/remote.mux` WebSocket URL, and launch token.
    #[must_use]
    pub fn parse(raw_url: &str, token_override: Option<String>) -> Self {
        let trimmed = raw_url.trim();
        let (base_part, query_part) = trimmed.split_once('?').unwrap_or((trimmed, ""));
        let origin = base_part.trim_end_matches('/').to_string();

        let token = token_override.or_else(|| {
            query_part.split('&').find_map(|pair| {
                let (k, v) = pair.split_once('=')?;
                (k == "token" && !v.is_empty()).then(|| v.to_string())
            })
        });

        let ws_origin = if let Some(rest) = origin.strip_prefix("https://") {
            format!("wss://{rest}")
        } else if let Some(rest) = origin.strip_prefix("http://") {
            format!("ws://{rest}")
        } else {
            origin.clone()
        };

        Self {
            ws_mux_url: format!("{ws_origin}/api/remote.mux"),
            origin,
            token,
        }
    }
}

/// Subscribes the Runlark daemon to DSH `session/follow` streams over `/api/remote.mux`,
/// reconciling lifecycle events across independent DSH/daemon restarts without duplicates.
pub struct DshSubscriber {
    state: AppState,
    endpoint: DshEndpoint,
    http: Client,
    poll_interval: Duration,
}

/// Authenticates against a DSH host `/connect?token=...` endpoint and returns the resulting
/// session `Cookie` header value, if any.
pub(crate) async fn authenticate_dsh_cookie(
    http: &Client,
    origin: &str,
    token: Option<&str>,
) -> Result<Option<String>, String> {
    let Some(token) = token.filter(|t| !t.trim().is_empty()) else {
        return Ok(None);
    };
    let connect_url = format!("{origin}/connect?token={token}&interactions=none");
    let resp = http
        .get(&connect_url)
        .send()
        .await
        .map_err(|e| format!("DSH /connect request failed: {e}"))?;

    if !resp.status().is_redirection() && !resp.status().is_success() {
        return Err(format!("DSH /connect rejected with {}", resp.status()));
    }

    let cookie = resp
        .headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .filter_map(|val| val.to_str().ok())
        .filter_map(|raw| raw.split(';').next())
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ");

    Ok((!cookie.is_empty()).then_some(cookie))
}

impl DshSubscriber {
    /// Creates a new [`DshSubscriber`] targeting `dsh_url`.
    ///
    /// # Panics
    /// Panics only if the HTTP client backend fails to initialize.
    #[must_use]
    pub fn new(state: AppState, dsh_url: &str, token: Option<String>) -> Self {
        let http = Client::builder()
            .redirect(Policy::none())
            .timeout(Duration::from_secs(10))
            .build()
            .expect("reqwest client must build");
        Self {
            state,
            endpoint: DshEndpoint::parse(dsh_url, token),
            http,
            poll_interval: Duration::from_secs(2),
        }
    }

    /// Runs the resilient DSH WebSocket subscription loop until `shutdown` signals `true`.
    pub async fn run_loop(
        self,
        notify: std::sync::Arc<Notify>,
        mut shutdown: watch::Receiver<bool>,
    ) {
        let mut backoff = Duration::from_millis(250);
        loop {
            if *shutdown.borrow() {
                break;
            }

            match self.run_connection(&notify, &mut shutdown).await {
                Ok(()) => break,
                Err(err) => {
                    tracing::debug!(
                        "DSH subscriber disconnected ({err}); reconnecting in {backoff:?}"
                    );
                    tokio::select! {
                        () = tokio::time::sleep(backoff) => {}
                        changed = shutdown.changed() => {
                            if changed.is_err() || *shutdown.borrow() {
                                break;
                            }
                        }
                    }
                    backoff = (backoff * 2).min(Duration::from_secs(5));
                }
            }
        }
    }

    async fn authenticate_cookie(&self) -> Result<Option<String>, String> {
        authenticate_dsh_cookie(
            &self.http,
            &self.endpoint.origin,
            self.endpoint.token.as_deref(),
        )
        .await
    }

    async fn run_connection(
        &self,
        notify: &Notify,
        shutdown: &mut watch::Receiver<bool>,
    ) -> Result<(), String> {
        let cookie = self.authenticate_cookie().await?;
        let mut request = self
            .endpoint
            .ws_mux_url
            .as_str()
            .into_client_request()
            .map_err(|e| format!("invalid WS URL: {e}"))?;

        request.headers_mut().insert(
            ORIGIN,
            self.endpoint
                .origin
                .parse()
                .map_err(|e| format!("invalid origin header: {e}"))?,
        );
        if let Some(ref c) = cookie {
            request.headers_mut().insert(
                COOKIE,
                c.parse()
                    .map_err(|e| format!("invalid cookie header: {e}"))?,
            );
        }

        let (ws_stream, _) = connect_async(request)
            .await
            .map_err(|e| format!("WS connect failed: {e}"))?;
        let (mut write, mut read) = ws_stream.split();

        let mut active_streams: HashMap<String, SessionId> = HashMap::new();
        let mut subscribed_sessions: HashMap<SessionId, String> = HashMap::new();
        let mut stream_counter: u64 = 0;

        self.sync_subscriptions(
            &mut write,
            &mut active_streams,
            &mut subscribed_sessions,
            &mut stream_counter,
        )
        .await?;

        let mut poll_ticker = tokio::time::interval(self.poll_interval);
        poll_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        poll_ticker.tick().await;

        loop {
            if *shutdown.borrow() {
                return Ok(());
            }

            tokio::select! {
                () = notify.notified() => {
                    self.sync_subscriptions(
                        &mut write,
                        &mut active_streams,
                        &mut subscribed_sessions,
                        &mut stream_counter,
                    ).await?;
                }
                _ = poll_ticker.tick() => {
                    self.sync_subscriptions(
                        &mut write,
                        &mut active_streams,
                        &mut subscribed_sessions,
                        &mut stream_counter,
                    ).await?;
                }
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        return Ok(());
                    }
                }
                frame = read.next() => {
                    match frame {
                        Some(Ok(Message::Text(text))) => {
                            self.handle_mux_text(
                                &text,
                                &mut active_streams,
                                &mut subscribed_sessions,
                            );
                        }
                        Some(Ok(Message::Ping(payload))) => {
                            write
                                .send(Message::Pong(payload))
                                .await
                                .map_err(|e| format!("WS pong failed: {e}"))?;
                        }
                        Some(Ok(Message::Close(_))) | None => {
                            return Err("DSH remote.mux stream closed".to_string());
                        }
                        Some(Ok(_)) => {}
                        Some(Err(e)) => {
                            return Err(format!("WS read error: {e}"));
                        }
                    }
                }
            }
        }
    }

    async fn sync_subscriptions<S>(
        &self,
        write: &mut S,
        active_streams: &mut HashMap<String, SessionId>,
        subscribed_sessions: &mut HashMap<SessionId, String>,
        stream_counter: &mut u64,
    ) -> Result<(), String>
    where
        S: futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
    {
        let sessions = {
            let conn = open_sqlite(&self.state.db_path).map_err(|e| e.to_string())?;
            SessionStore::list_sessions(&conn).map_err(|e| e.to_string())?
        };

        for session in sessions {
            if session.status == SessionStatus::Archived {
                if let Some(stream_id) = subscribed_sessions.remove(&session.id) {
                    active_streams.remove(&stream_id);
                    let cancel = json!({ "type": "cancel", "streamId": stream_id });
                    let _ = write.send(Message::Text(cancel.to_string().into())).await;
                }
                continue;
            }
            if subscribed_sessions.contains_key(&session.id) {
                continue;
            }
            *stream_counter += 1;
            let stream_id = format!("runlark_follow_{}", *stream_counter);
            let open_msg = json!({
                "type": "open",
                "streamId": stream_id,
                "endpoint": "session/follow",
                "payload": {
                    "args": {
                        "request": {
                            "address": { "kind": "session", "sessionId": session.id.0 },
                            "activate": false
                        }
                    }
                }
            });
            write
                .send(Message::Text(open_msg.to_string().into()))
                .await
                .map_err(|e| format!("failed to open session/follow stream: {e}"))?;
            active_streams.insert(stream_id.clone(), session.id.clone());
            subscribed_sessions.insert(session.id, stream_id);
        }

        Ok(())
    }

    fn handle_mux_text(
        &self,
        text: &str,
        active_streams: &mut HashMap<String, SessionId>,
        subscribed_sessions: &mut HashMap<SessionId, String>,
    ) {
        let Ok(msg) = serde_json::from_str::<MuxServerMessage>(text) else {
            return;
        };

        match msg {
            MuxServerMessage::Item { stream_id, value } => {
                let Some(session_id) = active_streams.get(&stream_id).cloned() else {
                    return;
                };
                let Some(frame) = value else {
                    return;
                };
                match frame {
                    FollowFrame::Snapshot { records } => {
                        self.apply_wire_batch(
                            &session_id,
                            records.iter().map(|record| &record.event),
                        );
                    }
                    FollowFrame::Event { event } => {
                        self.apply_wire_batch(&session_id, std::iter::once(&event));
                    }
                    FollowFrame::AssistantStream {} => {}
                }
            }
            MuxServerMessage::End { stream_id } => {
                if let Some(session_id) = active_streams.remove(&stream_id) {
                    subscribed_sessions.remove(&session_id);
                }
            }
            MuxServerMessage::Error { stream_id, error } => {
                if let Some(session_id) = active_streams.remove(&stream_id) {
                    subscribed_sessions.remove(&session_id);
                    tracing::debug!("DSH session/follow stream error for {session_id}: {error}");
                }
            }
        }
    }

    fn apply_wire_batch<'a>(
        &self,
        session_id: &SessionId,
        events: impl IntoIterator<Item = &'a DshWireEvent>,
    ) {
        let mut conn = match open_sqlite(&self.state.db_path) {
            Ok(conn) => conn,
            Err(err) => {
                tracing::debug!("Failed to open SQLite for wire events on {session_id}: {err}");
                return;
            }
        };
        for event in events {
            let now = Timestamp(AppState::current_timestamp_ms());
            if let Err(err) =
                process_dsh_wire_event(&mut conn, &self.state, session_id, None, event, now)
            {
                tracing::debug!(
                    "Ignoring/failed wire event {} for {}: {err}",
                    event.event_type,
                    session_id
                );
            }
        }
    }
}
