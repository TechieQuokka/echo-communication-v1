use std::sync::Arc;

use serde_json::{json, Value};

use crate::shared::Shared;

type CmdResult = Result<Value, (String, String)>;

fn err(code: &str, msg: &str) -> CmdResult {
    Err((code.to_string(), msg.to_string()))
}

fn daemon_err(e: String) -> (String, String) {
    if let Some(pos) = e.find(':') {
        let code = e[..pos].to_string();
        let msg = e[pos + 1..].to_string();
        return (code, msg);
    }
    (e.clone(), e)
}

/// CLI cmd에서 module payload를 만든다.
/// CLI 전용 필드(id, action)를 제거하고 module action으로 교체.
fn cmd_payload(cmd: &Value, module_action: &str) -> Value {
    let mut map = cmd.as_object().cloned().unwrap_or_default();
    map.remove("id");
    map.insert("action".to_string(), json!(module_action));
    Value::Object(map)
}

pub fn handle(shared: &Arc<Shared>, action: &str, cmd: &Value) -> CmdResult {
    match action {
        "auth.register"    => handle_register(shared, cmd),
        "auth.login"       => handle_login(shared, cmd),
        "auth.passwd"      => handle_passwd(shared, cmd),
        "auth.check"       => handle_check(shared, cmd),
        "auth.list"        => handle_auth_list(shared, cmd),
        "chat.connect"     => handle_connect(shared, cmd),
        "chat.join"        => handle_join(shared, cmd),
        "chat.send"        => handle_send(shared, cmd),
        "chat.leave"       => handle_leave(shared),
        "chat.list"        => handle_list(shared),
        "chat.state"       => handle_state(shared),
        "chat.disconnect"  => handle_disconnect(shared),
        "help"             => handle_help(shared),
        "state"            => handle_state(shared),
        _ => err("UNKNOWN_ACTION", &format!("unknown action: {}", action)),
    }
}

fn handle_register(shared: &Arc<Shared>, cmd: &Value) -> CmdResult {
    shared
        .send_and_wait("command", "auth", cmd_payload(cmd, "register"))
        .map_err(daemon_err)
}

fn handle_login(shared: &Arc<Shared>, cmd: &Value) -> CmdResult {
    let data = shared
        .send_and_wait("command", "auth", cmd_payload(cmd, "login"))
        .map_err(daemon_err)?;

    {
        let mut sess = shared.session.lock().unwrap();
        sess.user_id  = data["id"].as_str().map(str::to_string);
        sess.username = data["username"].as_str().map(str::to_string);
    }

    // 세션 변화를 CLI에 이벤트로 알림
    {
        let sess = shared.session.lock().unwrap();
        shared.write_to_cli(&json!({
            "type": "event",
            "topic": "controller.session_sync",
            "data": {
                "username": sess.username,
                "chat_connected": sess.chat_connected,
                "current_room": sess.current_room,
            },
        }));
    }

    Ok(data)
}

fn handle_connect(shared: &Arc<Shared>, cmd: &Value) -> CmdResult {
    let username = {
        let sess = shared.session.lock().unwrap();
        sess.username.clone()
    };
    let username = match username {
        Some(u) => u,
        None => return err("NOT_LOGGED_IN", "login required before connecting to chat"),
    };

    let mut payload = cmd_payload(cmd, "connect");
    payload["nickname"] = json!(username);

    shared
        .send_and_wait("command", "echo_client_chat", payload)
        .map_err(daemon_err)
}

fn handle_join(shared: &Arc<Shared>, cmd: &Value) -> CmdResult {
    require_chat_connected(shared)?;
    shared
        .send_and_wait("command", "echo_client_chat", cmd_payload(cmd, "join"))
        .map_err(daemon_err)
}

fn handle_send(shared: &Arc<Shared>, cmd: &Value) -> CmdResult {
    require_chat_connected(shared)?;
    shared
        .send_and_wait("command", "echo_client_chat", cmd_payload(cmd, "send"))
        .map_err(daemon_err)
}

fn handle_leave(shared: &Arc<Shared>) -> CmdResult {
    require_chat_connected(shared)?;
    shared
        .send_and_wait("command", "echo_client_chat", json!({ "action": "leave" }))
        .map_err(daemon_err)
}

fn handle_list(shared: &Arc<Shared>) -> CmdResult {
    require_chat_connected(shared)?;
    shared
        .send_and_wait("command", "echo_client_chat", json!({ "action": "list" }))
        .map_err(daemon_err)
}

fn handle_state(shared: &Arc<Shared>) -> CmdResult {
    let sess = shared.session.lock().unwrap();
    Ok(json!({
        "logged_in":     sess.username.is_some(),
        "user_id":       sess.user_id,
        "username":      sess.username,
        "chat_connected": sess.chat_connected,
        "current_room":  sess.current_room,
    }))
}

fn handle_disconnect(shared: &Arc<Shared>) -> CmdResult {
    require_chat_connected(shared)?;
    shared
        .send_and_wait("command", "echo_client_chat", json!({ "action": "disconnect" }))
        .map_err(daemon_err)
}

fn handle_passwd(shared: &Arc<Shared>, cmd: &Value) -> CmdResult {
    shared
        .send_and_wait("command", "auth", cmd_payload(cmd, "passwd"))
        .map_err(daemon_err)
}

fn handle_check(shared: &Arc<Shared>, cmd: &Value) -> CmdResult {
    shared
        .send_and_wait("command", "auth", cmd_payload(cmd, "check"))
        .map_err(daemon_err)
}

fn handle_auth_list(shared: &Arc<Shared>, cmd: &Value) -> CmdResult {
    shared
        .send_and_wait("command", "auth", cmd_payload(cmd, "list"))
        .map_err(daemon_err)
}

fn handle_help(shared: &Arc<Shared>) -> CmdResult {
    let auth_help = shared
        .send_and_wait("command", "auth", json!({ "action": "help" }))
        .map_err(daemon_err)?;
    let chat_help = shared
        .send_and_wait("command", "echo_client_chat", json!({ "action": "help" }))
        .map_err(daemon_err)?;
    Ok(json!({
        "auth": auth_help,
        "chat": chat_help,
    }))
}

fn require_chat_connected(shared: &Arc<Shared>) -> CmdResult {
    let connected = shared.session.lock().unwrap().chat_connected;
    if !connected {
        return err("NOT_CONNECTED", "connect to chat server first");
    }
    Ok(json!(null))
}
