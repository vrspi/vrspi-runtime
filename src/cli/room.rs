//! `vrspi room ...` — company rooms over the socket API.
//!
//! The host drives rooms from an ordinary terminal; an agent uses the same
//! commands from inside its pane, where `--as-agent` marks it as acting for
//! its own seat rather than as the host.

use crate::api::schema::{
    Method, Request, RoomAllowanceParams, RoomCreateParams, RoomDeliveryAckParams,
    RoomEventListParams, RoomLifecycleParams, RoomLifecycleValue, RoomListParams,
    RoomMemberAddParams, RoomMemberBindParams, RoomMemberGrantParams, RoomMemberTargetParams,
    RoomMemoryPutParams, RoomMemorySearchParams, RoomMemoryTargetParams, RoomPostParams,
    RoomTargetParams,
};

/// Pane ID of the calling agent, when the caller asked to act as its seat.
///
/// Absent this the server treats the caller as the host, so an agent must opt
/// in explicitly rather than being inferred from its environment.
fn agent_caller(args: &[String]) -> Option<String> {
    if !args.iter().any(|a| a == "--as-agent") {
        return None;
    }
    crate::brand::env_var(
        crate::integration::VRSPI_PANE_ID_ENV_VAR,
        crate::integration::HERDR_PANE_ID_ENV_VAR,
    )
    .ok()
    .filter(|value| !value.trim().is_empty())
}

/// Positional arguments only.
///
/// A `--flag value` pair consumes its value, so an objective like
/// "Improve onboarding" is not mistaken for a positional argument.
/// `--flag=value` and bare boolean flags consume nothing extra.
fn without_flags(args: &[String]) -> Vec<String> {
    const VALUE_FLAGS: &[&str] = &[
        "--objective",
        "--workspace",
        "--role",
        "--root",
        "--after",
        "--limit",
        "--record",
        "--revision",
        "--summary",
        "--body",
        "--tags",
    ];
    let mut out = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if VALUE_FLAGS.contains(&arg.as_str()) {
            iter.next();
            continue;
        }
        if arg.starts_with("--") {
            continue;
        }
        out.push(arg.clone());
    }
    out
}

fn flag_value(args: &[String], name: &str) -> Option<String> {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == name {
            return iter.next().cloned();
        }
        if let Some(rest) = arg.strip_prefix(&format!("{name}=")) {
            return Some(rest.to_string());
        }
    }
    None
}

pub(crate) fn run_room_command(args: &[String]) -> std::io::Result<i32> {
    let positional = without_flags(args);
    let caller_pane_id = agent_caller(args);

    let method = match positional
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["create", workspace_id, name] => Method::RoomCreate(RoomCreateParams {
            workspace_id: (*workspace_id).to_string(),
            name: (*name).to_string(),
            objective: flag_value(args, "--objective").unwrap_or_default(),
        }),
        ["list"] => Method::RoomList(RoomListParams {
            workspace_id: flag_value(args, "--workspace"),
        }),
        ["get", room_id] => Method::RoomGet(RoomTargetParams {
            room_id: (*room_id).to_string(),
        }),
        ["pause", room_id] => Method::RoomSetLifecycle(RoomLifecycleParams {
            room_id: (*room_id).to_string(),
            lifecycle: RoomLifecycleValue::Paused,
        }),
        ["resume", room_id] => Method::RoomSetLifecycle(RoomLifecycleParams {
            room_id: (*room_id).to_string(),
            lifecycle: RoomLifecycleValue::Active,
        }),
        ["archive", room_id] => Method::RoomSetLifecycle(RoomLifecycleParams {
            room_id: (*room_id).to_string(),
            lifecycle: RoomLifecycleValue::Archived,
        }),
        ["delete", room_id] => Method::RoomDelete(RoomTargetParams {
            room_id: (*room_id).to_string(),
        }),
        ["member", "add", room_id, handle] => Method::RoomMemberAdd(RoomMemberAddParams {
            room_id: (*room_id).to_string(),
            handle: (*handle).to_string(),
            role: flag_value(args, "--role"),
            orchestrator: args.iter().any(|a| a == "--orchestrator"),
        }),
        ["member", "remove", room_id, member_id] => {
            Method::RoomMemberRemove(RoomMemberTargetParams {
                room_id: (*room_id).to_string(),
                member_id: (*member_id).to_string(),
            })
        }
        ["member", "bind", room_id, member_id, target] => {
            Method::RoomMemberBind(RoomMemberBindParams {
                room_id: (*room_id).to_string(),
                member_id: (*member_id).to_string(),
                target: Some((*target).to_string()),
            })
        }
        ["member", "unbind", room_id, member_id] => Method::RoomMemberBind(RoomMemberBindParams {
            room_id: (*room_id).to_string(),
            member_id: (*member_id).to_string(),
            target: None,
        }),
        ["member", "grant", room_id, member_id] => Method::RoomMemberGrant(RoomMemberGrantParams {
            room_id: (*room_id).to_string(),
            member_id: (*member_id).to_string(),
            may_broadcast: !args.iter().any(|a| a == "--revoke"),
            caller_pane_id,
        }),
        ["post", room_id, body] => Method::RoomPost(RoomPostParams {
            room_id: (*room_id).to_string(),
            body: (*body).to_string(),
            caller_pane_id,
            recipients: Vec::new(),
            root_id: flag_value(args, "--root"),
            client_nonce: flag_value(args, "--key"),
        }),
        ["ack", room_id, event_id] => {
            let Some(caller_pane_id) = caller_pane_id else {
                eprintln!("room ack must be called from a seated agent with --as-agent");
                return Ok(2);
            };
            Method::RoomAck(RoomDeliveryAckParams {
                room_id: (*room_id).to_string(),
                event_id: (*event_id).to_string(),
                caller_pane_id,
            })
        }
        ["events", room_id] => Method::RoomEvents(RoomEventListParams {
            room_id: (*room_id).to_string(),
            after_sequence: flag_value(args, "--after")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
            limit: flag_value(args, "--limit").and_then(|v| v.parse().ok()),
        }),
        ["memory", "put", room_id, title] => Method::RoomMemoryPut(RoomMemoryPutParams {
            room_id: (*room_id).to_string(),
            record_id: flag_value(args, "--record"),
            expected_revision: flag_value(args, "--revision").and_then(|v| v.parse().ok()),
            title: (*title).to_string(),
            summary: flag_value(args, "--summary").unwrap_or_default(),
            body: flag_value(args, "--body").unwrap_or_default(),
            tags: flag_value(args, "--tags")
                .map(|v| v.split(',').map(|t| t.trim().to_string()).collect())
                .unwrap_or_default(),
            caller_pane_id,
        }),
        ["memory", "search", room_id, query] => Method::RoomMemorySearch(RoomMemorySearchParams {
            room_id: (*room_id).to_string(),
            query: (*query).to_string(),
            limit: flag_value(args, "--limit").and_then(|v| v.parse().ok()),
        }),
        ["memory", "get", room_id, record_id] => Method::RoomMemoryGet(RoomMemoryTargetParams {
            room_id: (*room_id).to_string(),
            record_id: (*record_id).to_string(),
            caller_pane_id: None,
        }),
        ["memory", "delete", room_id, record_id] => {
            Method::RoomMemoryDelete(RoomMemoryTargetParams {
                room_id: (*room_id).to_string(),
                record_id: (*record_id).to_string(),
                caller_pane_id,
            })
        }
        ["memory", "accept", room_id, record_id] => {
            Method::RoomMemoryAccept(RoomMemoryTargetParams {
                room_id: (*room_id).to_string(),
                record_id: (*record_id).to_string(),
                caller_pane_id,
            })
        }
        ["allowance", "extend", room_id, root_id, additional] => {
            let Ok(additional) = additional.parse::<u32>() else {
                eprintln!("additional must be a positive whole number");
                return Ok(2);
            };
            Method::RoomAllowanceExtend(RoomAllowanceParams {
                room_id: (*room_id).to_string(),
                root_id: (*root_id).to_string(),
                additional,
            })
        }
        ["help"] | ["--help"] | ["-h"] | [] => {
            print_room_help();
            return Ok(0);
        }
        _ => {
            print_room_help();
            return Ok(2);
        }
    };

    super::print_response(&super::send_request(&Request {
        id: "cli:room".into(),
        method,
    })?)
}

fn print_room_help() {
    let name = crate::EXECUTABLE_NAME;
    eprintln!("{name} room commands:");
    eprintln!("  {name} room create <workspace_id> <name> [--objective TEXT]");
    eprintln!("  {name} room list [--workspace ID]");
    eprintln!("  {name} room get <room_id>");
    eprintln!("  {name} room pause|resume|archive|delete <room_id>");
    eprintln!("  {name} room member add <room_id> <handle> [--role TEXT] [--orchestrator]");
    eprintln!("  {name} room member remove <room_id> <member_id>");
    eprintln!("  {name} room member bind <room_id> <member_id> <agent-target>");
    eprintln!("  {name} room member unbind <room_id> <member_id>");
    eprintln!(
        "  {name} room member grant <room_id> <member_id> [--revoke] [--as-agent]   # allow @all"
    );
    eprintln!("  {name} room post <room_id> <body> [--root ID] [--key ID] [--as-agent]");
    eprintln!("  {name} room ack <room_id> <event_id> --as-agent");
    eprintln!("  {name} room events <room_id> [--after SEQ] [--limit N]");
    eprintln!(
        "  {name} room memory put <room_id> <title> [--summary TEXT] [--body TEXT] [--tags a,b] [--record ID --revision N] [--as-agent]"
    );
    eprintln!("  {name} room memory search <room_id> <query> [--limit N]");
    eprintln!("  {name} room memory get|accept|delete <room_id> <record_id> [--as-agent]");
    eprintln!("  {name} room allowance extend <room_id> <root_id> <additional>");
    eprintln!();
    eprintln!("  Mention members as @handle inside a post body to address them.");
    eprintln!("  --key makes a post safe to retry: the same key returns the first event");
    eprintln!("  instead of posting again and waking every recipient a second time.");
    eprintln!("  @all addresses every seat in the room, except your own when posting as an agent.");
    eprintln!("  Handles are matched without regard to case: @Codex and @codex are one seat.");
    eprintln!("  Only addressed members are activated; everyone else can read it later.");
    eprintln!("  Mentions inside code fences, quotes, or backticks are inert.");
    eprintln!("  --as-agent acts as your own seat instead of as the host.");
    eprintln!("  A member may accept a peer's record, but never its own.");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| v.to_string()).collect()
    }

    #[test]
    fn a_flag_value_is_not_mistaken_for_a_positional_argument() {
        // Regression: an objective containing spaces used to survive as a
        // fourth positional and break the `create` match.
        let parsed = without_flags(&args(&[
            "create",
            "w1",
            "Team",
            "--objective",
            "Improve onboarding",
        ]));
        assert_eq!(parsed, vec!["create", "w1", "Team"]);
    }

    #[test]
    fn boolean_flags_and_inline_values_consume_nothing_extra() {
        let parsed = without_flags(&args(&[
            "member",
            "add",
            "room_1",
            "orchestrator",
            "--orchestrator",
            "--role=lead",
        ]));
        assert_eq!(parsed, vec!["member", "add", "room_1", "orchestrator"]);
    }

    #[test]
    fn flag_values_read_both_spellings() {
        let a = args(&["list", "--workspace", "w1"]);
        assert_eq!(flag_value(&a, "--workspace").as_deref(), Some("w1"));
        let b = args(&["list", "--workspace=w2"]);
        assert_eq!(flag_value(&b, "--workspace").as_deref(), Some("w2"));
        assert_eq!(flag_value(&b, "--missing"), None);
    }

    #[test]
    fn acting_as_an_agent_is_opt_in() {
        // Without the flag the caller is the host, even inside a managed pane.
        assert!(agent_caller(&args(&["post", "room_1", "hi"])).is_none());
    }
}
