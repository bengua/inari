//! Bus-to-bus links over the authenticated transport. Durable subjects queue while
//! the peer is away and replay on reconnect; the receiving bus drops duplicates.

use crate::bus::{is_durable, Bus, LinkHandle, Message};
use crate::wire::{TransportError, Wire};
use serde_json::json;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, Notify};

/// One peer, across reconnections: an outbox, and whichever connection is current.
pub struct PeerLink {
    pub handle: LinkHandle,
    connected: Arc<Mutex<bool>>,
    conn: Arc<Mutex<Option<mpsc::UnboundedSender<Message>>>>,
    outbox: Arc<Mutex<VecDeque<Message>>>,
    gone: Arc<Notify>,
}

impl PeerLink {
    pub fn new(bus: &Arc<Bus>, peer_name: &str) -> Arc<Self> {
        let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
        let link = Arc::new(Self {
            handle: LinkHandle {
                peer_name: Arc::new(Mutex::new(peer_name.into())),
                tx,
            },
            connected: Arc::new(Mutex::new(false)),
            conn: Arc::default(),
            outbox: Arc::default(),
            gone: Arc::new(Notify::new()),
        });
        bus.add_link(link.handle.clone());
        let (conn, outbox) = (Arc::clone(&link.conn), Arc::clone(&link.outbox));
        tokio::spawn(async move {
            while let Some(m) = rx.recv().await {
                let sent = conn
                    .lock()
                    .expect("not poisoned")
                    .as_ref()
                    .map(|c| c.send(m.clone()).is_ok())
                    .unwrap_or(false);
                if !sent && is_durable(&m.subject) {
                    outbox.lock().expect("not poisoned").push_back(m);
                }
            }
        });
        link
    }

    pub fn connected(&self) -> bool {
        *self.connected.lock().expect("not poisoned")
    }

    /// Take over a fresh connection: flush the outbox, then read and write until it drops.
    pub fn attach(self: &Arc<Self>, wire: Wire, bus: Arc<Bus>) {
        *self.handle.peer_name.lock().expect("not poisoned") = wire
            .peer
            .get("bus")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let (mut reader, mut writer) = wire.split();
        let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
        for m in self.outbox.lock().expect("not poisoned").drain(..) {
            let _ = tx.send(m);
        }
        *self.conn.lock().expect("not poisoned") = Some(tx);
        *self.connected.lock().expect("not poisoned") = true;
        let me = Arc::clone(self);
        tokio::spawn(async move {
            let write = async {
                while let Some(m) = rx.recv().await {
                    if writer.send(&m.to_json()).await.is_err() {
                        return Some(m);
                    }
                }
                None
            };
            let read = async {
                loop {
                    match reader.recv().await {
                        Ok(v) => {
                            if let Some(m) = Message::from_json(&v) {
                                bus.deliver(m);
                            }
                        }
                        Err(_) => return,
                    }
                }
            };
            let unsent = tokio::select! { m = write => m, _ = read => None };
            *me.conn.lock().expect("not poisoned") = None;
            *me.connected.lock().expect("not poisoned") = false;
            // Whatever was handed to this connection and not written goes back to the
            // outbox, in order, if it is durable.
            let mut back: Vec<Message> = unsent.into_iter().collect();
            while let Ok(m) = rx.try_recv() {
                back.push(m);
            }
            let mut outbox = me.outbox.lock().expect("not poisoned");
            for m in back.into_iter().rev().filter(|m| is_durable(&m.subject)) {
                outbox.push_front(m);
            }
            drop(outbox);
            me.gone.notify_waiters();
        });
    }

    pub async fn closed(&self) {
        if self.connected() {
            self.gone.notified().await;
        }
    }
}

/// Accepts peers; links for `expected` peers exist from the start, so durable
/// messages queue before their first visit.
pub struct BusServer {
    bus: Arc<Bus>,
    key: Vec<u8>,
    links: Mutex<HashMap<String, Arc<PeerLink>>>,
}

impl BusServer {
    pub fn new(bus: Arc<Bus>, key: &[u8], expected: &[&str]) -> Arc<Self> {
        let links = expected
            .iter()
            .map(|p| (p.to_string(), PeerLink::new(&bus, p)))
            .collect();
        Arc::new(Self {
            bus,
            key: key.to_vec(),
            links: Mutex::new(links),
        })
    }

    pub fn link(&self, peer: &str) -> Option<Arc<PeerLink>> {
        self.links.lock().expect("not poisoned").get(peer).cloned()
    }

    pub async fn start(self: &Arc<Self>, bind: &str, port: u16) -> std::io::Result<u16> {
        let listener = TcpListener::bind((bind, port)).await?;
        let port = listener.local_addr()?.port();
        let me = Arc::clone(self);
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let me = Arc::clone(&me);
                tokio::spawn(async move {
                    let mut wire = Wire::new(stream, &me.key);
                    if wire
                        .handshake(false, json!({"bus": me.bus.name}), Duration::from_secs(5))
                        .await
                        .is_err()
                    {
                        return;
                    }
                    let peer = wire
                        .peer
                        .get("bus")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let link = {
                        let mut links = me.links.lock().expect("not poisoned");
                        Arc::clone(
                            links
                                .entry(peer.clone())
                                .or_insert_with(|| PeerLink::new(&me.bus, &peer)),
                        )
                    };
                    link.attach(wire, Arc::clone(&me.bus));
                });
            }
        });
        Ok(port)
    }
}

/// Dials a server and keeps the link up, reconnecting with a capped backoff.
pub struct BusClient {
    pub link: Arc<PeerLink>,
}

impl BusClient {
    pub fn start(bus: Arc<Bus>, key: &[u8], addr: &str) -> Self {
        let link = PeerLink::new(&bus, "");
        let (me, key, addr) = (Arc::clone(&link), key.to_vec(), addr.to_string());
        tokio::spawn(async move {
            let mut delay = Duration::from_millis(100);
            loop {
                let attempt: Result<Wire, TransportError> = async {
                    let mut wire = Wire::connect(&addr, &key, Duration::from_secs(5)).await?;
                    wire.handshake(true, json!({"bus": bus.name}), Duration::from_secs(5))
                        .await?;
                    Ok(wire)
                }
                .await;
                if let Ok(wire) = attempt {
                    me.attach(wire, Arc::clone(&bus));
                    delay = Duration::from_millis(100);
                    me.closed().await;
                }
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(Duration::from_secs(5));
            }
        });
        Self { link }
    }
}
