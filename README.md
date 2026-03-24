# echo-communication-v1

Daemon 기반 채팅 서비스의 **Controller + CLI** 구현체입니다.

## 아키텍처

```
CLI (TUI) ──TCP:8888──▶ Controller ──TCP:7777──▶ Daemon ──stdin/stdout──▶ Modules
                                                              ├── auth-module
                                                              └── echo-client-chat
```

- **Controller**: Daemon을 통해 `auth-module`과 `echo-client-chat` 모듈을 조합해 채팅 서비스의 업무 흐름을 처리합니다.
- **CLI**: `ratatui` 기반 TUI. Controller에 TCP로 연결해 사용자 입력을 전달하고 결과를 표시합니다.
- 두 컴포넌트는 런타임 TCP 통신만 사용하며 Rust 크레이트 의존이 없습니다.

## 프로젝트 구조

```
echo-communication_v1/
├── Cargo.toml          # workspace (members: controller, cli)
├── controller/         # 업무 흐름 처리
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs     # 진입점, daemon 연결, CLI TCP 서버
│       ├── config.rs   # 설정 파일 로드
│       ├── session.rs  # 세션 상태
│       ├── shared.rs   # 공유 상태 + daemon 통신 헬퍼
│       └── handler.rs  # 비즈니스 로직
└── cli/                # TUI 인터페이스
    ├── Cargo.toml
    └── src/
        └── main.rs
```

## 의존 크레이트

| 컴포넌트 | 의존 |
|---------|------|
| controller | `serde`, `serde_json`, `uuid` |
| cli | `ratatui`, `crossterm`, `serde_json`, `uuid` |

## 빌드

```bash
cargo build
# 또는 릴리즈 빌드
cargo build --release
```

## 실행

### 1. Controller

```bash
./target/debug/echo-communication
```

바이너리 옆에 `controller.json`을 두면 설정을 덮어쓸 수 있습니다.

```json
{
  "daemon_addr": "127.0.0.1:7777",
  "daemon_token": null,
  "cli_port": 8888,
  "auth_module_path": "/path/to/auth-module",
  "chat_module_path": "/path/to/echo-client-chat"
}
```

### 2. CLI

```bash
./target/debug/echo-communication-cli
```

Controller가 실행 중이어야 합니다 (기본 `127.0.0.1:8888`).

## CLI 사용법

```
register <user> <pass>   계정 생성
login <user> <pass>      로그인
connect <ws_url>         채팅 서버 연결  (예: ws://localhost:8080/ws)
join <room>              방 입장
leave                    방 퇴장
send <text>              메시지 전송 (방 입장 후 텍스트 입력만으로도 동작)
list                     방 목록 조회
state                    현재 세션 상태 확인
disconnect               채팅 서버 연결 해제
quit / exit / Ctrl+C     종료
PageUp / PageDown        메시지 스크롤
```

## Controller ↔ CLI 프로토콜

TCP JSON Lines 기반.

**CLI → Controller:**
```json
{"id": "1", "action": "login", "username": "david", "password": "1234"}
{"id": "2", "action": "connect", "server_url": "ws://localhost:8080/ws"}
{"id": "3", "action": "join", "room": "general"}
{"id": "4", "action": "send", "text": "hello!"}
```

**Controller → CLI:**
```json
{"id": "1", "type": "response", "data": {"username": "david", "id": "<uuid>"}}
{"id": "1", "type": "error",    "code": "NOT_LOGGED_IN", "message": "..."}
{"type": "event", "topic": "echo_client_chat.message", "data": {"from": "alice", "text": "hi"}}
```

## 관련 프로젝트

- `auth-system_v1` — 인증 모듈 (PostgreSQL 기반 register/login)
- `echo-client-chat_v1` — WebSocket 채팅 클라이언트 모듈
- Daemon — 모듈 라우팅 및 생명주기 관리
