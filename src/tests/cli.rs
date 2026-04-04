use std::net::{IpAddr, Ipv4Addr};

use clap::{CommandFactory, Parser};
use zellij_utils::cli::{CliArgs, Command, ReviewCli, ViewCli};

#[test]
fn verify_cli() {
    CliArgs::command().debug_assert();
}

#[test]
fn web_cli_status_alone_works() {
    let args = CliArgs::try_parse_from(["zellij", "web", "--status"]);
    assert!(args.is_ok());
    if let Ok(CliArgs {
        command: Some(Command::Web(web)),
        ..
    }) = args
    {
        assert!(web.status);
        assert!(web.timeout.is_none());
    } else {
        panic!("Expected Web command");
    }
}

#[test]
fn web_cli_status_with_timeout_works() {
    let args = CliArgs::try_parse_from(["zellij", "web", "--status", "--timeout", "5"]);
    assert!(args.is_ok());
    if let Ok(CliArgs {
        command: Some(Command::Web(web)),
        ..
    }) = args
    {
        assert!(web.status);
        assert_eq!(web.timeout, Some(5));
    } else {
        panic!("Expected Web command");
    }
}

#[test]
fn web_cli_timeout_with_status_works() {
    // Test with --timeout before --status (order shouldn't matter)
    let args = CliArgs::try_parse_from(["zellij", "web", "--timeout", "10", "--status"]);
    assert!(args.is_ok());
    if let Ok(CliArgs {
        command: Some(Command::Web(web)),
        ..
    }) = args
    {
        assert!(web.status);
        assert_eq!(web.timeout, Some(10));
    } else {
        panic!("Expected Web command");
    }
}

#[test]
fn web_cli_timeout_without_status_fails() {
    let args = CliArgs::try_parse_from(["zellij", "web", "--timeout", "5"]);
    assert!(args.is_err());
}

#[test]
fn web_cli_status_with_start_fails() {
    let args = CliArgs::try_parse_from(["zellij", "web", "--status", "--start"]);
    assert!(args.is_err());
}

#[test]
fn web_cli_status_with_stop_fails() {
    let args = CliArgs::try_parse_from(["zellij", "web", "--status", "--stop"]);
    assert!(args.is_err());
}

#[test]
fn web_cli_status_with_ip_works() {
    let args = CliArgs::try_parse_from(["zellij", "web", "--status", "--ip", "127.0.0.1"]);
    assert!(args.is_ok());
    if let Ok(CliArgs {
        command: Some(Command::Web(web)),
        ..
    }) = args
    {
        assert!(web.status);
        assert_eq!(web.ip, Some(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))));
    } else {
        panic!("Expected Web command");
    }
}

#[test]
fn web_cli_status_with_port_works() {
    let args = CliArgs::try_parse_from(["zellij", "web", "--status", "--port", "9000"]);
    assert!(args.is_ok());
    if let Ok(CliArgs {
        command: Some(Command::Web(web)),
        ..
    }) = args
    {
        assert!(web.status);
        assert_eq!(web.port, Some(9000));
    } else {
        panic!("Expected Web command");
    }
}

#[test]
fn web_cli_status_with_ip_and_port_works() {
    let args = CliArgs::try_parse_from([
        "zellij", "web", "--status", "--ip", "0.0.0.0", "--port", "9000",
    ]);
    assert!(args.is_ok());
    if let Ok(CliArgs {
        command: Some(Command::Web(web)),
        ..
    }) = args
    {
        assert!(web.status);
        assert_eq!(web.ip, Some(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0))));
        assert_eq!(web.port, Some(9000));
    } else {
        panic!("Expected Web command");
    }
}

#[test]
fn claude_room_command_without_id_works() {
    let args = CliArgs::try_parse_from(["zellij", "claude"]);
    assert!(args.is_ok());
    if let Ok(CliArgs {
        command: Some(Command::Claude(room)),
        ..
    }) = args
    {
        assert!(room.shared_id.is_none());
    } else {
        panic!("Expected Claude room command");
    }
}

#[test]
fn reviewer_room_command_with_id_works() {
    let args = CliArgs::try_parse_from(["zellij", "reviewer", "team-42"]);
    assert!(args.is_ok());
    if let Ok(CliArgs {
        command: Some(Command::Reviewer(room)),
        ..
    }) = args
    {
        assert_eq!(room.shared_id.as_deref(), Some("team-42"));
    } else {
        panic!("Expected Reviewer room command");
    }
}

#[test]
fn view_events_command_with_id_works() {
    let args = CliArgs::try_parse_from(["zellij", "view", "events", "team-42"]);
    assert!(args.is_ok());
    if let Ok(CliArgs {
        command: Some(Command::View(ViewCli::Events(room))),
        ..
    }) = args
    {
        assert_eq!(room.shared_id.as_deref(), Some("team-42"));
    } else {
        panic!("Expected view events command");
    }
}

#[test]
fn review_request_command_with_id_works() {
    let args = CliArgs::try_parse_from(["zellij", "review", "request", "team-42"]);
    assert!(args.is_ok());
    if let Ok(CliArgs {
        command: Some(Command::Review(ReviewCli::Request(room))),
        ..
    }) = args
    {
        assert_eq!(room.shared_id.as_deref(), Some("team-42"));
    } else {
        panic!("Expected review request command");
    }
}

#[test]
fn review_auto_start_command_with_id_works() {
    let args = CliArgs::try_parse_from(["zellij", "review", "auto-start", "team-42"]);
    assert!(args.is_ok());
    if let Ok(CliArgs {
        command: Some(Command::Review(ReviewCli::AutoStart(room))),
        ..
    }) = args
    {
        assert_eq!(room.shared_id.as_deref(), Some("team-42"));
    } else {
        panic!("Expected review auto-start command");
    }
}

#[test]
fn review_auto_stop_command_with_id_works() {
    let args = CliArgs::try_parse_from(["zellij", "review", "auto-stop", "team-42"]);
    assert!(args.is_ok());
    if let Ok(CliArgs {
        command: Some(Command::Review(ReviewCli::AutoStop(room))),
        ..
    }) = args
    {
        assert_eq!(room.shared_id.as_deref(), Some("team-42"));
    } else {
        panic!("Expected review auto-stop command");
    }
}

#[test]
fn review_auto_send_on_command_with_id_works() {
    let args = CliArgs::try_parse_from(["zellij", "review", "auto-send-on", "team-42"]);
    assert!(args.is_ok());
    if let Ok(CliArgs {
        command: Some(Command::Review(ReviewCli::AutoSendOn(room))),
        ..
    }) = args
    {
        assert_eq!(room.shared_id.as_deref(), Some("team-42"));
    } else {
        panic!("Expected review auto-send-on command");
    }
}

#[test]
fn review_auto_send_off_command_with_id_works() {
    let args = CliArgs::try_parse_from(["zellij", "review", "auto-send-off", "team-42"]);
    assert!(args.is_ok());
    if let Ok(CliArgs {
        command: Some(Command::Review(ReviewCli::AutoSendOff(room))),
        ..
    }) = args
    {
        assert_eq!(room.shared_id.as_deref(), Some("team-42"));
    } else {
        panic!("Expected review auto-send-off command");
    }
}

#[test]
fn review_auto_status_command_with_id_works() {
    let args = CliArgs::try_parse_from(["zellij", "review", "auto-status", "team-42"]);
    assert!(args.is_ok());
    if let Ok(CliArgs {
        command: Some(Command::Review(ReviewCli::AutoStatus(room))),
        ..
    }) = args
    {
        assert_eq!(room.shared_id.as_deref(), Some("team-42"));
    } else {
        panic!("Expected review auto-status command");
    }
}

#[test]
fn review_feedback_command_with_file_and_source_endpoint_works() {
    let args = CliArgs::try_parse_from([
        "zellij",
        "review",
        "feedback",
        "team-42",
        "--file",
        "/tmp/review.txt",
        "--source-endpoint",
        "reviewer-1234",
    ]);
    assert!(args.is_ok());
    if let Ok(CliArgs {
        command: Some(Command::Review(ReviewCli::Feedback(feedback))),
        ..
    }) = args
    {
        assert_eq!(feedback.shared_id.as_deref(), Some("team-42"));
        assert_eq!(
            feedback.file.as_deref().and_then(|path| path.to_str()),
            Some("/tmp/review.txt")
        );
        assert_eq!(feedback.source_endpoint.as_deref(), Some("reviewer-1234"));
    } else {
        panic!("Expected review feedback command");
    }
}
