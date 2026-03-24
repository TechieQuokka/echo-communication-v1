mod config;
mod handler;
mod session;
mod shared;

use std::io::{BufRead, BufReader, BufWriter};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;

use serde_json::{json, Value};

use shared::Shared;

const CONTROLLER_NAME: &str = "echo_communication";
const AUTH_MODULE: &str = "auth";
const CHAT_MODULE: &str = "echo_client_chat";

fn main() {
    let config = config::load();

    // ── Daemon 연결 ──────────────────────────────────────────────
    let daemon_stream = TcpStream::connect(&config.daemon_addr)
        .unwrap_or_else(|e| panic!("daemon 연결 실패 ({}): {}", config.daemon_addr, e));

    let daemon_reader = daemon_stream.try_clone().expect("daemon stream clone");

    let shared = Shared::new(daemon_stream, CONTROLLER_NAME);

    // ── Daemon reader 스레드 (send_and_wait 전에 반드시 먼저 실행) ──
    spawn_daemon_reader(daemon_reader, Arc::clone(&shared));

    // ── Daemon 인증 ──────────────────────────────────────────────
    shared
        .send_and_wait("system", "daemon", json!({
            "action": "auth",
            "name": CONTROLLER_NAME,
            "token": config.daemon_token,
        }))
        .unwrap_or_else(|e| panic!("daemon 인증 실패: {}", e));

    eprintln!("[ctrl] daemon 인증 완료");

    // ── 모듈 구독 + 시작 ─────────────────────────────────────────
    init_module(&shared, AUTH_MODULE, config.auth_module_path.as_deref())
        .unwrap_or_else(|e| panic!("auth 모듈 초기화 실패: {}", e));

    init_module(&shared, CHAT_MODULE, config.chat_module_path.as_deref())
        .unwrap_or_else(|e| panic!("chat 모듈 초기화 실패: {}", e));

    // ── 버스 구독 (채팅 이벤트) ──────────────────────────────────
    shared
        .send_and_wait("system", "daemon", json!({
            "action": "bus.subscribe",
            "topic": "echo_client_chat.#",
        }))
        .unwrap_or_else(|e| panic!("버스 구독 실패: {}", e));

    eprintln!("[ctrl] 모듈 준비 완료");

    // ── CLI TCP 리스너 ───────────────────────────────────────────
    let listener = TcpListener::bind(format!("0.0.0.0:{}", config.cli_port))
        .unwrap_or_else(|e| panic!("CLI 포트 바인딩 실패: {}", e));

    eprintln!("[ctrl] CLI 대기 중 (:{}) ...", config.cli_port);

    for incoming in listener.incoming() {
        let cli_stream = match incoming {
            Ok(s) => s,
            Err(e) => { eprintln!("[ctrl] accept 오류: {}", e); continue; }
        };

        eprintln!("[ctrl] CLI 연결: {}", cli_stream.peer_addr().map(|a| a.to_string()).unwrap_or_default());

        let cli_reader = cli_stream.try_clone().expect("cli stream clone");
        *shared.cli_writer.lock().unwrap() = Some(BufWriter::new(cli_stream));

        run_cli_session(cli_reader, Arc::clone(&shared));

        // CLI 연결 종료 처리
        *shared.cli_writer.lock().unwrap() = None;
        *shared.session.lock().unwrap() = session::Session::new();
        shared.pending.lock().unwrap().clear();

        eprintln!("[ctrl] CLI 연결 종료, 다음 연결 대기 중 ...");
    }
}

/// 모듈 구독 후 stopped 상태이면 시작
fn init_module(shared: &Arc<Shared>, name: &str, path: Option<&str>) -> Result<(), String> {
    let resp = shared.send_and_wait("system", "daemon", json!({
        "action": "subscribe",
        "module": name,
    }))?;

    let status = resp["status"].as_str().unwrap_or("unknown");
    eprintln!("[ctrl] 모듈 '{}' 상태: {}", name, status);

    if status == "stopped" {
        let mut payload = json!({ "action": "module.start", "name": name });
        if let Some(p) = path {
            payload["path"] = json!(p);
            payload["type"] = json!("demand");
        }
        match shared.send_and_wait("system", "daemon", payload) {
            Ok(_) => {}
            Err(e) if e == "MODULE_ALREADY_RUNNING" => {}
            Err(e) => return Err(e),
        }
        eprintln!("[ctrl] 모듈 '{}' 시작됨", name);
    }

    Ok(())
}

/// Daemon에서 오는 메시지를 읽어 pending 응답 라우팅 or CLI 이벤트 포워딩
fn spawn_daemon_reader(stream: TcpStream, shared: Arc<Shared>) {
    std::thread::spawn(move || {
        let reader = BufReader::new(stream);
        for line in reader.lines() {
            let line = match line {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("[daemon reader] 연결 종료: {}", e);
                    break;
                }
            };
            if line.trim().is_empty() { continue; }

            let msg: Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("[daemon reader] 파싱 오류: {}", e);
                    continue;
                }
            };

            match msg["type"].as_str().unwrap_or("") {
                "response" => {
                    if let Some(id) = msg["id"].as_str() {
                        let tx = shared.pending.lock().unwrap().remove(id);
                        if let Some(tx) = tx {
                            let _ = tx.send(msg);
                        }
                    }
                }
                "event" => {
                    let topic = msg["topic"].as_str().unwrap_or("").to_string();
                    let data = msg["payload"].clone();
                    on_event(&shared, &topic, &data);

                    shared.write_to_cli(&json!({
                        "type": "event",
                        "topic": topic,
                        "data": data,
                    }));
                }
                _ => {} // heartbeat 등 무시
            }
        }
    });
}

/// 이벤트로부터 세션 상태 업데이트
fn on_event(shared: &Arc<Shared>, topic: &str, data: &Value) {
    let mut sess = shared.session.lock().unwrap();
    match topic {
        "echo_client_chat.connected" => {
            sess.chat_connected = true;
        }
        "echo_client_chat.joined" => {
            if let Some(room) = data["room"].as_str() {
                sess.current_room = Some(room.to_string());
            }
        }
        "echo_client_chat.left" => {
            sess.current_room = None;
        }
        "echo_client_chat.disconnected" => {
            sess.chat_connected = false;
            sess.current_room = None;
        }
        _ => {}
    }
}

/// CLI 세션: JSON Lines 명령을 읽고 결과를 반환
fn run_cli_session(stream: TcpStream, shared: Arc<Shared>) {
    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.trim().is_empty() { continue; }

        let cmd: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                shared.write_to_cli(&json!({
                    "type": "error",
                    "code": "PARSE_ERROR",
                    "message": e.to_string(),
                }));
                continue;
            }
        };

        let id = cmd["id"].as_str().unwrap_or("").to_string();
        let action = cmd["action"].as_str().unwrap_or("").to_string();

        let response = match handler::handle(&shared, &action, &cmd) {
            Ok(data) => json!({ "id": id, "type": "response", "data": data }),
            Err((code, msg)) => json!({ "id": id, "type": "error", "code": code, "message": msg }),
        };

        shared.write_to_cli(&response);
    }
}
