# echo-communication-v1 사용 가이드

## 전체 구조

```
echo-server (ws:55001)
       ▲
       │  WebSocket
       │
echo-client-chat (module)
       ▲
       │  stdin/stdout
       │
    Daemon (:7777)  ◀──── auth (module)
       ▲
       │  TCP
       │
  Controller (:8888)
       ▲
       │  TCP
       │
    CLI (TUI)
```

이 레포지토리는 **Controller**와 **CLI** 두 컴포넌트를 포함합니다.
나머지(Daemon, echo-server, auth-module, echo-client-chat)는 각자의 레포에서 빌드합니다.

---

## 1. 사전 준비

다음 바이너리들이 빌드되어 있어야 합니다.

| 컴포넌트 | 경로 (예시) |
|---------|-----------|
| Daemon | `laboratory/daemon/target/debug/daemon` |
| auth-module | `laboratory/module/auth-system_v1/target/debug/auth-module` |
| echo-client-chat | `laboratory/module/echo-client-chat_v1/target/debug/echo-client-chat` |
| echo-server | `laboratory/prototype/echo-server_v1/target/debug/echo-server-v1` |

이 프로젝트 빌드:

```bash
cd echo-communication_v1
cargo build
```

---

## 2. 실행 순서

컴포넌트는 반드시 아래 순서로 실행합니다.

### Step 1 — echo-server 시작

```bash
cd laboratory/prototype/echo-server_v1
cargo run
# ws://127.0.0.1:55001 에서 대기
```

### Step 2 — Daemon 시작

```bash
cd laboratory/daemon
cargo run
# TCP :7777 에서 대기
```

### Step 3 — Controller 시작

```bash
cd echo-communication_v1
./target/debug/echo-communication
# CLI 연결 대기 중 (:8888) ...
```

> **controller.json 설정** (바이너리 옆에 두면 자동 로드):
> ```json
> {
>   "daemon_addr": "127.0.0.1:7777",
>   "daemon_token": null,
>   "cli_port": 8888,
>   "auth_module_path": "/절대경로/auth-module",
>   "chat_module_path": "/절대경로/echo-client-chat"
> }
> ```
> `auth_module_path` / `chat_module_path`는 Daemon의 `daemon.yaml`에 모듈이 등록되어 있으면 생략 가능합니다.

### Step 4 — CLI 시작

```bash
./target/debug/echo-communication-cli
```

---

## 3. CLI 화면 구성

```
┌─────────────────────────────────────────────────────────┐
│  echo-communication │ user: david │ #general │ connected│  ← 상태 바
├─────────────────────────────────────────────────────────┤
│                                                         │
│  echo-communication cli. type 'help' for commands.      │
│  ✓ logged in as david                                   │  ← 메시지 영역
│  ✓ connected to chat server                             │
│  ✓ joined #general (alice, bob)                         │
│  [general] alice: hello!                                │
│  → charlie joined #general                              │
│                                                         │
├─────────────────────────────────────────────────────────┤
│ > _                                                     │  ← 입력창
└─────────────────────────────────────────────────────────┘
```

| 색상 | 의미 |
|-----|------|
| 초록 (`✓`) | 성공/시스템 알림 |
| 빨강 (`✗`) | 오류 |
| 노랑 (`→` / `←`) | 유저 입퇴장 |
| 하늘 (`[room]`) | 채팅 메시지 |
| 회색 | 일반 시스템 메시지 |

---

## 4. 명령어 레퍼런스

### 인증

```
register <username> <password>    계정 생성
login    <username> <password>    로그인
```

### 채팅 서버 연결

```
connect <ws_url>    채팅 서버에 연결
                    예) connect ws://127.0.0.1:55001
```

### 방 관리

```
join  <room>    방 입장 (없으면 생성)
leave           현재 방 퇴장
list            방 목록 조회
```

### 메시지

```
send <text>    메시지 전송
<text>         방 입장 후에는 명령어 없이 텍스트만 입력해도 전송됨
```

### 기타

```
state          현재 세션 상태 확인 (로그인/연결/방 정보)
disconnect     채팅 서버 연결 해제
help           명령어 목록 표시
quit / exit    종료
Ctrl+C         강제 종료
PageUp/Down    메시지 스크롤
```

---

## 5. 일반적인 사용 흐름

```
register david secret123
login david secret123
connect ws://127.0.0.1:55001
join general
hello everyone!
hello everyone!
join dev
leave
disconnect
```

---

## 6. 비즈니스 규칙

| 명령 | 필요 조건 |
|-----|---------|
| `connect` | 로그인 상태 |
| `join`, `leave`, `send`, `list`, `disconnect` | `connect` 완료 |
| 나머지 | 없음 |

조건 미충족 시 `✗ NOT_LOGGED_IN` 또는 `✗ NOT_CONNECTED` 오류가 표시됩니다.

---

## 7. 내부 프로토콜 (CLI ↔ Controller)

CLI와 Controller는 `127.0.0.1:8888`에서 JSON Lines로 통신합니다.
직접 `nc`로 테스트할 수 있습니다.

```bash
nc 127.0.0.1 8888
{"id":"1","action":"login","username":"david","password":"secret123"}
{"id":"2","action":"connect","server_url":"ws://127.0.0.1:55001"}
{"id":"3","action":"join","room":"general"}
{"id":"4","action":"send","text":"hello!"}
```

응답 형식:
```json
{"id":"1","type":"response","data":{"username":"david","id":"<uuid>"}}
{"id":"2","type":"error","code":"NOT_LOGGED_IN","message":"..."}
{"type":"event","topic":"echo_client_chat.message","data":{"from":"alice","text":"hi"}}
```

---

## 8. 에러 코드

| 코드 | 의미 |
|-----|------|
| `NOT_LOGGED_IN` | 로그인 필요 |
| `NOT_CONNECTED` | 채팅 서버 연결 필요 |
| `MISSING_FIELD` | 필수 파라미터 누락 |
| `UNKNOWN_ACTION` | 알 수 없는 명령 |
| `DAEMON_ERROR` | Daemon 통신 오류 |
| Daemon 에러 코드 그대로 전달 | `MODULE_NOT_RUNNING` 등 |
