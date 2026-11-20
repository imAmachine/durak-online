use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

fn state(seq: u64) -> Value {
    json!({"seq":seq,"room":{"id":"room1","status":"playing"},"hand":[],"table":[],"log":[]})
}

fn event(stream: &mut TcpStream, seq: u64) {
    write!(
        stream,
        "data: {}\n\n",
        json!({"type":"game","state":state(seq)})
    )
    .unwrap();
    stream.flush().unwrap();
}

struct Backend {
    address: String,
    requests: Arc<Mutex<Vec<(String, Value)>>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Backend {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap().to_string();
        listener.set_nonblocking(true).unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let thread = thread::spawn(move || {
            let sse: Arc<Mutex<Option<TcpStream>>> = Arc::new(Mutex::new(None));
            while !stopped.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let sse = sse.clone();
                        let captured = captured.clone();
                        thread::spawn(move || {
                            stream
                                .set_read_timeout(Some(Duration::from_secs(3)))
                                .unwrap();
                            let mut reader = BufReader::new(stream.try_clone().unwrap());
                            let mut first = String::new();
                            reader.read_line(&mut first).unwrap();
                            let path = first.split_whitespace().nth(1).unwrap_or("").to_owned();
                            let mut length = 0;
                            loop {
                                let mut line = String::new();
                                if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                                    break;
                                }
                                if let Some((key, value)) = line.split_once(':') {
                                    if key.eq_ignore_ascii_case("content-length") {
                                        length = value.trim().parse().unwrap();
                                    }
                                }
                            }
                            if path.starts_with("/events?") {
                                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\n\r\n").unwrap();
                                event(&mut stream, 10);
                                *sse.lock().unwrap() = Some(stream);
                                return;
                            }
                            let mut bytes = vec![0; length];
                            reader.read_exact(&mut bytes).unwrap();
                            let body: Value = serde_json::from_slice(&bytes).unwrap();
                            captured.lock().unwrap().push((path.clone(), body.clone()));
                            let (status, response) = match path.as_str() {
                                "/api/action" => match body["cardId"].as_str().unwrap_or("") {
                                    "reject" => ("409 Conflict", json!({"error":"move rejected"})),
                                    "new" => ("200 OK", json!({"state":state(11)})),
                                    "stale" => {
                                        event(sse.lock().unwrap().as_mut().unwrap(), 12);
                                        thread::sleep(Duration::from_millis(80));
                                        ("200 OK", json!({"state":state(9)}))
                                    }
                                    "none" => {
                                        event(sse.lock().unwrap().as_mut().unwrap(), 13);
                                        thread::sleep(Duration::from_millis(80));
                                        ("200 OK", json!({"ok":true}))
                                    }
                                    _ => panic!("unexpected action: {body}"),
                                },
                                "/api/leave" => {
                                    let count = captured
                                        .lock()
                                        .unwrap()
                                        .iter()
                                        .filter(|(path, _)| path == "/api/leave")
                                        .count();
                                    if count <= 2 {
                                        ("409 Conflict", json!({"error":"leave denied"}))
                                    } else {
                                        ("200 OK", json!({"ok":true}))
                                    }
                                }
                                _ => panic!("unexpected request: {path}"),
                            };
                            let payload = response.to_string();
                            write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}", payload.len()).unwrap();
                        });
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5))
                    }
                    Err(e) => panic!("accept: {e}"),
                }
            }
        });
        Self {
            address,
            requests,
            stop,
            thread: Some(thread),
        }
    }
}
impl Drop for Backend {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.thread.take().unwrap().join().unwrap();
    }
}

struct Mcp {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    session: std::path::PathBuf,
}
impl Mcp {
    fn new(backend: &Backend) -> Self {
        let session = std::env::temp_dir().join(format!(
            "durak-error-semantics-{}-{}.json",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut child = Command::new(env!("CARGO_BIN_EXE_platform-mcp"))
            .env("DURAK_URL", format!("http://{}", backend.address))
            .env("DURAK_MCP_SESSIONS", &session)
            .env("DURAK_LOGIN", "ignored-for-explicit-session")
            .env("DURAK_PASSWORD", "ignored-password")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let output = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            input,
            output,
            session,
        }
    }
    fn call(&mut self, name: &str, args: Value) -> Value {
        writeln!(self.input, "{}", json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":args}})).unwrap();
        self.input.flush().unwrap();
        let mut line = String::new();
        assert_ne!(
            self.output.read_line(&mut line).unwrap(),
            0,
            "subprocess exited during {name}"
        );
        let response: Value = serde_json::from_str(&line).unwrap();
        assert!(response.get("error").is_none(), "{response}");
        response["result"].clone()
    }
}
impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_file(&self.session);
    }
}

#[test]
fn action_and_leave_error_semantics() {
    let backend = Backend::new();
    let mut mcp = Mcp::new(&backend);
    let connected = mcp.call(
        "durak_connect",
        json!({"seat":"a","clientId":"test-id","token":"test-token"}),
    );
    assert_ne!(connected["isError"], true, "{connected}");
    let mut initial = Value::Null;
    for _ in 0..100 {
        initial = mcp.call("durak_state", json!({"seat":"a"}));
        if initial["structuredContent"]["state"]["seq"] == 10 {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(initial["structuredContent"]["state"]["seq"], 10);
    let action = |mcp: &mut Mcp, card: &str| {
        mcp.call(
            "durak_action",
            json!({"seat":"a","action":"attack","cardId":card,"verbose":true}),
        )
    };
    let rejected = action(&mut mcp, "reject");
    assert_eq!(rejected["isError"], true, "{rejected}");
    assert_eq!(rejected["structuredContent"]["seat"], "a");
    assert_eq!(rejected["structuredContent"]["action"], "attack");
    assert!(rejected["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("move rejected"));
    assert_eq!(
        backend
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(p, _)| p == "/api/action")
            .count(),
        1
    );
    for (card, seq) in [("new", 11), ("stale", 12), ("none", 13)] {
        let result = action(&mut mcp, card);
        assert_ne!(result["isError"], true, "{card}: {result}");
        assert_eq!(
            result["structuredContent"]["state"]["seq"], seq,
            "{card}: {result}"
        );
    }
    for (tool, args) in [
        ("durak_room_leave", json!({"seat":"a"})),
        ("durak_disconnect", json!({"seat":"a","leave":true})),
    ] {
        let result = mcp.call(tool, args);
        assert_eq!(result["isError"], true, "{result}");
        assert!(result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("leave denied"));
        let state = mcp.call("durak_state", json!({"seat":"a"}));
        assert_eq!(state["structuredContent"]["state"]["seq"], 13, "{state}");
        let seats = mcp.call("durak_seats", json!({}));
        assert_eq!(seats["structuredContent"]["seats"][0]["seat"], "a");
    }
    let left = mcp.call("durak_room_leave", json!({"seat":"a"}));
    assert_ne!(left["isError"], true, "{left}");
    let state = mcp.call("durak_state", json!({"seat":"a"}));
    assert!(state["structuredContent"]["state"].is_null());
    let disconnected = mcp.call("durak_disconnect", json!({"seat":"a","leave":true}));
    assert_ne!(disconnected["isError"], true, "{disconnected}");
    let seats = mcp.call("durak_seats", json!({}));
    assert_eq!(seats["structuredContent"]["seats"], json!([]));
    let saved: Value = serde_json::from_slice(&fs::read(&mcp.session).unwrap()).unwrap();
    assert_eq!(saved, json!([]));
    let requests = backend.requests.lock().unwrap();
    assert_eq!(
        requests.iter().filter(|(p, _)| p == "/api/action").count(),
        4
    );
    assert_eq!(
        requests.iter().filter(|(p, _)| p == "/api/leave").count(),
        4
    );
    assert!(requests
        .iter()
        .all(|(_, b)| b["clientId"] == "test-id" && b["token"] == "test-token"));
}
