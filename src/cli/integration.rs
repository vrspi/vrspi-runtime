use crate::api::schema::IntegrationTarget;

pub(super) fn run_integration_command(args: &[String]) -> std::io::Result<i32> {
    let Some(subcommand) = args.first().map(|arg| arg.as_str()) else {
        print_integration_help();
        return Ok(2);
    };

    match subcommand {
        "install" => integration_install(&args[1..]),
        "uninstall" => integration_uninstall(&args[1..]),
        "status" => integration_status(&args[1..]),
        "help" | "--help" | "-h" => {
            print_integration_help();
            Ok(0)
        }
        _ => {
            print_integration_help();
            Ok(2)
        }
    }
}

fn integration_status(args: &[String]) -> std::io::Result<i32> {
    let outdated_only = match args {
        [] => false,
        [flag] if flag == "--outdated-only" => true,
        _ => {
            eprintln!("usage: herdr integration status [--outdated-only]");
            return Ok(2);
        }
    };

    if outdated_only {
        crate::integration::print_outdated_update_notice();
        return Ok(0);
    }

    for status in crate::integration::installed_integration_statuses() {
        let target = crate::integration::integration_target_label(status.target);
        let version = match status.installed_version {
            Some(version) => format!("v{version}"),
            None => "legacy".to_string(),
        };
        let state = match status.state {
            crate::integration::IntegrationStatusKind::NotInstalled => "not installed".to_string(),
            crate::integration::IntegrationStatusKind::Current => {
                format!("current ({version})")
            }
            crate::integration::IntegrationStatusKind::Outdated
                if status
                    .installed_version
                    .is_some_and(|installed| installed >= status.expected_version) =>
            {
                format!("needs repair ({version})")
            }
            crate::integration::IntegrationStatusKind::Outdated => {
                format!("outdated ({version} < v{})", status.expected_version)
            }
        };
        println!("{target}: {state} ({})", status.path.display());
    }

    Ok(0)
}

fn integration_install(args: &[String]) -> std::io::Result<i32> {
    let Some(target) = parse_integration_target(args, "install")? else {
        return Ok(2);
    };

    match crate::integration::install_target(target) {
        Ok(messages) => {
            print_integration_messages(messages);
            Ok(0)
        }
        Err(err) => {
            eprintln!("{err}");
            Ok(1)
        }
    }
}

fn integration_uninstall(args: &[String]) -> std::io::Result<i32> {
    let Some(target) = parse_integration_target(args, "uninstall")? else {
        return Ok(2);
    };

    match crate::integration::uninstall_target(target) {
        Ok(messages) => {
            print_integration_messages(messages);
            Ok(0)
        }
        Err(err) => {
            eprintln!("{err}");
            Ok(1)
        }
    }
}

fn print_integration_messages(messages: Vec<String>) {
    for message in messages {
        println!("{message}");
    }
}

fn parse_integration_target(
    args: &[String],
    action: &str,
) -> std::io::Result<Option<IntegrationTarget>> {
    let Some(target) = args.first().map(|arg| arg.as_str()) else {
        eprintln!(
            "usage: herdr integration {action} <pi|omp|claude|codex|copilot|devin|droid|kimi|opencode|kilo|hermes|qodercli|qwen|cursor|mastracode|grok>"
        );
        return Ok(None);
    };
    if args.len() != 1 {
        eprintln!(
            "usage: herdr integration {action} <pi|omp|claude|codex|copilot|devin|droid|kimi|opencode|kilo|hermes|qodercli|qwen|cursor|mastracode|grok>"
        );
        return Ok(None);
    }

    let parsed = match target {
        "pi" => IntegrationTarget::Pi,
        "omp" => IntegrationTarget::Omp,
        "claude" => IntegrationTarget::Claude,
        "codex" => IntegrationTarget::Codex,
        "copilot" => IntegrationTarget::Copilot,
        "devin" => IntegrationTarget::Devin,
        "droid" => IntegrationTarget::Droid,
        "kimi" => IntegrationTarget::Kimi,
        "opencode" => IntegrationTarget::Opencode,
        "kilo" => IntegrationTarget::Kilo,
        "hermes" => IntegrationTarget::Hermes,
        "qodercli" => IntegrationTarget::Qodercli,
        "qwen" => IntegrationTarget::Qwen,
        "cursor" => IntegrationTarget::Cursor,
        "mastracode" => IntegrationTarget::Mastracode,
        "antigravity-cli" | "antigravity_cli" => IntegrationTarget::AntigravityCli,
        "grok" => IntegrationTarget::Grok,
        _ => {
            eprintln!("unknown integration target: {target}");
            eprintln!(
                "currently supported: pi, omp, claude, codex, copilot, devin, droid, kimi, opencode, kilo, hermes, qodercli, qwen, cursor, mastracode, antigravity-cli, grok"
            );
            return Ok(None);
        }
    };

    Ok(Some(parsed))
}

fn print_integration_help() {
    eprintln!(
        "{name} integration commands:",
        name = crate::EXECUTABLE_NAME
    );
    eprintln!(
        "  {name} integration install pi",
        name = crate::EXECUTABLE_NAME
    );
    eprintln!(
        "  {name} integration install omp",
        name = crate::EXECUTABLE_NAME
    );
    eprintln!(
        "  {name} integration install claude",
        name = crate::EXECUTABLE_NAME
    );
    eprintln!(
        "  {name} integration install codex",
        name = crate::EXECUTABLE_NAME
    );
    eprintln!(
        "  {name} integration install copilot",
        name = crate::EXECUTABLE_NAME
    );
    eprintln!(
        "  {name} integration install devin",
        name = crate::EXECUTABLE_NAME
    );
    eprintln!(
        "  {name} integration install droid",
        name = crate::EXECUTABLE_NAME
    );
    eprintln!(
        "  {name} integration install kimi",
        name = crate::EXECUTABLE_NAME
    );
    eprintln!(
        "  {name} integration install opencode",
        name = crate::EXECUTABLE_NAME
    );
    eprintln!(
        "  {name} integration install kilo",
        name = crate::EXECUTABLE_NAME
    );
    eprintln!(
        "  {name} integration install hermes",
        name = crate::EXECUTABLE_NAME
    );
    eprintln!(
        "  {name} integration install qodercli",
        name = crate::EXECUTABLE_NAME
    );
    eprintln!(
        "  {name} integration install qwen",
        name = crate::EXECUTABLE_NAME
    );
    eprintln!(
        "  {name} integration install cursor",
        name = crate::EXECUTABLE_NAME
    );
    eprintln!(
        "  {name} integration install mastracode",
        name = crate::EXECUTABLE_NAME
    );
    eprintln!(
        "  {name} integration install antigravity-cli",
        name = crate::EXECUTABLE_NAME
    );
    eprintln!(
        "  {name} integration install grok",
        name = crate::EXECUTABLE_NAME
    );
    eprintln!(
        "  {name} integration uninstall pi",
        name = crate::EXECUTABLE_NAME
    );
    eprintln!(
        "  {name} integration uninstall omp",
        name = crate::EXECUTABLE_NAME
    );
    eprintln!(
        "  {name} integration uninstall claude",
        name = crate::EXECUTABLE_NAME
    );
    eprintln!(
        "  {name} integration uninstall codex",
        name = crate::EXECUTABLE_NAME
    );
    eprintln!(
        "  {name} integration uninstall copilot",
        name = crate::EXECUTABLE_NAME
    );
    eprintln!(
        "  {name} integration uninstall devin",
        name = crate::EXECUTABLE_NAME
    );
    eprintln!(
        "  {name} integration uninstall droid",
        name = crate::EXECUTABLE_NAME
    );
    eprintln!(
        "  {name} integration uninstall kimi",
        name = crate::EXECUTABLE_NAME
    );
    eprintln!(
        "  {name} integration uninstall opencode",
        name = crate::EXECUTABLE_NAME
    );
    eprintln!(
        "  {name} integration uninstall kilo",
        name = crate::EXECUTABLE_NAME
    );
    eprintln!(
        "  {name} integration uninstall hermes",
        name = crate::EXECUTABLE_NAME
    );
    eprintln!(
        "  {name} integration uninstall qodercli",
        name = crate::EXECUTABLE_NAME
    );
    eprintln!(
        "  {name} integration uninstall qwen",
        name = crate::EXECUTABLE_NAME
    );
    eprintln!(
        "  {name} integration uninstall cursor",
        name = crate::EXECUTABLE_NAME
    );
    eprintln!(
        "  {name} integration uninstall mastracode",
        name = crate::EXECUTABLE_NAME
    );
    eprintln!(
        "  {name} integration uninstall antigravity-cli",
        name = crate::EXECUTABLE_NAME
    );
    eprintln!(
        "  {name} integration uninstall grok",
        name = crate::EXECUTABLE_NAME
    );
    eprintln!(
        "  {name} integration status [--outdated-only]",
        name = crate::EXECUTABLE_NAME
    );
}
