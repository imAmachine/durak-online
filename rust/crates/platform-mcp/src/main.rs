use reqwest::blocking::Client;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    env,
    io::{self, BufRead, Write},
    sync::{Arc, Mutex, OnceLock},
    thread,
    time::{Duration, Instant},
};

#[derive(Default)]
struct Seat {
    label: String,
    name: String,
    id: String,
    token: String,
    streaming: bool,
    closed: bool,
    account: Value,
    game: Option<Value>,
    lobby: Option<Value>,
    last_error: Option<String>,
    seq: u64,
}
type Seats = Arc<Mutex<HashMap<String, Arc<Mutex<Seat>>>>>;
static SEATS: OnceLock<Seats> = OnceLock::new();
fn seats() -> &'static Seats {
    SEATS.get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
}
fn base() -> String {
    env::var("DURAK_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:4173".into())
        .trim_end_matches('/')
        .into()
}
fn client() -> Client {
    Client::builder()
        .timeout(Duration::from_secs(65))
        .build()
        .unwrap()
}
fn body(path: &str, data: &Value) -> Result<Value, String> {
    let r = client()
        .post(format!("{}{}", base(), path))
        .json(data)
        .send()
        .map_err(|e| e.without_url().to_string())?;
    let status = r.status();
    let v: Result<Value, _> = r.json();
    let v = v.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(v["error"].as_str().unwrap_or("HTTP error").into());
    }
    Ok(v)
}
fn seat_arg(a: &Value) -> Result<Arc<Mutex<Seat>>, String> {
    let label = a.get("seat").and_then(Value::as_str).unwrap_or("");
    let m = seats().lock().unwrap();
    if !label.is_empty() {
        if let Some(s) = m.get(label) {
            return Ok(s.clone());
        }
        return Err(format!("Нет места «{}»", label));
    }
    if m.len() == 1 {
        return Ok(m.values().next().unwrap().clone());
    }
    if m.is_empty() {
        return Err("Нет открытых мест: сначала durak_connect".into());
    }
    Err("Открыто несколько мест: укажите seat".into())
}
fn compact(s: &Value) -> Value {
    if s.is_null() {
        return Value::Null;
    }
    json!({"room":s["room"]["id"],"status":s["room"]["status"],"role":s["you"]["role"],"turn":s["turnName"],"attacker":s["attackerName"],"defender":s["defenderName"],"trump":s["trump"],"deck":s["deck"],"table":s["table"],"hand":s["hand"],"counts":s["players"],"actions":{"attack":s["canAttack"],"defend":s["canDefend"],"transfer":s["canTransfer"],"take":s["canTake"],"pass":s["canPass"],"start":s["canStart"]},"message":s["message"],"seq":s["seq"],"finished":s["finished"]})
}
fn result(text: String, data: Value) -> Value {
    json!({"content":[{"type":"text","text":text}],"structuredContent":data})
}
fn error(text: String) -> Value {
    json!({"content":[{"type":"text","text":text}],"isError":true})
}
fn update_game(seat: &Arc<Mutex<Seat>>, game: Value) {
    let mut s = seat.lock().unwrap();
    s.seq = game["seq"].as_u64().unwrap_or(s.seq);
    s.game = Some(game);
}
fn start_sse(seat: Arc<Mutex<Seat>>) {
    thread::spawn(move || loop {
        if seat.lock().unwrap().closed {
            break;
        }
        let (id, token, name) = {
            let s = seat.lock().unwrap();
            (s.id.clone(), s.token.clone(), s.name.clone())
        };
        let u = format!(
            "{}/events?id={}&token={}&name={}",
            base(),
            urlencoding(&id),
            urlencoding(&token),
            urlencoding(&name)
        );
        match Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(60))
            .build()
            .unwrap()
            .get(u)
            .send()
        {
            Ok(r) if r.status().is_success() => {
                seat.lock().unwrap().streaming = true;
                let mut lines = io::BufReader::new(r).lines();
                let mut data = String::new();
                while let Some(Ok(line)) = lines.next() {
                    if seat.lock().unwrap().closed {
                        break;
                    }
                    if line.starts_with("data: ") {
                        data = line[6..].into()
                    }
                    if line.is_empty() && !data.is_empty() {
                        if let Ok(v) = serde_json::from_str::<Value>(&data) {
                            let mut s = seat.lock().unwrap();
                            match v["type"].as_str() {
                                Some("game") => {
                                    s.seq = v["state"]["seq"].as_u64().unwrap_or(0);
                                    s.game = Some(v["state"].clone())
                                }
                                Some("account") => s.account = v["account"].clone(),
                                Some("lobby") => s.lobby = Some(v.clone()),
                                Some("noroom") => s.game = None,
                                _ => {}
                            }
                        }
                        data.clear()
                    }
                }
                seat.lock().unwrap().streaming = false
            }
            Ok(r) => seat.lock().unwrap().last_error = Some(format!("SSE HTTP {}", r.status())),
            Err(e) => seat.lock().unwrap().last_error = Some(e.without_url().to_string()),
        }
        if !seat.lock().unwrap().streaming {
            thread::sleep(Duration::from_millis(500));
        }
    });
}
fn urlencoding(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() {
                (b as char).to_string()
            } else {
                format!("%{:02X}", b)
            }
        })
        .collect()
}
fn connect(a: &Value) -> Result<Value, String> {
    let label = a
        .get("seat")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let label = if label.is_empty() {
        format!("p{}", seats().lock().unwrap().len() + 1)
    } else {
        label
    };
    let name = a
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or(&format!("MCP {}", label))
        .chars()
        .take(20)
        .collect::<String>();
    let accounts: Value = serde_json::from_str(&env::var("DURAK_DISCORD_ACCOUNTS").unwrap_or_default()).unwrap_or(Value::Null);
    let selected = accounts.as_object().and_then(|v| v.get(&label));
    let id = selected.and_then(|v| v["clientId"].as_str()).map(str::to_owned).or_else(|| env::var("DURAK_DISCORD_CLIENT_ID").ok()).filter(|v| !v.trim().is_empty()).ok_or("Укажите DURAK_DISCORD_CLIENT_ID и DURAK_DISCORD_TOKEN или сессию места в DURAK_DISCORD_ACCOUNTS")?;
    let token = selected.and_then(|v| v["token"].as_str()).map(str::to_owned).or_else(|| env::var("DURAK_DISCORD_TOKEN").ok()).filter(|v| !v.trim().is_empty()).ok_or("Укажите DURAK_DISCORD_CLIENT_ID и DURAK_DISCORD_TOKEN или сессию места в DURAK_DISCORD_ACCOUNTS")?;
    let seat = Arc::new(Mutex::new(Seat {
        label: label.clone(),
        name,
        id,
        token,
        ..Default::default()
    }));
    {
        let mut m = seats().lock().unwrap();
        if m.contains_key(&label) {
            return Err("Место уже открыто".into());
        }
        m.insert(label.clone(), seat.clone());
    }
    start_sse(seat.clone());
    let until = Instant::now() + Duration::from_secs(5);
    while Instant::now() < until {
        if seat.lock().unwrap().streaming {
            let s = seat.lock().unwrap();
            return Ok(result(
                format!("Открыто место {}.", label),
                json!({"seat":label,"name":s.name,"clientId":s.id,"connected":true,"account":s.account,"room":s.game.as_ref().map(compact)}),
            ));
        }
        thread::sleep(Duration::from_millis(20));
    }
    seat.lock().unwrap().closed = true;
    seats().lock().unwrap().remove(&label);
    Err("Не удалось открыть SSE поток".into())
}
fn wait_watch(seat: &Arc<Mutex<Seat>>, until: &str, timeout: u64) -> Result<Value, String> {
    let (id, token, since) = {
        let s = seat.lock().unwrap();
        (s.id.clone(), s.token.clone(), s.seq)
    };
    let u = format!(
        "{}/api/watch?id={}&token={}&since={}&until={}&timeout={}",
        base(),
        urlencoding(&id),
        urlencoding(&token),
        since,
        urlencoding(until),
        timeout
    );
    let v = client()
        .get(u)
        .send()
        .map_err(|e| e.without_url().to_string())?
        .json::<Value>()
        .map_err(|e| e.to_string())?;
    if v["state"].is_object() {
        update_game(seat, v["state"].clone())
    }
    Ok(v)
}
fn action(a: &Value, act: &str) -> Result<Value, String> {
    let seat = seat_arg(a)?;
    let (id, token, before) = {
        let s = seat.lock().unwrap();
        (s.id.clone(), s.token.clone(), s.seq)
    };
    let mut b = json!({"clientId":id,"token":token,"action":act});
    for k in ["cardId", "targetCardId"] {
        if let Some(v) = a.get(k) {
            b[k] = v.clone()
        }
    }
    let v = body("/api/action", &b)?;
    if let Some(st) = v.get("state") {
        update_game(&seat, st.clone())
    } else {
        wait_seq(&seat, before, Duration::from_secs(3))?;
    }
    let st = seat.lock().unwrap().game.clone().unwrap_or(Value::Null);
    Ok(result(
        format!("{}: принято.", act),
        json!({"seat":seat.lock().unwrap().label,"action":act,"snapshot":compact(&st),"state":if a["verbose"].as_bool().unwrap_or(false){st}else{Value::Null}}),
    ))
}

fn wait_seq(seat: &Arc<Mutex<Seat>>, before: u64, timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        {
            let s = seat.lock().unwrap();
            if s.seq > before {
                return Ok(());
            }
            if s.closed {
                return Err("Место закрыто".into());
            }
        }
        if Instant::now() >= deadline {
            return Err("Нет нового снимка SSE после действия".into());
        }
        thread::sleep(Duration::from_millis(5));
    }
}

fn card_text(c: &Value) -> String {
    format!(
        "{}{}",
        c["rank"].as_str().unwrap_or(""),
        c["suit"].as_str().unwrap_or("")
    )
}
fn rank(c: &Value) -> usize {
    [
        "2", "3", "4", "5", "6", "7", "8", "9", "10", "В", "Д", "К", "Т",
    ]
    .iter()
    .position(|r| Some(*r) == c["rank"].as_str())
    .unwrap_or(0)
}
fn options(st: &Value) -> (Vec<Value>, Vec<Value>, Vec<Value>) {
    let hand = st["hand"].as_array().cloned().unwrap_or_default();
    let table = st["table"].as_array().cloned().unwrap_or_default();
    let attacks = if st["canAttack"] == true {
        hand.iter()
            .filter(|c| {
                table.is_empty()
                    || table.iter().any(|r| {
                        r["attack"]["rank"] == c["rank"]
                            || (!r["defense"].is_null() && r["defense"]["rank"] == c["rank"])
                    })
            })
            .cloned()
            .collect()
    } else {
        vec![]
    };
    let mut defenses = Vec::new();
    if st["canDefend"] == true {
        for row in &table {
            if !row["defense"].is_null() {
                continue;
            }
            let attack = &row["attack"];
            for c in &hand {
                if (attack["suit"] == c["suit"] && rank(c) > rank(attack))
                    || (c["suit"] == st["trump"] && attack["suit"] != st["trump"])
                {
                    defenses.push(json!({"row":row,"card":c}));
                }
            }
        }
    }
    defenses.sort_by_key(|v| (v["card"]["suit"] == st["trump"], rank(&v["card"])));
    let transfers = if st["canTransfer"] == true && !table.is_empty() {
        hand.iter()
            .filter(|c| c["rank"] == table[0]["attack"]["rank"])
            .cloned()
            .collect()
    } else {
        vec![]
    };
    (attacks, defenses, transfers)
}
fn hints(a: &Value) -> Result<Value, String> {
    let q = seat_arg(a)?;
    let s = q.lock().unwrap();
    let st = s.game.as_ref().ok_or("Место вне стола")?;
    let (attacks, defenses, transfers) = options(st);
    let attack: Vec<_> = attacks
        .iter()
        .map(|c| json!({"card":card_text(c),"id":c["id"],"trump":c["suit"]==st["trump"]}))
        .collect();
    let defend:Vec<_>=defenses.iter().map(|d|json!({"attack":card_text(&d["row"]["attack"]),"targetCardId":d["row"]["attack"]["id"],"card":card_text(&d["card"]),"cardId":d["card"]["id"],"trump":d["card"]["suit"]==st["trump"]})).collect();
    let transfer: Vec<_> = transfers
        .iter()
        .map(|c| json!({"card":card_text(c),"id":c["id"]}))
        .collect();
    Ok(result(
        "Подсказки".into(),
        json!({"seat":s.label,"role":st["you"]["role"],"hand":st["hand"],"trump":st["trump"],"attack":attack,"defend":defend,"transfer":transfer,"canTake":st["canTake"],"canPass":st["canPass"],"canTransfer":st["canTransfer"]}),
    ))
}
fn my_move(st: &Value) -> Option<Value> {
    if st["room"]["status"] != "playing"
        || st["you"]["finished"] == true
        || st["you"]["left"] == true
    {
        return None;
    }
    let mine = &st["you"]["id"];
    if st["takePending"] == true {
        return if st["canPass"] == true {
            Some(json!({"action":"pass"}))
        } else {
            None
        };
    }
    let table = st["table"].as_array()?;
    let (mut attacks, defenses, _) = options(st);
    if table.iter().any(|r| r["defense"].is_null()) {
        if &st["defenderId"] != mine {
            return None;
        }
        if let Some(d) = defenses.first() {
            return Some(
                json!({"action":"defend","cardId":d["card"]["id"],"targetCardId":d["row"]["attack"]["id"]}),
            );
        }
        return if st["canTake"] == true {
            Some(json!({"action":"take"}))
        } else {
            None
        };
    }
    attacks.sort_by_key(|c| (c["suit"] == st["trump"], rank(c)));
    if let Some(c) = attacks.first() {
        return Some(json!({"action":"attack","cardId":c["id"]}));
    }
    if st["canPass"] == true {
        return Some(json!({"action":"pass"}));
    }
    None
}
fn follow(a: &Value) -> Result<Value, String> {
    let q = seat_arg(a)?;
    let label = q.lock().unwrap().label.clone();
    let until = a["until"].as_str().unwrap_or("finished");
    if until != "finished" && until != "turn" {
        return Err("until должен быть finished или turn".into());
    }
    let max = a["maxSteps"].as_u64().unwrap_or(400).clamp(1, 2000);
    let duration =
        Duration::from_millis(a["totalMs"].as_u64().unwrap_or(240000).clamp(1000, 900000));
    let start = Instant::now();
    let mut steps = 0;
    let mut log = Vec::new();
    let reason = loop {
        let st = q.lock().unwrap().game.clone().ok_or("Место вне стола")?;
        if st["room"]["status"] == "finished" || !st["finished"].is_null() {
            break "finished";
        }
        if steps >= max {
            break "steps";
        }
        if start.elapsed() >= duration {
            break "timeout";
        }
        if let Some(mut m) = my_move(&st) {
            m["seat"] = label.clone().into();
            let act = m["action"].as_str().unwrap().to_string();
            action(&m, &act)?;
            steps += 1;
            log.push(m);
            if log.len() > 60 {
                log.remove(0);
            }
        } else {
            if until == "turn" {
                break "opponent";
            }
            let since = st["seq"].as_u64().unwrap_or(0);
            let left = duration.saturating_sub(start.elapsed()).as_millis() as u64;
            let _ = wait_watch_since(&q, "your-turn", left.min(25000).max(200), since)?;
        }
    };
    let st = q.lock().unwrap().game.clone().unwrap_or(Value::Null);
    Ok(result(
        format!("Партия: {}, действий {}", reason, steps),
        json!({"seat":label,"roomId":st["room"]["id"],"steps":steps,"finished":reason=="finished","reason":reason,"loser":st["finished"]["loser"],"payout":st["payout"],"waitedMs":start.elapsed().as_millis(),"actions":log,"snapshot":compact(&st),"state":if a["verbose"]==true{st}else{Value::Null}}),
    ))
}
fn wait_watch_since(
    seat: &Arc<Mutex<Seat>>,
    until: &str,
    timeout: u64,
    since: u64,
) -> Result<Value, String> {
    let (id, token) = {
        let s = seat.lock().unwrap();
        (s.id.clone(), s.token.clone())
    };
    let url = format!(
        "{}/api/watch?id={}&token={}&since={}&until={}&timeout={}",
        base(),
        urlencoding(&id),
        urlencoding(&token),
        since,
        urlencoding(until),
        timeout
    );
    let r = client().get(url).send().map_err(|e| e.without_url().to_string())?;
    let status = r.status();
    let v: Value = r.json().map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(v["error"].as_str().unwrap_or("watch HTTP error").into());
    }
    if v["state"].is_object() {
        let seq = v["state"]["seq"].as_u64().unwrap_or(0);
        let current = seat.lock().unwrap().seq;
        if seq >= current {
            update_game(seat, v["state"].clone());
        }
    }
    Ok(v)
}
struct AutoCleanup {
    labels: Vec<String>,
    keep: bool,
}
impl Drop for AutoCleanup {
    fn drop(&mut self) {
        if !self.keep {
            for label in &self.labels {
                if let Some(q) = seats().lock().unwrap().remove(label) {
                    q.lock().unwrap().closed = true;
                }
            }
        }
    }
}
fn autoplay(a: &Value) -> Result<Value, String> {
    let count = a["seats"].as_u64().unwrap_or(2).clamp(2, 6);
    let credentials: Value = serde_json::from_str(&env::var("DURAK_DISCORD_ACCOUNTS").unwrap_or_default()).unwrap_or(Value::Null);
    let accounts = credentials.as_object().ok_or("Для autoplay нужны отдельные сессии auto1...autoN в DURAK_DISCORD_ACCOUNTS")?;
    let mut ids = std::collections::HashSet::new();
    let mut tokens = std::collections::HashSet::new();
    for i in 1..=count {
        let account = accounts.get(&format!("auto{}", i)).ok_or("Для autoplay нужны отдельные сессии auto1...autoN в DURAK_DISCORD_ACCOUNTS")?;
        let id = account["clientId"].as_str().unwrap_or("");
        let token = account["token"].as_str().unwrap_or("");
        if id.trim().is_empty() || token.trim().is_empty() || !ids.insert(id.to_string()) || !tokens.insert(token.to_string()) {
            return Err("Для autoplay нужны отдельные сессии auto1...autoN в DURAK_DISCORD_ACCOUNTS".into());
        }
    }
    let max = a["maxSteps"].as_u64().unwrap_or(300).clamp(10, 2000);
    let mut cleanup = AutoCleanup {
        labels: Vec::new(),
        keep: a["keepSeats"] == true,
    };
    for i in 1..=count {
        let label = format!("auto{}", i);
        if seats().lock().unwrap().contains_key(&label) { return Err("Место autoplay уже открыто".into()); }
        let name = format!(
            "{} {}",
            a["name"]
                .as_str()
                .unwrap_or("Бот")
                .chars()
                .take(16)
                .collect::<String>(),
            i
        );
        connect(&json!({"seat":label,"name":name}))?;
        cleanup.labels.push(label);
    }
    let host = &cleanup.labels[0];
    let room = tool(
        "durak_room_create",
        &json!({"seat":host,"name":"MCP тест","mode":a["mode"],"deckSize":a["deckSize"],"bet":a["bet"]}),
    )?;
    let code = room["structuredContent"]["code"].clone();
    for label in cleanup.labels.iter().skip(1) {
        tool("durak_room_join", &json!({"seat":label,"code":code}))?;
    }
    action(&json!({"seat":host}), "start")?;
    let mut log = Vec::new();
    let mut seq = seat_arg(&json!({"seat":host}))?.lock().unwrap().seq;
    let mut final_state = Value::Null;
    for _ in 0..max {
        for label in &cleanup.labels {
            let q = seat_arg(&json!({"seat":label}))?;
            if q.lock().unwrap().seq < seq {
                wait_seq(&q, seq - 1, Duration::from_secs(3))?;
            }
        }
        let mut chosen = None;
        for label in &cleanup.labels {
            let q = seat_arg(&json!({"seat":label}))?;
            let st = q.lock().unwrap().game.clone().unwrap_or(Value::Null);
            if st["room"]["status"] == "finished" || !st["finished"].is_null() {
                final_state = st;
                break;
            }
            if let Some(m) = my_move(&st) {
                chosen = Some((label.clone(), m));
                break;
            }
        }
        if !final_state.is_null() {
            break;
        }
        let (label, mut m) = chosen.ok_or("Боты не нашли допустимый ход")?;
        m["seat"] = label.clone().into();
        let act = m["action"].as_str().unwrap().to_string();
        action(&m, &act)?;
        let q = seat_arg(&json!({"seat":label}))?;
        let st = q.lock().unwrap().game.clone().unwrap_or(Value::Null);
        seq = st["seq"].as_u64().unwrap_or(seq);
        final_state = st;
        log.push(m);
        if final_state["room"]["status"] == "finished" || !final_state["finished"].is_null() {
            break;
        }
        final_state = Value::Null;
    }
    if final_state.is_null() {
        final_state = seat_arg(&json!({"seat":host}))?
            .lock()
            .unwrap()
            .game
            .clone()
            .unwrap_or(Value::Null);
    }
    let finished =
        final_state["room"]["status"] == "finished" || !final_state["finished"].is_null();
    let balances:Vec<Value>=cleanup.labels.iter().map(|l|{let q=seat_arg(&json!({"seat":l})).unwrap();let s=q.lock().unwrap();json!({"seat":l,"balance":s.account["balance"],"payout":s.game.as_ref().map(|v|v["payout"].clone())})}).collect();
    Ok(result(
        format!(
            "Стол {}: {}",
            code,
            if finished {
                "партия завершена"
            } else {
                "лимит ходов"
            }
        ),
        json!({"code":code,"seats":cleanup.labels,"steps":log.len(),"finished":finished,"loser":final_state["finished"]["loser"],"deck":final_state["deck"],"table":final_state["table"].as_array().map(|v|v.len()),"hands":final_state["players"].as_array().map(|v|v.iter().map(|p|p["cards"].as_u64().unwrap_or(0)).sum::<u64>()),"balances":balances,"actions":log,"state":final_state}),
    ))
}

fn tool(name: &str, a: &Value) -> Result<Value, String> {
    match name {
        "durak_connect" => connect(a),
        "durak_seats" => {
            let m = seats().lock().unwrap();
            let list:Vec<Value>=m.values().map(|q|{let s=q.lock().unwrap();json!({"seat":s.label,"name":s.name,"clientId":s.id,"connected":s.streaming,"account":s.account,"room":s.game.as_ref().map(|g|g["room"].clone()),"lastError":s.last_error})}).collect();
            Ok(result("Открытые места".into(), json!({"seats":list})))
        }
        "durak_status" => {
            let v = client()
                .get(format!("{}/api/status", base()))
                .send()
                .map_err(|e| e.to_string())?
                .json::<Value>()
                .map_err(|e| e.to_string())?;
            Ok(result("Статус сервиса".into(), v))
        }
        "durak_lobby" => {
            let lobby = if let Ok(q) = seat_arg(a) {
                q.lock().unwrap().lobby.clone()
            } else {
                None
            };
            let v = match lobby {
                Some(v) => v,
                None => client()
                    .get(format!("{}/api/status", base()))
                    .send()
                    .map_err(|e| e.to_string())?
                    .json::<Value>()
                    .map_err(|e| e.to_string())?,
            };
            Ok(result("Лобби".into(), v))
        }
        "durak_disconnect" | "durak_room_leave" => {
            let q = seat_arg(a)?;
            let (id, token, label) = {
                let s = q.lock().unwrap();
                (s.id.clone(), s.token.clone(), s.label.clone())
            };
            if name == "durak_room_leave" || a["leave"] == true {
                body("/api/leave", &json!({"clientId":id,"token":token}))?;
                q.lock().unwrap().game = None;
            }
            if name == "durak_disconnect" {
                q.lock().unwrap().closed = true;
                seats().lock().unwrap().remove(&label);
            }
            Ok(result("Готово".into(), json!({"seat":label})))
        }
        "durak_room_create" | "durak_room_join" | "durak_profile" => {
            let q = seat_arg(a)?;
            let (id, token, player, label) = {
                let s = q.lock().unwrap();
                (
                    s.id.clone(),
                    s.token.clone(),
                    s.name.clone(),
                    s.label.clone(),
                )
            };
            let mut b = json!({"clientId":id,"token":token,"playerName":player});
            let path = if name == "durak_room_create" {
                for k in ["name", "mode", "deckSize", "bet"] {
                    if let Some(v) = a.get(k) {
                        b[k] = v.clone()
                    }
                }
                "/api/rooms"
            } else if name == "durak_room_join" {
                b["roomId"] = a.get("code").cloned().ok_or("Укажите code")?;
                "/api/join"
            } else {
                b["name"] = a.get("name").cloned().ok_or("Укажите name")?;
                "/api/profile"
            };
            let v = body(path, &b)?;
            if name == "durak_profile" {
                q.lock().unwrap().name = a["name"].as_str().unwrap_or("").into();
            } else {
                for _ in 0..100 {
                    if q.lock()
                        .unwrap()
                        .game
                        .as_ref()
                        .map(|g| g["room"]["id"] == v["room"]["id"])
                        .unwrap_or(false)
                    {
                        break;
                    }
                    thread::sleep(Duration::from_millis(20));
                }
            }
            let st = q.lock().unwrap().game.clone().unwrap_or(Value::Null);
            Ok(result(
                "Готово".into(),
                json!({"seat":label,"code":v["room"]["id"],"room":v["room"],"account":v["account"],"snapshot":compact(&st),"state":st}),
            ))
        }
        "durak_start" => action(a, "start"),
        "durak_action" => {
            let act = a["action"].as_str().unwrap_or("");
            if !["attack", "defend", "transfer", "take", "pass"].contains(&act) {
                return Err("Неверное действие".into());
            }
            action(a, act)
        }
        "durak_state" => {
            let q = seat_arg(a)?;
            let s = q.lock().unwrap();
            let st = s.game.clone().unwrap_or(Value::Null);
            Ok(result(
                "Состояние стола".into(),
                json!({"seat":s.label,"snapshot":compact(&st),"state":st}),
            ))
        }
        "durak_wait" | "durak_watch" => {
            let q = seat_arg(a)?;
            let default = if name == "durak_watch" {
                "your-turn"
            } else {
                "any"
            };
            let until = a["until"].as_str().unwrap_or(default);
            if !["any", "game", "your-turn", "opponent", "finished"].contains(&until) {
                return Err("Неверный until".into());
            }
            let total = if name == "durak_watch" {
                a["totalMs"].as_u64().unwrap_or(180000).clamp(200, 600000)
            } else {
                a["timeoutMs"].as_u64().unwrap_or(5000).clamp(200, 120000)
            };
            let chunk = a["chunkMs"].as_u64().unwrap_or(25000).clamp(200, 60000);
            let start = Instant::now();
            loop {
                let rem = total.saturating_sub(start.elapsed().as_millis() as u64);
                let v = wait_watch(&q, until, rem.min(chunk).max(200))?;
                if v["hit"] == true
                    || start.elapsed().as_millis() as u64 >= total
                    || v["roomId"].is_null()
                {
                    return Ok(result("Ожидание события".into(), v));
                }
            }
        }
        "durak_admin" => {
            let mut b = a.clone();
            if b["key"].is_null() {
                b["key"] = env::var("DURAK_ADMIN_KEY")
                    .or_else(|_| env::var("ADMIN_KEY"))
                    .unwrap_or_default()
                    .into()
            }
            let v = body("/api/admin", &b)?;
            Ok(result("Администрирование".into(), v))
        }
        "durak_hints" => hints(a),
        "durak_follow" => follow(a),
        "durak_autoplay" => autoplay(a),
        _ => Err("Неизвестный инструмент".into()),
    }
}
fn response(request: Value) -> Option<Value> {
    let obj = match request.as_object() {
        Some(o) => o,
        None => {
            return Some(
                json!({"jsonrpc":"2.0","id":null,"error":{"code":-32600,"message":"Invalid Request"}}),
            )
        }
    };
    let id = obj.get("id").cloned();
    if obj.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
        || obj.get("method").and_then(Value::as_str).is_none()
    {
        return Some(
            json!({"jsonrpc":"2.0","id":id,"error":{"code":-32600,"message":"Invalid Request"}}),
        );
    }
    let id = id?;
    let method = obj["method"].as_str().unwrap();
    let value = match method {
        "initialize" => {
            let req = request["params"]["protocolVersion"]
                .as_str()
                .unwrap_or("2024-11-05");
            let v = if ["2024-11-05", "2025-03-26", "2025-06-18"].contains(&req) {
                req
            } else {
                "2024-11-05"
            };
            json!({"protocolVersion":v,"capabilities":{"tools":{}},"serverInfo":{"name":"durak","version":"0.1.0"}})
        }
        "ping" => json!({}),
        "tools/list" => {
            json!({"tools":serde_json::from_str::<Value>(include_str!("../tools.json")).unwrap()})
        }
        "tools/call" => {
            let name = request["params"]["name"].as_str().unwrap_or("");
            let tools: Value = serde_json::from_str(include_str!("../tools.json")).unwrap();
            if !tools.as_array().unwrap().iter().any(|t| t["name"] == name) {
                return Some(
                    json!({"jsonrpc":"2.0","id":id,"error":{"code":-32602,"message":"Unknown or missing tool name"}}),
                );
            }
            let args = request["params"]
                .get("arguments")
                .cloned()
                .unwrap_or(json!({}));
            tool(name, &args).unwrap_or_else(error)
        }
        _ => {
            return Some(
                json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Method not found"}}),
            )
        }
    };
    Some(json!({"jsonrpc":"2.0","id":id,"result":value}))
}
fn main() -> io::Result<()> {
    let out = Arc::new(Mutex::new(io::BufWriter::new(io::stdout())));
    let mut jobs = Vec::new();
    for line in io::stdin().lock().lines() {
        let line = line?;
        let out = out.clone();
        jobs.push(thread::spawn(move||{let reply=match serde_json::from_str(&line){Ok(v)=>response(v),Err(_)=>Some(json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Parse error"}}))};if let Some(v)=reply{let mut w=out.lock().unwrap();let _=serde_json::to_writer(&mut *w,&v);let _=w.write_all(b"\n");let _=w.flush();}}));
    }
    for job in jobs {
        let _ = job.join();
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn protocol() {
        assert!(response(json!({"jsonrpc":"2.0","method":"notifications/initialized"})).is_none());
        assert_eq!(
            response(json!({"jsonrpc":"2.0","id":1,"method":"tools/list"})).unwrap()["result"]
                ["tools"]
                .as_array()
                .unwrap()
                .len(),
            18
        );
        assert_eq!(
            response(json!({"jsonrpc":"2.0","id":1,"method":"bogus"})).unwrap()["error"]["code"],
            -32601
        );
    }

}

#[cfg(test)]
mod follow_tests {
    use super::*;

    #[test]
    fn follows_permissions_for_other_attackers() {
        let mut state = json!({"room":{"status":"playing"},"you":{"id":"c","finished":false,"left":false},"attackerId":"a","defenderId":"b","takePending":false,"table":[{"attack":{"id":"6♠","rank":"6","suit":"♠"},"defense":{"id":"7♠","rank":"7","suit":"♠"}}],"hand":[{"id":"6♥","rank":"6","suit":"♥"}],"trump":"♣","canAttack":true,"canDefend":false,"canPass":true});
        assert_eq!(my_move(&state).unwrap()["action"], "attack");
        state["canAttack"] = json!(false);
        assert_eq!(my_move(&state).unwrap()["action"], "pass");
        state["takePending"] = json!(true);
        assert_eq!(my_move(&state).unwrap()["action"], "pass");
        state["canPass"] = json!(false);
        assert!(my_move(&state).is_none());
    }
}
