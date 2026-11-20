use crate::{
    seat::Seat,
    strategy::{self, card_text, defense_options, dump_options, transfer_options, Ledger},
};
use serde_json::{json, Value};
pub(super) fn compact(s: &Value, ledger: Option<&Ledger>) -> Value {
    if s.is_null() {
        return Value::Null;
    }
    let hand: Vec<String> = array(s, "hand").iter().map(card_text).collect();
    let counts: Vec<String> = array(s, "players")
        .iter()
        .map(|p| format!("{}:{}", p["name"].as_str().unwrap_or(""), p["cards"]))
        .collect();
    let mut view = json!({"room":s["room"]["id"],"status":s["room"]["status"],"role":s["you"]["role"].as_str().unwrap_or("spectator"),"turn":s["turnName"],"attacker":s["attackerName"],"defender":s["defenderName"],"trump":s["trump"],"deck":s["deck"],"table":table_text(s),"hand":hand,"counts":counts,"actions":flags(s),"message":s["message"],"seq":s["seq"],"finished":s["finished"]});
    if let Some(ledger) = ledger {
        if s["room"]["status"] == "playing" {
            view["hidden"] = ledger.summary(s);
        }
    }
    view
}
pub(super) fn result(text: String, data: Value) -> Value {
    json!({"content":[{"type":"text","text":text}],"structuredContent":data})
}
pub(super) fn error(text: String) -> Value {
    json!({"content":[{"type":"text","text":text}],"isError":true})
}
pub(super) fn hint_payload(seat: &Seat, st: &Value) -> Value {
    let attack: Vec<_> = strategy::attach_options(st)
        .iter()
        .map(|c| json!({"card":card_text(c),"id":c["id"],"trump":c["suit"]==st["trump"]}))
        .collect();
    let defend: Vec<_> = defense_options(st)
        .iter()
        .map(|(row, card)| json!({"attack":card_text(&row["attack"]),"targetCardId":row["attack"]["id"],"card":card_text(card),"cardId":card["id"],"trump":card["suit"]==st["trump"]}))
        .collect();
    let transfer: Vec<_> = transfer_options(st)
        .iter()
        .map(|c| json!({"card":card_text(c),"id":c["id"]}))
        .collect();
    let dump: Vec<_> = dump_options(st)
        .iter()
        .map(|c| json!({"card":card_text(c),"id":c["id"]}))
        .collect();
    json!({"seat":seat.label,"role":st["you"]["role"],"hand":st["hand"],"trump":st["trump"],"attack":attack,"defend":defend,"transfer":transfer,"dump":dump,"canTake":st["canTake"],"canPass":st["canPass"],"canTransfer":st["canTransfer"],"hidden":seat.ledger.summary(st)})
}

fn array<'a>(s: &'a Value, key: &str) -> &'a [Value] {
    s[key].as_array().map(Vec::as_slice).unwrap_or(&[])
}

fn flags(s: &Value) -> Vec<&'static str> {
    [
        ("canAttack", "можно подкинуть"),
        ("canDefend", "можно отбиваться"),
        ("canTransfer", "можно перевести"),
        ("canTake", "можно взять карты"),
        ("canPass", "можно сказать «бито»"),
        ("canStart", "можно начать игру"),
        ("takePending", "решено взять карты"),
    ]
    .iter()
    .filter_map(|(key, text)| (s[*key] == true).then_some(*text))
    .collect()
}

fn table_text(s: &Value) -> String {
    let rows: Vec<String> = array(s, "table")
        .iter()
        .map(|row| {
            format!(
                "{}{}",
                card_text(&row["attack"]),
                if row["defense"].is_null() {
                    "!".to_owned()
                } else {
                    format!("←{}", card_text(&row["defense"]))
                }
            )
        })
        .collect();
    if rows.is_empty() {
        "пуст".into()
    } else {
        rows.join(" ")
    }
}

pub(super) fn without_log(st: &Value) -> Value {
    let mut state = st.clone();
    if let Some(obj) = state.as_object_mut() {
        obj.remove("log");
    }
    state
}

pub(super) fn format_state(seat: &Seat, st: &Value, verbose: bool) -> String {
    if st.is_null() {
        return format!("{} ({}): вне стола", seat.label, seat.name);
    }
    let hand: Vec<_> = array(st, "hand").iter().map(card_text).collect();
    let trump = if st["trumpCard"].is_object() {
        card_text(&st["trumpCard"])
    } else {
        st["trump"].as_str().unwrap_or("—").to_owned()
    };
    let text = |key: &str| st[key].as_str().filter(|s| !s.is_empty()).unwrap_or("—");
    let actions = flags(st);
    let mut lines = vec![
        format!(
            "{} ({}) · стол {} «{}» · {}",
            seat.label,
            seat.name,
            st["room"]["id"].as_str().unwrap_or("—"),
            st["room"]["name"].as_str().unwrap_or("—"),
            st["room"]["status"].as_str().unwrap_or("—")
        ),
        format!(
            "Режим: {} · колода {} · ставка {}",
            if st["room"]["mode"] == "transfer" {
                "переводной"
            } else {
                "подкидной"
            },
            st["room"]["deckSize"],
            st["room"]["bet"]
        ),
        format!("Козырь {} · в колоде {}", trump, st["deck"]),
        format!(
            "Атакует {} · защищается {} · ход {}",
            text("attackerName"),
            text("defenderName"),
            text("turnName")
        ),
        if st["you"].is_object() {
            format!(
                "Вы: {}, карт {}",
                st["you"]["role"].as_str().unwrap_or("—"),
                hand.len()
            )
        } else {
            "Вы: зритель".into()
        },
        format!("Стол: {}", table_text(st)),
        format!(
            "Рука: {}",
            if hand.is_empty() {
                "—".into()
            } else {
                hand.join(" ")
            }
        ),
        format!(
            "Действия: {}",
            if actions.is_empty() {
                "—".into()
            } else {
                actions.join(", ")
            }
        ),
    ];
    if let Some(message) = st["message"].as_str().filter(|s| !s.is_empty()) {
        lines.push(format!("Сообщение: {}", message));
    }
    if !st["finished"].is_null() && st["finished"] != false {
        lines.push(format!(
            "Итог: дурак — {}",
            st["finished"]["loser"].as_str().unwrap_or("—")
        ));
    }
    if verbose {
        lines.push(format!(
            "\nСостояние:\n{}",
            serde_json::to_string_pretty(&without_log(st)).unwrap_or_default()
        ));
    }
    lines.join("\n")
}

pub(super) fn wait_result(
    seat: &Seat,
    until: &str,
    value: &Value,
    waited_ms: u64,
    verbose: bool,
) -> Value {
    let st = seat.game.as_ref().unwrap_or(&Value::Null);
    let your_turn = st["you"].is_object()
        && [
            "canAttack",
            "canDefend",
            "canTransfer",
            "canTake",
            "canPass",
            "canStart",
        ]
        .iter()
        .any(|key| st[*key] == true);
    let events = array(value, "events");
    let short: Vec<Value> = events
        .iter()
        .map(|event| {
            let mut item = json!({});
            for (src, dst) in [
                ("seq", "seq"),
                ("action", "action"),
                ("cardId", "card"),
                ("targetCardId", "target"),
            ] {
                if let Some(v) = event.get(src) {
                    item[dst] = v.clone();
                }
            }
            if event["actorId"].as_str().is_some_and(|id| id == seat.id) {
                item["actor"] = json!("я");
            } else if let Some(v) = event.get("actorName") {
                item["actor"] = v.clone();
            }
            item
        })
        .collect();
    let via = value["via"].as_str().unwrap_or("sse");
    let source = match via {
        "hook" => "хук /api/watch",
        "cache" => "уже случилось до вызова",
        _ => "SSE",
    };
    let hit = value["hit"] == true;
    let mut lines = vec![if hit && via == "cache" {
        format!(
            "Условие «{}» выполнилось ещё до вызова: отдаю накопленные события и текущий снимок.",
            until
        )
    } else {
        format!(
            "Хук «{}» {} за {:.1} с ({}).",
            until,
            if hit {
                "сработал"
            } else {
                "не сработал"
            },
            waited_ms as f64 / 1000.0,
            source
        )
    }];
    let descriptions: Vec<String> = events
        .iter()
        .map(|event| {
            let action = event["action"].as_str().unwrap_or("");
            let verb = match action {
                "attack" => "ходит",
                "defend" => "отбивает",
                "transfer" => "переводит",
                "take" => "берёт карты",
                "pass" => "говорит «бито»",
                "start" => "начинает партию",
                "create" => "создаёт стол",
                "join" => "садится за стол",
                "leave" => "покидает стол",
                "connect" => "подключается",
                "disconnect" => "отключается",
                "rename" => "меняет имя",
                "admin" => "правит профиль",
                "cancel" => "отменяет партию",
                "sync" => "обновляет стол",
                _ => action,
            };
            format!(
                "{} {}{}{}",
                event["actorName"].as_str().unwrap_or("кто-то"),
                verb,
                event["cardId"]
                    .as_str()
                    .map(|s| format!(" {}", s))
                    .unwrap_or_default(),
                event["targetCardId"]
                    .as_str()
                    .map(|s| format!(" на {}", s))
                    .unwrap_or_default()
            )
        })
        .collect();
    lines.push(format!(
        "События: {}",
        if descriptions.is_empty() {
            "новых действий не было".into()
        } else {
            descriptions.join("; ")
        }
    ));
    if your_turn {
        let hints = hint_payload(seat, st);
        let attacks: Vec<&str> = array(&hints, "attack")
            .iter()
            .filter_map(|v| v["card"].as_str())
            .collect();
        lines.push(format!(
            "\nАтака: {}",
            if attacks.is_empty() {
                "нет".into()
            } else {
                attacks.join(" ")
            }
        ));
    }
    lines.push(format!("\n{}", format_state(seat, st, verbose)));
    let mut data = json!({"seat":seat.label,"until":until,"hit":hit,"via":via,"seq":value["seq"],"since":value["since"],"roomId":value["roomId"],"waitedMs":waited_ms,"events":short,"yourTurn":your_turn,"snapshot":compact(st, Some(&seat.ledger)),"lastError":seat.last_error});
    if verbose {
        data["state"] = without_log(st);
    }
    result(lines.join("\n"), data)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> Value {
        json!({"room":{"id":"ABC","status":"waiting"},"you":{"role":"attacker"},"hand":[{"rank":"6","suit":"♠"}],"players":[{"name":"Аня","cards":6}],"table":[{"attack":{"rank":"6","suit":"♠"},"defense":{"rank":"7","suit":"♠"}},{"attack":{"rank":"8","suit":"♥"},"defense":null}],"canAttack":true,"canPass":true,"takePending":true,"log":["private"]})
    }

    #[test]
    fn compact_matches_node_strings() {
        let snap = compact(&state(), None);
        assert_eq!(snap["table"], "6♠←7♠ 8♥!");
        assert_eq!(snap["hand"], json!(["6♠"]));
        assert_eq!(snap["counts"], json!(["Аня:6"]));
        assert_eq!(
            snap["actions"],
            json!([
                "можно подкинуть",
                "можно сказать «бито»",
                "решено взять карты"
            ])
        );
        let mut empty = state();
        empty["table"] = json!([]);
        empty["you"] = Value::Null;
        assert_eq!(compact(&empty, None)["table"], "пуст");
        assert_eq!(compact(&empty, None)["role"], "spectator");
    }

    #[test]
    fn wait_response_shortens_events_and_respects_verbose() {
        let seat = Seat {
            label: "p1".into(),
            name: "Аня".into(),
            id: "me".into(),
            game: Some(state()),
            ..Default::default()
        };
        let wait = json!({"hit":true,"via":"hook","seq":9,"since":8,"roomId":"ABC","events":[{"seq":9,"action":"attack","actorId":"me","actorName":"Аня","cardId":"6♠"}]});
        let response = wait_result(&seat, "your-turn", &wait, 120, false);
        let data = &response["structuredContent"];
        assert_eq!(
            data["events"],
            json!([{"seq":9,"action":"attack","actor":"я","card":"6♠"}])
        );
        assert_eq!(data["waitedMs"], 120);
        assert_eq!(data["yourTurn"], true);
        assert!(data.get("state").is_none());
        assert!(response["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Рука: 6♠"));
        let verbose = wait_result(&seat, "any", &wait, 0, true);
        assert!(verbose["structuredContent"]["state"].get("log").is_none());
        assert_eq!(verbose["structuredContent"]["state"]["room"]["id"], "ABC");
    }
}
