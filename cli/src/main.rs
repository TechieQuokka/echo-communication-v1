use std::io::{self, BufRead, BufReader, Write};
use std::net::TcpStream;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use unicode_width::UnicodeWidthStr;

use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Terminal,
};
use serde_json::{json, Value};

const CONTROLLER_ADDR: &str = "127.0.0.1:8888";

// ── 앱 상태 ─────────────────────────────────────────────────────────────────

struct AppState {
    messages: Vec<String>,
    input: String,
    username: Option<String>,
    current_room: Option<String>,
    chat_connected: bool,
    scroll_offset: usize,
}

impl AppState {
    fn new() -> Self {
        let mut s = Self {
            messages: Vec::new(),
            input: String::new(),
            username: None,
            current_room: None,
            chat_connected: false,
            scroll_offset: 0,
        };
        s.push_sys("echo-communication cli. type /help for commands.");
        s
    }

    fn push(&mut self, line: String) {
        self.messages.push(line);
        // 새 메시지가 오면 맨 아래로 스크롤
        self.scroll_offset = self.messages.len().saturating_sub(1);
    }

    fn push_sys(&mut self, msg: &str) {
        self.push(format!("  {}", msg));
    }

    fn push_ok(&mut self, msg: &str) {
        self.push(format!("✓ {}", msg));
    }

    fn push_err(&mut self, code: &str, msg: &str) {
        self.push(format!("✗ {}: {}", code, msg));
    }
}

// ── 소켓 → 앱으로 전달하는 이벤트 ──────────────────────────────────────────

enum CtrlEvent {
    Response { _id: String, action: String, data: Value },
    Error { _id: String, code: String, message: String },
    ChatEvent { topic: String, data: Value },
    Disconnected,
}

// ── 진입점 ──────────────────────────────────────────────────────────────────

fn main() {
    let stream = TcpStream::connect(CONTROLLER_ADDR).unwrap_or_else(|e| {
        eprintln!("controller 연결 실패 ({}): {}", CONTROLLER_ADDR, e);
        std::process::exit(1);
    });

    let reader_stream = stream.try_clone().expect("stream clone");
    let writer: Arc<Mutex<Box<dyn Write + Send>>> =
        Arc::new(Mutex::new(Box::new(stream)));

    let (event_tx, event_rx) = mpsc::channel::<CtrlEvent>();

    // 소켓 읽기 스레드
    {
        let event_tx = event_tx.clone();
        std::thread::spawn(move || {
            let reader = BufReader::new(reader_stream);
            for line in reader.lines() {
                let line = match line {
                    Ok(l) => l,
                    Err(_) => {
                        let _ = event_tx.send(CtrlEvent::Disconnected);
                        break;
                    }
                };
                if line.trim().is_empty() { continue; }
                let msg: Value = match serde_json::from_str(&line) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let evt = match msg["type"].as_str().unwrap_or("") {
                    "response" => CtrlEvent::Response {
                        _id: msg["id"].as_str().unwrap_or("").to_string(),
                        action: msg["action"].as_str().unwrap_or("").to_string(),
                        data: msg["data"].clone(),
                    },
                    "error" => CtrlEvent::Error {
                        _id: msg["id"].as_str().unwrap_or("").to_string(),
                        code: msg["code"].as_str().unwrap_or("ERROR").to_string(),
                        message: msg["message"].as_str().unwrap_or("").to_string(),
                    },
                    "event" => CtrlEvent::ChatEvent {
                        topic: msg["topic"].as_str().unwrap_or("").to_string(),
                        data: msg["data"].clone(),
                    },
                    _ => continue,
                };
                if event_tx.send(evt).is_err() { break; }
            }
        });
    }

    // TUI 초기화
    enable_raw_mode().expect("enable raw mode");
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).expect("enter alternate screen");
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).expect("terminal");

    let mut state = AppState::new();
    let mut msg_id_counter: u64 = 0;

    // 메인 루프
    loop {
        // 소켓 이벤트 처리 (non-blocking drain)
        loop {
            match event_rx.try_recv() {
                Ok(evt) => handle_ctrl_event(evt, &mut state),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    state.push_err("DISCONNECTED", "controller connection lost");
                    break;
                }
            }
        }

        // 렌더링
        terminal.draw(|f| render(f, &state)).expect("draw");

        // 키보드 이벤트 (50ms 폴링)
        if !event::poll(Duration::from_millis(50)).unwrap_or(false) {
            continue;
        }

        if let Ok(Event::Key(key)) = event::read() {
            if handle_key(key, &mut state, &writer, &mut msg_id_counter) {
                break; // quit
            }
        }
    }

    // TUI 종료
    disable_raw_mode().expect("disable raw mode");
    execute!(terminal.backend_mut(), LeaveAlternateScreen).expect("leave alternate screen");
}

// ── 키 처리 ─────────────────────────────────────────────────────────────────

fn handle_key(
    key: KeyEvent,
    state: &mut AppState,
    writer: &Arc<Mutex<Box<dyn Write + Send>>>,
    id_counter: &mut u64,
) -> bool {
    match key.code {
        // 종료
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return true,

        // 엔터: 명령 실행
        KeyCode::Enter => {
            let input = state.input.trim().to_string();
            if input.is_empty() { return false; }
            state.input.clear();

            if input == "/quit" || input == "/exit" {
                return true;
            }

            if input == "/clear" {
                state.messages.clear();
                state.scroll_offset = 0;
                return false;
            }

            // 슬래시 없으면 채팅 메시지로 바로 처리
            if !input.starts_with('/') {
                if state.current_room.is_some() {
                    *id_counter += 1;
                    send_to_ctrl(writer, &json!({
                        "id": id_counter.to_string(),
                        "action": "chat.send",
                        "text": input,
                    }));
                } else {
                    state.push_err("USAGE", "join a room first, or use /command");
                }
                return false;
            }

            match parse_command(input.trim_start_matches('/'), state) {
                Some(payload) => {
                    *id_counter += 1;
                    let id = id_counter.to_string();
                    let msg = json!({
                        "id": id,
                        "action": payload["action"],
                    });
                    // action 외 나머지 필드 병합
                    let mut full = msg.as_object().cloned().unwrap_or_default();
                    if let Some(obj) = payload.as_object() {
                        for (k, v) in obj {
                            full.insert(k.clone(), v.clone());
                        }
                    }
                    send_to_ctrl(writer, &Value::Object(full));
                }
                None => {} // 파싱 오류는 parse_command 내에서 state에 push
            }
        }

        // 백스페이스
        KeyCode::Backspace => { state.input.pop(); }

        // 스크롤 (PageUp/PageDown)
        KeyCode::PageUp => {
            state.scroll_offset = state.scroll_offset.saturating_sub(5);
        }
        KeyCode::PageDown => {
            state.scroll_offset = (state.scroll_offset + 5)
                .min(state.messages.len().saturating_sub(1));
        }

        // 문자 입력
        KeyCode::Char(c) => { state.input.push(c); }

        _ => {}
    }
    false
}

// ── 명령 파싱 ────────────────────────────────────────────────────────────────

fn parse_command(input: &str, state: &mut AppState) -> Option<Value> {
    // 모듈 접두사 없는 특수 명령
    match input.split_whitespace().next().unwrap_or("") {
        "help"  => return Some(json!({ "action": "help" })),
        "state" => return Some(json!({ "action": "state" })),
        _ => {}
    }

    // module.action [args...] 형식 파싱
    let dot = match input.find('.') {
        Some(p) => p,
        None => {
            state.push_err("UNKNOWN", &format!("unknown command: /{}. use /module.action format", input));
            return None;
        }
    };

    let module = &input[..dot];
    let rest   = &input[dot + 1..];
    let parts: Vec<&str> = rest.splitn(3, ' ').collect();
    let action = parts[0];

    let payload = match (module, action) {
        // ── auth ──────────────────────────────────────────────────────────
        ("auth", "register") => {
            if parts.len() < 3 { state.push_err("USAGE", "/auth.register <username> <password>"); return None; }
            json!({ "action": "auth.register", "username": parts[1], "password": parts[2] })
        }
        ("auth", "login") => {
            if parts.len() < 3 { state.push_err("USAGE", "/auth.login <username> <password>"); return None; }
            json!({ "action": "auth.login", "username": parts[1], "password": parts[2] })
        }
        ("auth", "passwd") => {
            let sub: Vec<&str> = rest.splitn(4, ' ').collect();
            if sub.len() < 4 { state.push_err("USAGE", "/auth.passwd <username> <old_pw> <new_pw>"); return None; }
            json!({ "action": "auth.passwd", "username": sub[1], "old_password": sub[2], "new_password": sub[3] })
        }
        ("auth", "check") => {
            if parts.len() < 2 { state.push_err("USAGE", "/auth.check <username>"); return None; }
            json!({ "action": "auth.check", "username": parts[1] })
        }
        ("auth", "list") => {
            json!({ "action": "auth.list" })
        }
        // ── chat ──────────────────────────────────────────────────────────
        ("chat", "connect") => {
            if parts.len() < 2 { state.push_err("USAGE", "/chat.connect <server_url>"); return None; }
            json!({ "action": "chat.connect", "server_url": parts[1] })
        }
        ("chat", "join") => {
            if parts.len() < 2 { state.push_err("USAGE", "/chat.join <room>"); return None; }
            json!({ "action": "chat.join", "room": parts[1] })
        }
        ("chat", "send") => {
            if parts.len() < 2 { state.push_err("USAGE", "/chat.send <text>"); return None; }
            let text: Vec<&str> = rest.splitn(2, ' ').collect();
            json!({ "action": "chat.send", "text": text.get(1).unwrap_or(&"") })
        }
        ("chat", "leave")      => json!({ "action": "chat.leave" }),
        ("chat", "list")       => json!({ "action": "chat.list" }),
        ("chat", "state")      => json!({ "action": "chat.state" }),
        ("chat", "disconnect") => json!({ "action": "chat.disconnect" }),
        _ => {
            state.push_err("UNKNOWN", &format!("unknown command: /{}.{}. type /help", module, action));
            return None;
        }
    };

    Some(payload)
}

// ── 컨트롤러 이벤트 처리 ────────────────────────────────────────────────────

fn handle_ctrl_event(evt: CtrlEvent, state: &mut AppState) {
    match evt {
        CtrlEvent::Response { action, data, _id: _ } => {
            match action.as_str() {
                "auth.register" => {
                    let username = data["username"].as_str().unwrap_or("?");
                    state.push_ok(&format!("registered: {}", username));
                }
                "auth.login" => {
                    let username = data["username"].as_str().unwrap_or("?");
                    state.push_ok(&format!("logged in as {}", username));
                }
                "auth.passwd" => {
                    state.push_ok("password changed");
                }
                "auth.check" => {
                    if data["exists"].as_bool().unwrap_or(false) {
                        state.push_ok("user exists");
                    } else {
                        state.push_sys("user does not exist");
                    }
                }
                "auth.list" => {
                    if let Some(users) = data.as_array() {
                        if users.is_empty() {
                            state.push_sys("no users");
                        } else {
                            for u in users {
                                state.push_sys(&format!(
                                    "  {} ({})",
                                    u["username"].as_str().unwrap_or("?"),
                                    u["created_at"].as_str().unwrap_or("?"),
                                ));
                            }
                        }
                    }
                }
                "chat.connect" => {
                    state.push_ok("connecting to chat server...");
                }
                "chat.join" | "chat.leave" | "chat.send" | "chat.list" | "chat.disconnect" => {
                    state.push_ok("ok");
                }
                "chat.state" | "state" => {
                    display_state(state, &data);
                }
                "help" => {
                    for (section, label) in [("auth", "── auth ──"), ("chat", "── chat ──")] {
                        if let Some(cmds) = data[section].as_array() {
                            state.push_sys(label);
                            for cmd in cmds {
                                let name = cmd["command"].as_str().unwrap_or("");
                                if name == "help" { continue; }
                                let args = cmd["args"].as_str().unwrap_or("");
                                let desc = cmd["description"].as_str().unwrap_or("");
                                if args.is_empty() {
                                    state.push_sys(&format!("  /{}.{} — {}", section, name, desc));
                                } else {
                                    state.push_sys(&format!("  /{}.{} {} — {}", section, name, args, desc));
                                }
                            }
                        }
                    }
                }
                _ => {
                    if !data.is_null() {
                        state.push(format!("  {}", data));
                    }
                }
            }
        }

        CtrlEvent::Error { code, message, _id: _ } => {
            state.push_err(&code, &message);
        }

        CtrlEvent::ChatEvent { topic, data } => {
            handle_chat_event(state, &topic, &data);
        }

        CtrlEvent::Disconnected => {
            state.push_err("DISCONNECTED", "lost connection to controller");
        }
    }
}

fn handle_chat_event(state: &mut AppState, topic: &str, data: &Value) {
    match topic {
        "controller.session_sync" => {
            state.username = data["username"].as_str().map(str::to_string);
            state.chat_connected = data["chat_connected"].as_bool().unwrap_or(false);
            state.current_room = data["current_room"].as_str().map(str::to_string);
        }
        "echo_client_chat.connected" => {
            state.chat_connected = true;
            state.push_ok("connected to chat server");
        }
        "echo_client_chat.joined" => {
            let room = data["room"].as_str().unwrap_or("?");
            state.current_room = Some(room.to_string());
            let members: Vec<&str> = data["members"]
                .as_array()
                .map(|arr| arr.iter()
                    .filter_map(|m| m["nickname"].as_str())
                    .collect())
                .unwrap_or_default();
            if members.is_empty() {
                state.push_ok(&format!("joined #{}", room));
            } else {
                state.push_ok(&format!("joined #{} ({})", room, members.join(", ")));
            }
        }
        "echo_client_chat.left" => {
            let room = data["room"].as_str().unwrap_or("?");
            state.current_room = None;
            state.push_ok(&format!("left #{}", room));
        }
        "echo_client_chat.message" => {
            let from = data["from"].as_str().unwrap_or("?");
            let room = data["room"].as_str().unwrap_or("?");
            let text = data["text"].as_str().unwrap_or("");
            state.push(format!("[{}] {}: {}", room, from, text));
        }
        "echo_client_chat.user_joined" => {
            let display = data["display"].as_str().unwrap_or("?");
            let room = data["room"].as_str().unwrap_or("?");
            state.push(format!("→ {} joined #{}", display, room));
        }
        "echo_client_chat.user_left" => {
            let display = data["display"].as_str().unwrap_or("?");
            let room = data["room"].as_str().unwrap_or("?");
            state.push(format!("← {} left #{}", display, room));
        }
        "echo_client_chat.room_list" => {
            let rooms: Vec<&str> = data["rooms"]
                .as_array()
                .map(|arr| arr.iter().filter_map(|r| r.as_str()).collect())
                .unwrap_or_default();
            if rooms.is_empty() {
                state.push_sys("rooms: (none)");
            } else {
                state.push_sys(&format!("rooms: {}", rooms.join(", ")));
            }
        }
        "echo_client_chat.disconnected" => {
            state.chat_connected = false;
            state.current_room = None;
            state.push_err("DISCONNECTED", "disconnected from chat server");
        }
        "echo_client_chat.error" => {
            let msg = data["message"].as_str().unwrap_or("unknown error");
            state.push_err("CHAT_ERROR", msg);
        }
        _ => {}
    }
}

fn display_state(state: &mut AppState, data: &Value) {
    state.push_sys(&format!(
        "logged_in={} username={} chat={} room={}",
        data["logged_in"].as_bool().unwrap_or(false),
        data["username"].as_str().unwrap_or("none"),
        data["chat_connected"].as_bool().unwrap_or(false),
        data["current_room"].as_str().unwrap_or("none"),
    ));
}

// ── 컨트롤러로 JSON 전송 ─────────────────────────────────────────────────────

fn send_to_ctrl(writer: &Arc<Mutex<Box<dyn Write + Send>>>, msg: &Value) {
    let mut s = serde_json::to_string(msg).unwrap_or_default();
    s.push('\n');
    let mut w = writer.lock().unwrap();
    let _ = w.write_all(s.as_bytes());
    let _ = w.flush();
}

// ── TUI 렌더링 ───────────────────────────────────────────────────────────────

fn render(f: &mut ratatui::Frame, state: &AppState) {
    let area = f.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),  // 상태 바
            Constraint::Min(0),     // 메시지 영역
            Constraint::Length(3),  // 입력창
        ])
        .split(area);

    // ── 상태 바 ──
    let user_str = state.username.as_deref().unwrap_or("(not logged in)");
    let room_str = state.current_room.as_deref().unwrap_or("(no room)");
    let conn_str = if state.chat_connected { "connected" } else { "disconnected" };
    let status_text = format!(
        " echo-communication  │  user: {}  │  {}  │  {}",
        user_str, room_str, conn_str
    );
    let status = Paragraph::new(status_text)
        .style(Style::default().bg(Color::DarkGray).fg(Color::White).add_modifier(Modifier::BOLD));
    f.render_widget(status, chunks[0]);

    // ── 메시지 영역 ──
    let msg_height = chunks[1].height as usize;
    let total = state.messages.len();

    // 스크롤 오프셋 기준으로 보이는 메시지 슬라이스 계산
    let start = if total <= msg_height {
        0
    } else {
        state.scroll_offset.min(total.saturating_sub(msg_height))
    };
    let end = (start + msg_height).min(total);

    let items: Vec<ListItem> = state.messages[start..end]
        .iter()
        .map(|line| {
            let style = if line.starts_with('✓') {
                Style::default().fg(Color::Green)
            } else if line.starts_with('✗') {
                Style::default().fg(Color::Red)
            } else if line.starts_with('→') || line.starts_with('←') {
                Style::default().fg(Color::Yellow)
            } else if line.starts_with('[') {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default().fg(Color::Gray)
            };
            ListItem::new(Line::from(Span::styled(line.clone(), style)))
        })
        .collect();

    let msg_list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(" messages "));
    f.render_widget(msg_list, chunks[1]);

    // ── 입력창 ──
    let input_text = format!("> {}", state.input);
    let input_widget = Paragraph::new(input_text)
        .block(Block::default().borders(Borders::ALL).title(" input "))
        .style(Style::default().fg(Color::White));
    f.render_widget(input_widget, chunks[2]);

    // 커서 위치 설정
    let cursor_x = chunks[2].x + 2 + UnicodeWidthStr::width(state.input.as_str()) as u16 + 1;
    let cursor_y = chunks[2].y + 1;
    if cursor_x < chunks[2].x + chunks[2].width - 1 {
        f.set_cursor_position((cursor_x, cursor_y));
    }
}
