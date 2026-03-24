use std::collections::HashMap;
use std::io::{BufWriter, Write};
use std::net::TcpStream;
use std::sync::{mpsc, Arc, Mutex};

use serde_json::{json, Value};
use uuid::Uuid;

use crate::session::Session;

pub type PendingMap = Arc<Mutex<HashMap<String, mpsc::Sender<Value>>>>;

pub struct Shared {
    pub daemon_writer: Mutex<BufWriter<TcpStream>>,
    pub cli_writer: Mutex<Option<BufWriter<TcpStream>>>,
    pub pending: PendingMap,
    pub session: Mutex<Session>,
    pub from: String,
}

impl Shared {
    pub fn new(daemon_stream: TcpStream, from: &str) -> Arc<Self> {
        Arc::new(Self {
            daemon_writer: Mutex::new(BufWriter::new(daemon_stream)),
            cli_writer: Mutex::new(None),
            pending: Arc::new(Mutex::new(HashMap::new())),
            session: Mutex::new(Session::new()),
            from: from.to_string(),
        })
    }

    fn write_to_daemon(&self, msg: &Value) -> Result<(), String> {
        let mut s = serde_json::to_string(msg).map_err(|e| e.to_string())?;
        s.push('\n');
        let mut writer = self.daemon_writer.lock().unwrap();
        writer.write_all(s.as_bytes()).map_err(|e| e.to_string())?;
        writer.flush().map_err(|e| e.to_string())
    }

    pub fn send_and_wait(&self, msg_type: &str, to: &str, payload: Value) -> Result<Value, String> {
        let id = Uuid::new_v4().to_string();
        let (tx, rx) = mpsc::channel();
        self.pending.lock().unwrap().insert(id.clone(), tx);

        let msg = json!({
            "v": 1,
            "type": msg_type,
            "from": self.from,
            "to": to,
            "id": id,
            "topic": null,
            "error": null,
            "payload": payload,
        });

        if let Err(e) = self.write_to_daemon(&msg) {
            self.pending.lock().unwrap().remove(&id);
            return Err(e);
        }

        let response = rx.recv().map_err(|e| e.to_string())?;
        self.pending.lock().unwrap().remove(&id);

        if let Some(err) = response["error"].as_str() {
            return Err(err.to_string());
        }

        Ok(response["payload"].clone())
    }

    pub fn write_to_cli(&self, msg: &Value) {
        let mut s = serde_json::to_string(msg).unwrap_or_default();
        s.push('\n');
        let mut guard = self.cli_writer.lock().unwrap();
        if let Some(w) = guard.as_mut() {
            let _ = w.write_all(s.as_bytes());
            let _ = w.flush();
        }
    }
}
