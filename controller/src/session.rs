pub struct Session {
    pub user_id: Option<String>,
    pub username: Option<String>,
    pub chat_connected: bool,
    pub current_room: Option<String>,
}

impl Session {
    pub fn new() -> Self {
        Self {
            user_id: None,
            username: None,
            chat_connected: false,
            current_room: None,
        }
    }
}
