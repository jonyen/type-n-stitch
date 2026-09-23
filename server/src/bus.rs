//! Fan-out for a project's live events. `Bus` is the seam: the in-process
//! `LocalBus` keeps one hub per project while anyone is subscribed; a
//! multi-instance implementation (Postgres LISTEN/NOTIFY) would replace it
//! without touching call sites. Presence lives here, never in SQLite.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};

use engine::{Edit, Source, Transition};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::auth::User;

/// Who a peer is, as the client renders it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerInfo {
    pub id: String,
    pub display_name: String,
    pub color: String,
    #[serde(default)]
    pub bot: bool,
}

impl From<&User> for PeerInfo {
    fn from(u: &User) -> Self {
        PeerInfo {
            id: u.id.clone(),
            display_name: u.display_name.clone(),
            color: u.color.clone(),
            bot: u.is_bot(),
        }
    }
}

/// Where a peer is looking. All fields are in source time / word indices.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresenceState {
    pub playhead: f64,
    pub selection: Option<[usize; 2]>,
    pub caret: Option<usize>,
    pub playing: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Peer {
    pub conn_id: String,
    pub user: PeerInfo,
    pub state: PresenceState,
}

/// Everything the server pushes down a socket.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "lowercase")]
pub enum ServerMsg {
    #[serde(rename_all = "camelCase")]
    Hello {
        head_seq: i64,
        edits: Vec<Edit>,
        speaker_names: Vec<String>,
        transition: Transition,
        #[serde(default)]
        splits: Vec<f64>,
        #[serde(default)]
        order: Vec<f64>,
        /// Sources appended after the project's own media (the fold's `doc.sources`).
        #[serde(default)]
        sources: Vec<Source>,
        peers: Vec<Peer>,
        you: Peer,
    },
    /// The fold after an append. Sent to everyone, sender included.
    #[serde(rename_all = "camelCase")]
    Doc {
        seq: i64,
        author_id: String,
        head_seq: i64,
        edits: Vec<Edit>,
        speaker_names: Vec<String>,
        transition: Transition,
        #[serde(default)]
        splits: Vec<f64>,
        #[serde(default)]
        order: Vec<f64>,
        /// Sources appended after the project's own media (the fold's `doc.sources`).
        #[serde(default)]
        sources: Vec<Source>,
    },
    Presence(Peer),
    #[serde(rename_all = "camelCase")]
    Left {
        conn_id: String,
    },
    #[serde(rename_all = "camelCase")]
    Resync {
        head_seq: i64,
        edits: Vec<Edit>,
        speaker_names: Vec<String>,
        transition: Transition,
        #[serde(default)]
        splits: Vec<f64>,
        #[serde(default)]
        order: Vec<f64>,
        /// Sources appended after the project's own media (the fold's `doc.sources`).
        #[serde(default)]
        sources: Vec<Source>,
    },
    Error {
        code: String,
        detail: String,
    },
    /// The answer to a client's `{"t":"ping"}` liveness probe.
    Pong,
}

pub trait Bus: Send + Sync {
    fn publish(&self, project_id: &str, msg: ServerMsg);
    fn subscribe(&self, project_id: &str, conn_id: &str, user: PeerInfo) -> Subscription;
    /// Store a peer's new state and return the full `Peer` to broadcast, or
    /// `None` if the connection is unknown.
    fn update_presence(
        &self,
        project_id: &str,
        conn_id: &str,
        state: PresenceState,
    ) -> Option<Peer>;
    fn peers(&self, project_id: &str) -> Vec<Peer>;
}

/// Broadcast capacity per hub. A subscriber that falls this far behind gets
/// `Lagged`, which the socket answers with a resync.
const CAPACITY: usize = 256;

struct Hub {
    tx: broadcast::Sender<ServerMsg>,
    peers: Mutex<HashMap<String, Peer>>,
}

/// One project's live subscription. Dropping it removes the peer and tells
/// the others; the hub itself goes away with its last subscription.
pub struct Subscription {
    pub rx: broadcast::Receiver<ServerMsg>,
    hub: Arc<Hub>,
    conn_id: String,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.hub
            .peers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.conn_id);
        let _ = self.hub.tx.send(ServerMsg::Left {
            conn_id: self.conn_id.clone(),
        });
    }
}

#[derive(Default)]
pub struct LocalBus {
    hubs: Mutex<HashMap<String, Weak<Hub>>>,
}

impl LocalBus {
    pub fn new() -> Self {
        Self::default()
    }

    /// Live hubs, for tests and diagnostics.
    #[allow(dead_code)] // Diagnostic surface; only tests call it until then.
    pub fn hub_count(&self) -> usize {
        self.hubs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .filter(|w| w.strong_count() > 0)
            .count()
    }

    fn hub(&self, project_id: &str) -> Option<Arc<Hub>> {
        self.hubs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(project_id)
            .and_then(Weak::upgrade)
    }

    fn hub_or_create(&self, project_id: &str) -> Arc<Hub> {
        let mut hubs = self.hubs.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(hub) = hubs.get(project_id).and_then(Weak::upgrade) {
            return hub;
        }
        let (tx, _) = broadcast::channel(CAPACITY);
        let hub = Arc::new(Hub {
            tx,
            peers: Mutex::new(HashMap::new()),
        });
        hubs.retain(|_, w| w.strong_count() > 0);
        hubs.insert(project_id.to_owned(), Arc::downgrade(&hub));
        hub
    }
}

impl Bus for LocalBus {
    fn publish(&self, project_id: &str, msg: ServerMsg) {
        if let Some(hub) = self.hub(project_id) {
            // No subscribers is not an error: nobody is watching.
            let _ = hub.tx.send(msg);
        }
    }

    fn subscribe(&self, project_id: &str, conn_id: &str, user: PeerInfo) -> Subscription {
        let hub = self.hub_or_create(project_id);
        let peer = Peer {
            conn_id: conn_id.to_owned(),
            user,
            state: PresenceState::default(),
        };
        // Take the receiver before announcing, so nothing published between
        // the two is lost. The echo of our own join is the price: subscribers
        // filter presence by `connId`, and the hello's `peers` already has us.
        let rx = hub.tx.subscribe();
        hub.peers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(conn_id.to_owned(), peer.clone());
        let _ = hub.tx.send(ServerMsg::Presence(peer));
        Subscription {
            rx,
            hub,
            conn_id: conn_id.to_owned(),
        }
    }

    fn update_presence(
        &self,
        project_id: &str,
        conn_id: &str,
        state: PresenceState,
    ) -> Option<Peer> {
        let hub = self.hub(project_id)?;
        let mut peers = hub.peers.lock().unwrap_or_else(|e| e.into_inner());
        let peer = peers.get_mut(conn_id)?;
        peer.state = state;
        Some(peer.clone())
    }

    fn peers(&self, project_id: &str) -> Vec<Peer> {
        let Some(hub) = self.hub(project_id) else {
            return Vec::new();
        };
        let mut peers: Vec<Peer> = hub
            .peers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .cloned()
            .collect();
        peers.sort_by(|a, b| a.conn_id.cmp(&b.conn_id));
        peers
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn who(n: &str) -> PeerInfo {
        PeerInfo {
            id: format!("u-{n}"),
            display_name: n.to_owned(),
            color: "#123456".into(),
            bot: false,
        }
    }

    fn doc(seq: i64) -> ServerMsg {
        ServerMsg::Doc {
            seq,
            author_id: "u-a".into(),
            head_seq: seq,
            edits: vec![],
            speaker_names: vec![],
            transition: Transition::None,
            splits: vec![],
            order: vec![],
            sources: vec![],
        }
    }

    #[tokio::test]
    async fn publish_reaches_every_subscriber_of_that_project_only() {
        let bus = LocalBus::new();
        let mut a = bus.subscribe("p1", "c1", who("a"));
        let mut b = bus.subscribe("p1", "c2", who("b"));
        let mut other = bus.subscribe("p2", "c3", who("c"));
        // A subscriber sees its own join first, then every later one.
        let _ = a.rx.try_recv(); // a joined
        let _ = a.rx.try_recv(); // b joined
        let _ = b.rx.try_recv(); // b joined
        let _ = other.rx.try_recv(); // c joined
        bus.publish("p1", doc(7));
        assert_eq!(a.rx.recv().await.unwrap(), doc(7));
        assert_eq!(b.rx.recv().await.unwrap(), doc(7));
        assert!(other.rx.try_recv().is_err());
    }

    #[test]
    fn subscribe_announces_presence_and_drop_announces_left() {
        let bus = LocalBus::new();
        let mut a = bus.subscribe("p1", "c1", who("a"));
        let b = bus.subscribe("p1", "c2", who("b"));
        // Our own join echoes back first, then the one that followed it.
        match a.rx.try_recv().unwrap() {
            ServerMsg::Presence(p) => assert_eq!(p.conn_id, "c1"),
            other => panic!("expected presence, got {other:?}"),
        }
        match a.rx.try_recv().unwrap() {
            ServerMsg::Presence(p) => assert_eq!(p.conn_id, "c2"),
            other => panic!("expected presence, got {other:?}"),
        }
        assert_eq!(bus.peers("p1").len(), 2);
        drop(b);
        assert_eq!(
            a.rx.try_recv().unwrap(),
            ServerMsg::Left {
                conn_id: "c2".into()
            }
        );
        assert_eq!(bus.peers("p1").len(), 1);
    }

    #[test]
    fn hub_lives_only_while_subscribed() {
        let bus = LocalBus::new();
        assert_eq!(bus.hub_count(), 0);
        let a = bus.subscribe("p1", "c1", who("a"));
        assert_eq!(bus.hub_count(), 1);
        drop(a);
        assert_eq!(bus.hub_count(), 0);
        bus.publish("p1", doc(1)); // no hub, no panic
        assert!(bus.peers("p1").is_empty());
    }

    #[test]
    fn update_presence_returns_the_peer_and_ignores_unknown_connections() {
        let bus = LocalBus::new();
        let _a = bus.subscribe("p1", "c1", who("a"));
        let state = PresenceState {
            playhead: 3.5,
            selection: Some([2, 4]),
            caret: None,
            playing: true,
        };
        let peer = bus.update_presence("p1", "c1", state.clone()).unwrap();
        assert_eq!(peer.state, state);
        assert_eq!(bus.peers("p1")[0].state.playhead, 3.5);
        assert!(bus.update_presence("p1", "nope", state).is_none());
    }

    #[test]
    fn server_msg_json_shape() {
        let json = serde_json::to_value(ServerMsg::Left {
            conn_id: "c9".into(),
        })
        .unwrap();
        assert_eq!(json, serde_json::json!({ "t": "left", "connId": "c9" }));
        let json = serde_json::to_value(ServerMsg::Presence(Peer {
            conn_id: "c1".into(),
            user: who("a"),
            state: PresenceState::default(),
        }))
        .unwrap();
        assert_eq!(json["t"], "presence");
        assert_eq!(json["user"]["displayName"], "a");
        assert_eq!(json["state"]["playhead"], 0.0);
        // A doc frame carries the project transition so peers see a change to it.
        let json = serde_json::to_value(ServerMsg::Doc {
            seq: 4,
            author_id: "u-a".into(),
            head_seq: 4,
            edits: vec![],
            speaker_names: vec![],
            transition: Transition::Dip,
            splits: vec![],
            order: vec![],
            sources: vec![],
        })
        .unwrap();
        assert_eq!(json["t"], "doc");
        assert_eq!(json["transition"], "dip");
        let json = serde_json::to_value(ServerMsg::Resync {
            head_seq: 4,
            edits: vec![],
            speaker_names: vec![],
            transition: Transition::None,
            splits: vec![],
            order: vec![],
            sources: vec![],
        })
        .unwrap();
        assert_eq!(json["transition"], "none");
    }
}
