use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Write},
    net::TcpListener,
    process::{Child, Command, Stdio},
    thread,
    time::Duration,
};

struct Kill(Child);
impl Drop for Kill {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn stdio_game_flow() {
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let root = format!("{}/../../..", env!("CARGO_MANIFEST_DIR"));
    let _server = Kill(
        Command::new("node")
            .arg(format!("{root}/server.js"))
            .env("PORT", port.to_string())
            .env("HOST", "127.0.0.1")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("node is required for the service integration test"),
    );
    for _ in 0..100 {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    let mut mcp = Kill(
        Command::new(env!("CARGO_BIN_EXE_platform-mcp"))
            .env("DURAK_URL", format!("http://127.0.0.1:{port}"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let mut input = mcp.0.stdin.take().unwrap();
    let mut output = BufReader::new(mcp.0.stdout.take().unwrap());
    let mut call = |name: &str, args: Value| -> Value {
        writeln!(input, "{}", json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":args}})).unwrap();
        input.flush().unwrap();
        let mut line = String::new();
        output.read_line(&mut line).unwrap();
        let response: Value = serde_json::from_str(&line).unwrap();
        assert!(response["result"]["isError"] != true, "{name}: {response}");
        response["result"]["structuredContent"].clone()
    };
    call("durak_connect", json!({"seat":"a"}));
    call("durak_connect", json!({"seat":"b"}));
    let room = call("durak_room_create", json!({"seat":"a"}));
    call("durak_room_join", json!({"seat":"b","code":room["code"]}));
    call("durak_start", json!({"seat":"a"}));
    let state = call("durak_state", json!({"seat":"a"}));
    assert_eq!(state["state"]["room"]["status"], "playing");
    assert_eq!(state["state"]["hand"].as_array().unwrap().len(), 6);
    let a = call("durak_state", json!({"seat":"a"}));
    let attacker = if a["state"]["canAttack"] == true {
        "a"
    } else {
        "b"
    };
    let state = call("durak_state", json!({"seat":attacker}));
    let card = state["state"]["hand"][0]["id"].clone();
    let played = call(
        "durak_action",
        json!({"seat":attacker,"action":"attack","cardId":card}),
    );
    assert_eq!(played["snapshot"]["table"].as_array().unwrap().len(), 1);
    let hint = call("durak_hints", json!({"seat":attacker}));
    assert!(hint["attack"].is_array());
    let auto = call("durak_autoplay", json!({"maxSteps":1000}));
    assert_eq!(auto["finished"], true, "{auto}");
    assert!(auto["steps"].as_u64().unwrap() > 0);
    drop(call);
    for (id, seat) in [(2, "a"), (3, "b")] {
        writeln!(input,"{}",json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":"durak_follow","arguments":{"seat":seat,"totalMs":30000,"maxSteps":1000}}})).unwrap();
    }
    input.flush().unwrap();
    for _ in 0..2 {
        let mut line = String::new();
        output.read_line(&mut line).unwrap();
        let response: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(
            response["result"]["structuredContent"]["finished"], true,
            "{response}"
        );
    }
}
