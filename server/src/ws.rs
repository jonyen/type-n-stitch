//! Live updates for an open project: `GET /api/projects/:id/ws`. The socket
//! is fan-out and presence only — edits still arrive by `POST …/ops`, which
//! is what keeps one write path. Authorization is the HTTP upgrade itself.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::Response;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::broadcast::error::RecvError;
use uuid::Uuid;

use crate::bus::{PeerInfo, PresenceState, ServerMsg};
use crate::error::AppResult;
use crate::ops::load_doc;
use crate::projects::ProjectAccess;
use crate::AppState;

const PING_EVERY: Duration = Duration::from_secs(30);
/// Close after this many pings go unanswered.
const MISSED_PINGS: u32 = 2;

#[derive(Debug, Deserialize)]
#[serde(tag = "t", rename_all = "lowercase")]
pub enum ClientMsg {
    Presence(PresenceState),
    Ping,
}

/// `GET /api/projects/:id/ws`. The extractor has already answered 401/404/403;
/// what reaches the socket is a member.
pub async fn handler(
    State(state): State<Arc<AppState>>,
    access: ProjectAccess,
    upgrade: WebSocketUpgrade,
) -> AppResult<Response> {
    Ok(upgrade.on_upgrade(move |socket| session(state, access, socket)))
}

fn text(msg: &ServerMsg) -> Message {
    Message::Text(
        serde_json::to_string(msg)
            .expect("ServerMsg serialises")
            .into(),
    )
}

async fn session(state: Arc<AppState>, access: ProjectAccess, socket: WebSocket) {
    let project_id = access.project.id.clone();
    let conn_id = Uuid::new_v4().to_string();
    let me = PeerInfo::from(&access.user);
    let (mut sink, mut stream) = socket.split();

    // Subscribe before reading the doc so nothing published in between is
    // missed. `subscribe` announces our join before creating our receiver, so
    // the queue holds only frames from others.
    let mut sub = state.bus.subscribe(&project_id, &conn_id, me.clone());
    let hello = match load_doc(&state, &project_id).await {
        Ok((head_seq, doc)) => {
            let peers = state.bus.peers(&project_id);
            let you = peers
                .iter()
                .find(|p| p.conn_id == conn_id)
                .cloned()
                .expect("just subscribed");
            ServerMsg::Hello {
                head_seq,
                edits: doc.edits,
                speaker_names: doc.speaker_names,
                peers,
                you,
            }
        }
        Err(e) => ServerMsg::Error {
            code: "load".into(),
            detail: format!("{e:?}"),
        },
    };
    if sink.send(text(&hello)).await.is_err() {
        return;
    }

    let mut ping = tokio::time::interval(PING_EVERY);
    ping.tick().await; // the first tick fires immediately; skip it
    let mut unanswered = 0u32;

    loop {
        tokio::select! {
            incoming = stream.next() => {
                match incoming {
                    Some(Ok(Message::Text(body))) => {
                        match serde_json::from_str::<ClientMsg>(&body) {
                            Ok(ClientMsg::Presence(p)) => {
                                if let Some(peer) = state.bus.update_presence(&project_id, &conn_id, p) {
                                    state.bus.publish(&project_id, ServerMsg::Presence(peer));
                                }
                            }
                            Ok(ClientMsg::Ping) => {
                                if sink.send(text(&ServerMsg::Pong)).await.is_err() { break; }
                            }
                            Err(e) => {
                                let err = ServerMsg::Error { code: "bad_frame".into(), detail: e.to_string() };
                                if sink.send(text(&err)).await.is_err() { break; }
                            }
                        }
                    }
                    Some(Ok(Message::Pong(_))) => unanswered = 0,
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    Some(Ok(_)) => {}
                }
            }
            outgoing = sub.rx.recv() => {
                match outgoing {
                    Ok(msg) => {
                        if sink.send(text(&msg)).await.is_err() { break; }
                    }
                    Err(RecvError::Lagged(_)) => {
                        // Drain whatever is left; the resync supersedes it all.
                        while sub.rx.try_recv().is_ok() {}
                        let resync = match load_doc(&state, &project_id).await {
                            Ok((head_seq, doc)) => ServerMsg::Resync {
                                head_seq,
                                edits: doc.edits,
                                speaker_names: doc.speaker_names,
                            },
                            Err(e) => ServerMsg::Error { code: "load".into(), detail: format!("{e:?}") },
                        };
                        if sink.send(text(&resync)).await.is_err() { break; }
                    }
                    Err(RecvError::Closed) => break,
                }
            }
            _ = ping.tick() => {
                unanswered += 1;
                if unanswered > MISSED_PINGS { break; }
                if sink.send(Message::Ping(Vec::new().into())).await.is_err() { break; }
            }
        }
    }
    // Dropping `sub` removes our presence and announces `left`.
    drop(sub);
    let _ = sink.close().await;
}

#[cfg(test)]
mod tests {
    use axum::http::{Method, StatusCode};
    use futures_util::{SinkExt, StreamExt};
    use serde_json::{json, Value};
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tokio_tungstenite::tungstenite::http::header::COOKIE;
    use tokio_tungstenite::tungstenite::Message;
    use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

    use super::*;
    use crate::projects::create_project;
    use crate::projects::test_support::seed_media;
    use crate::test_util::{app, call, json_req, register, serve, state};

    type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

    async fn connect(base: &str, project: &str, cookie: &str) -> Result<Socket, u16> {
        let url = format!(
            "{}/api/projects/{project}/ws",
            base.replacen("http", "ws", 1)
        );
        let mut req = url.into_client_request().unwrap();
        req.headers_mut().insert(COOKIE, cookie.parse().unwrap());
        match connect_async(req).await {
            Ok((socket, _)) => Ok(socket),
            Err(tokio_tungstenite::tungstenite::Error::Http(resp)) => Err(resp.status().as_u16()),
            Err(e) => panic!("connect: {e}"),
        }
    }

    async fn next_json(socket: &mut Socket) -> Value {
        loop {
            match tokio::time::timeout(Duration::from_secs(3), socket.next())
                .await
                .expect("timed out waiting for a frame")
                .expect("socket closed")
                .unwrap()
            {
                Message::Text(text) => return serde_json::from_str(&text).unwrap(),
                Message::Ping(_) | Message::Pong(_) => continue,
                other => panic!("unexpected frame {other:?}"),
            }
        }
    }

    /// Ada owns a 10 s project; bob has `role` (None = not a member).
    async fn setup(
        role: Option<&str>,
    ) -> (
        Arc<AppState>,
        tempfile::TempDir,
        String,
        String,
        String,
        String,
    ) {
        let (state, dir) = state().await;
        let ada = register(&state, "ada@example.com").await;
        let bob = register(&state, "bob@example.com").await;
        let media = seed_media(&state, 10.0).await;
        let (_, me, _) = call(
            app(&state),
            json_req(Method::GET, "/api/me", Some(&ada), None),
        )
        .await;
        let owner = serde_json::from_value(me).unwrap();
        let project = create_project(&state.db, &owner, &media, "Clip")
            .await
            .unwrap();
        if let Some(role) = role {
            call(
                app(&state),
                json_req(
                    Method::POST,
                    &format!("/api/projects/{}/members", project.id),
                    Some(&ada),
                    Some(json!({ "email": "bob@example.com", "role": role })),
                ),
            )
            .await;
        }
        let base = serve(&state).await;
        (state, dir, base, ada, bob, project.id)
    }

    #[tokio::test]
    async fn upgrade_is_authorized_like_any_project_route() {
        let (_s, _d, base, ada, bob, project) = setup(None).await;
        assert_eq!(connect(&base, &project, "").await.err(), Some(401));
        assert_eq!(connect(&base, "nope", &ada).await.err(), Some(404));
        assert_eq!(connect(&base, &project, &bob).await.err(), Some(403));
        assert!(connect(&base, &project, &ada).await.is_ok());
    }

    #[tokio::test]
    async fn hello_carries_the_doc_and_peers_and_presence_flows() {
        let (_s, _d, base, ada, bob, project) = setup(Some("viewer")).await;
        let mut a = connect(&base, &project, &ada).await.unwrap();
        let hello = next_json(&mut a).await;
        assert_eq!(hello["t"], "hello");
        assert_eq!(hello["headSeq"], 0);
        assert_eq!(hello["you"]["user"]["displayName"], "ada@example.com");
        assert_eq!(hello["peers"].as_array().unwrap().len(), 1); // just ada

        let mut b = connect(&base, &project, &bob).await.unwrap();
        let hello_b = next_json(&mut b).await;
        assert_eq!(hello_b["peers"].as_array().unwrap().len(), 2);
        // Ada learns bob joined.
        let joined = next_json(&mut a).await;
        assert_eq!(joined["t"], "presence");
        assert_eq!(joined["user"]["displayName"], "bob@example.com");

        // Bob moves; ada sees it.
        b.send(Message::Text(
            json!({ "t": "presence", "playhead": 4.5, "selection": [1, 2], "caret": null, "playing": true })
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
        let moved = next_json(&mut a).await;
        assert_eq!(moved["t"], "presence");
        assert_eq!(moved["state"]["playhead"], 4.5);
        assert_eq!(moved["state"]["selection"], json!([1, 2]));

        // Bob leaves; ada sees it.
        drop(b);
        let left = next_json(&mut a).await;
        assert_eq!(left["t"], "left");
    }

    #[tokio::test]
    async fn an_append_over_rest_is_broadcast_as_the_fold_to_everyone() {
        let (state, _d, base, ada, bob, project) = setup(Some("editor")).await;
        let mut a = connect(&base, &project, &ada).await.unwrap();
        next_json(&mut a).await; // hello
        let mut b = connect(&base, &project, &bob).await.unwrap();
        next_json(&mut b).await; // hello
        next_json(&mut a).await; // bob's presence

        let (status, body, _) = call(
            app(&state),
            json_req(
                Method::POST,
                &format!("/api/projects/{project}/ops"),
                Some(&bob),
                Some(json!({ "ops": [{ "opId": "x", "kind": "cut", "start": 1.0, "end": 2.0 }] })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        for socket in [&mut a, &mut b] {
            let doc = next_json(socket).await;
            assert_eq!(doc["t"], "doc");
            assert_eq!(doc["seq"], 1);
            assert_eq!(doc["headSeq"], 1);
            assert_eq!(doc["edits"][0]["start"], 1.0);
            assert!(
                doc.get("undoable").is_none(),
                "per-user fields never ride a broadcast"
            );
        }
    }

    #[tokio::test]
    async fn ping_is_answered_and_a_lagged_client_gets_a_resync() {
        let (state, _d, base, ada, _bob, project) = setup(None).await;
        let mut a = connect(&base, &project, &ada).await.unwrap();
        next_json(&mut a).await;
        a.send(Message::Text(json!({ "t": "ping" }).to_string().into()))
            .await
            .unwrap();
        assert_eq!(next_json(&mut a).await["t"], "pong");

        // Flood the hub past its capacity while the client is not reading.
        for i in 0..600 {
            state.bus.publish(
                &project,
                ServerMsg::Doc {
                    seq: i,
                    author_id: "x".into(),
                    head_seq: i,
                    edits: vec![],
                    speaker_names: vec![],
                },
            );
        }
        // The first thing the client reads after the lag is a resync, not a close.
        let mut saw_resync = false;
        for _ in 0..700 {
            let msg = next_json(&mut a).await;
            if msg["t"] == "resync" {
                saw_resync = true;
                break;
            }
        }
        assert!(saw_resync);
    }
}
