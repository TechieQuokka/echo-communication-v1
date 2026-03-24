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
    let username = cmd["username"].as_str().ok_or_else(|| ("MISSING_FIELD".to_string(), "missing username".to_string()))?;
    let password = cmd["password"].as_str().ok_or_else(|| ("MISSING_FIELD".to_string(), "missing password".to_string()))?;

    let data = shared
        .send_and_wait("command", "auth", json!({
            "action": "register",
            "username": username,
            "password": password,
        }))
        .map_err(daemon_err)?;

    Ok(data)
}

fn handle_login(shared: &Arc<Shared>, cmd: &Value) -> CmdResult {
    let username = cmd["username"].as_str().ok_or_else(|| ("MISSING_FIELD".to_string(), "missing username".to_string()))?;
    let password = cmd["password"].as_str().ok_or_else(|| ("MISSING_FIELD".to_string(), "missing password".to_string()))?;

    let data = shared
        .send_and_wait("command", "auth", json!({
            "action": "login",
            "username": username,
            "password": password,
        }))
        .map_err(daemon_err)?;

    {
        let mut sess = shared.session.lock().unwrap();
        sess.user_id = data["id"].as_str().map(str::to_string);
        sess.username = data["username"].as_str().map(str::to_string);
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

    let server_url = cmd["server_url"].as_str().ok_or_else(|| ("MISSING_FIELD".to_string(), "missing server_url".to_string()))?;

    let data = shared
        .send_and_wait("command", "echo_client_chat", json!({
            "action": "connect",
            "server_url": server_url,
            "nickname": username,
        }))
        .map_err(daemon_err)?;

    {
        let mut sess = shared.session.lock().unwrap();
        sess.chat_connected = true;
    }

    Ok(data)
}

fn handle_join(shared: &Arc<Shared>, cmd: &Value) -> CmdResult {
    require_chat_connected(shared)?;

    let room = cmd["room"].as_str().ok_or_else(|| ("MISSING_FIELD".to_string(), "missing room".to_string()))?;

    let data = shared
        .send_and_wait("command", "echo_client_chat", json!({
            "action": "join",
            "room": room,
        }))
        .map_err(daemon_err)?;

    // Optimistic update; confirmed via echo_client_chat.joined event
    {
        let mut sess = shared.session.lock().unwrap();
        sess.current_room = Some(room.to_string());
    }

    Ok(data)
}

fn handle_send(shared: &Arc<Shared>, cmd: &Value) -> CmdResult {
    require_chat_connected(shared)?;

    let text = cmd["text"].as_str().ok_or_else(|| ("MISSING_FIELD".to_string(), "missing text".to_string()))?;

    let data = shared
        .send_and_wait("command", "echo_client_chat", json!({
            "action": "send",
            "text": text,
        }))
        .map_err(daemon_err)?;

    Ok(data)
}

fn handle_leave(shared: &Arc<Shared>) -> CmdResult {
    require_chat_connected(shared)?;

    let data = shared
        .send_and_wait("command", "echo_client_chat", json!({ "action": "leave" }))
        .map_err(daemon_err)?;

    {
        let mut sess = shared.session.lock().unwrap();
        sess.current_room = None;
    }

    Ok(data)
}

fn handle_list(shared: &Arc<Shared>) -> CmdResult {
    require_chat_connected(shared)?;

    let data = shared
        .send_and_wait("command", "echo_client_chat", json!({ "action": "list" }))
        .map_err(daemon_err)?;

    Ok(data)
}

fn handle_state(shared: &Arc<Shared>) -> CmdResult {
    let sess = shared.session.lock().unwrap();
    Ok(json!({
        "logged_in": sess.username.is_some(),
        "user_id": sess.user_id,
        "username": sess.username,
        "chat_connected": sess.chat_connected,
        "current_room": sess.current_room,
    }))
}

fn handle_disconnect(shared: &Arc<Shared>) -> CmdResult {
    require_chat_connected(shared)?;

    let data = shared
        .send_and_wait("command", "echo_client_chat", json!({ "action": "disconnect" }))
        .map_err(daemon_err)?;

    {
        let mut sess = shared.session.lock().unwrap();
        sess.chat_connected = false;
        sess.current_room = None;
    }

    Ok(data)
}

fn handle_passwd(shared: &Arc<Shared>, cmd: &Value) -> CmdResult {
    let username = cmd["username"].as_str().ok_or_else(|| ("MISSING_FIELD".to_string(), "missing username".to_string()))?;
    let old_password = cmd["old_password"].as_str().ok_or_else(|| ("MISSING_FIELD".to_string(), "missing old_password".to_string()))?;
    let new_password = cmd["new_password"].as_str().ok_or_else(|| ("MISSING_FIELD".to_string(), "missing new_password".to_string()))?;

    let data = shared
        .send_and_wait("command", "auth", json!({
            "action": "passwd",
            "username": username,
            "old_password": old_password,
            "new_password": new_password,
        }))
        .map_err(daemon_err)?;

    Ok(data)
}

fn handle_check(shared: &Arc<Shared>, cmd: &Value) -> CmdResult {
    let username = cmd["username"].as_str().ok_or_else(|| ("MISSING_FIELD".to_string(), "missing username".to_string()))?;

    let data = shared
        .send_and_wait("command", "auth", json!({
            "action": "check",
            "username": username,
        }))
        .map_err(daemon_err)?;

    Ok(data)
}

fn handle_auth_list(shared: &Arc<Shared>, cmd: &Value) -> CmdResult {
    let mut payload = json!({ "action": "list" });
    if let Some(start) = cmd["start"].as_str() {
        payload["start"] = json!(start);
    }
    if let Some(end) = cmd["end"].as_str() {
        payload["end"] = json!(end);
    }
    let data = shared
        .send_and_wait("command", "auth", payload)
        .map_err(daemon_err)?;
    Ok(data)
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
