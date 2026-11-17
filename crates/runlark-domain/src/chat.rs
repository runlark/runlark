use crate::id::{ProjectId, QuestionId, RunId, TaskId};
use crate::task::{TaskKind, TaskStage};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Structured command parsed from chat or CLI input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ParsedChatCommand {
    CreateTask {
        title: String,
        kind: TaskKind,
    },
    MoveTask {
        task_id: TaskId,
        stage: TaskStage,
    },
    UpdatePlan {
        task_id: TaskId,
        plan_text: String,
    },
    ApprovePlan {
        task_id: TaskId,
        comment: Option<String>,
    },
    StartRun {
        task_id: TaskId,
        prompt: Option<String>,
    },
    StopRun {
        run_id: RunId,
        reason: Option<String>,
    },
    ResumeRun {
        run_id: RunId,
    },
    AnswerQuestion {
        question_id: QuestionId,
        answer: String,
    },
    Accept {
        target_id: String,
        comment: Option<String>,
    },
    Reject {
        target_id: String,
        reason: String,
    },
    CreatePr {
        run_id: RunId,
        title: Option<String>,
        draft: bool,
    },
    Status {
        target_id: Option<String>,
    },
    Board {
        project_id: Option<ProjectId>,
    },
    Budget {
        target_id: Option<String>,
    },
    Help,
    Prompt {
        text: String,
    },
}

/// Errors produced when parsing a chat command string.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ChatParseError {
    #[error("input cannot be empty")]
    EmptyInput,

    #[error("unterminated quote in command input")]
    UnterminatedQuote,

    #[error("dangling escape character at end of command input")]
    DanglingEscape,

    #[error("unknown slash command `/{0}` (try /help)")]
    UnknownCommand(String),

    #[error("missing required argument `{arg}` for `{command}`")]
    MissingArgument {
        command: &'static str,
        arg: &'static str,
    },

    #[error("invalid flag `{flag}` for `{command}`")]
    InvalidFlag { command: &'static str, flag: String },

    #[error("invalid task stage `{0}` (expected inbox, ready, in_progress, acceptance, done)")]
    InvalidStage(String),
}

fn tokenize_args(input: &str) -> Result<Vec<String>, ChatParseError> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut in_token = false;

    for ch in input.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            in_token = true;
            continue;
        }
        if ch == '\\' && quote != Some('\'') {
            escaped = true;
            in_token = true;
            continue;
        }
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            } else {
                current.push(ch);
            }
            in_token = true;
            continue;
        }
        match ch {
            '"' | '\'' => {
                quote = Some(ch);
                in_token = true;
            }
            c if c.is_whitespace() => {
                if in_token {
                    tokens.push(std::mem::take(&mut current));
                    in_token = false;
                }
            }
            other => {
                current.push(other);
                in_token = true;
            }
        }
    }

    if escaped {
        return Err(ChatParseError::DanglingEscape);
    }
    if quote.is_some() {
        return Err(ChatParseError::UnterminatedQuote);
    }
    if in_token {
        tokens.push(current);
    }
    Ok(tokens)
}

fn parse_stage(raw: &str) -> Result<TaskStage, ChatParseError> {
    match raw.to_ascii_lowercase().as_str() {
        "inbox" => Ok(TaskStage::Inbox),
        "ready" => Ok(TaskStage::Ready),
        "in_progress" | "in-progress" | "inprogress" => Ok(TaskStage::InProgress),
        "acceptance" | "review" => Ok(TaskStage::Acceptance),
        "done" => Ok(TaskStage::Done),
        _ => Err(ChatParseError::InvalidStage(raw.to_string())),
    }
}

fn join_non_empty(parts: &[String]) -> Option<String> {
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" "))
    }
}

fn parse_task_cmd(args: &[String]) -> Result<ParsedChatCommand, ChatParseError> {
    let Some(first) = args.first() else {
        return Err(ChatParseError::MissingArgument {
            command: "/task",
            arg: "title or subcommand",
        });
    };

    if first == "move" {
        let task_id = args.get(1).ok_or(ChatParseError::MissingArgument {
            command: "/task move",
            arg: "task_id",
        })?;
        let stage_raw = args.get(2).ok_or(ChatParseError::MissingArgument {
            command: "/task move",
            arg: "stage",
        })?;
        return Ok(ParsedChatCommand::MoveTask {
            task_id: TaskId::from(task_id.as_str()),
            stage: parse_stage(stage_raw)?,
        });
    }

    let slice = if first == "new" { &args[1..] } else { args };
    let mut kind = TaskKind::Standard;
    let mut title_parts = Vec::new();
    for arg in slice {
        if arg == "--quick" {
            kind = TaskKind::QuickFix;
        } else if arg.starts_with("--") {
            return Err(ChatParseError::InvalidFlag {
                command: "/task",
                flag: arg.clone(),
            });
        } else {
            title_parts.push(arg.clone());
        }
    }

    let title = join_non_empty(&title_parts).ok_or(ChatParseError::MissingArgument {
        command: "/task",
        arg: "title",
    })?;
    Ok(ParsedChatCommand::CreateTask { title, kind })
}

fn parse_plan_cmd(args: &[String]) -> Result<ParsedChatCommand, ChatParseError> {
    let Some(first) = args.first() else {
        return Err(ChatParseError::MissingArgument {
            command: "/plan",
            arg: "task_id or approve",
        });
    };
    if first == "approve" {
        let task_id = args.get(1).ok_or(ChatParseError::MissingArgument {
            command: "/plan approve",
            arg: "task_id",
        })?;
        return Ok(ParsedChatCommand::ApprovePlan {
            task_id: TaskId::from(task_id.as_str()),
            comment: join_non_empty(&args[2..]),
        });
    }
    let plan_text = join_non_empty(&args[1..]).ok_or(ChatParseError::MissingArgument {
        command: "/plan",
        arg: "plan_text",
    })?;
    Ok(ParsedChatCommand::UpdatePlan {
        task_id: TaskId::from(first.as_str()),
        plan_text,
    })
}

fn parse_run_cmd(args: &[String]) -> Result<ParsedChatCommand, ChatParseError> {
    let Some(first) = args.first() else {
        return Err(ChatParseError::MissingArgument {
            command: "/run",
            arg: "task_id or subcommand",
        });
    };
    if first == "stop" {
        let run_id = args.get(1).ok_or(ChatParseError::MissingArgument {
            command: "/run stop",
            arg: "run_id",
        })?;
        return Ok(ParsedChatCommand::StopRun {
            run_id: RunId::from(run_id.as_str()),
            reason: join_non_empty(&args[2..]),
        });
    }
    if first == "resume" {
        let run_id = args.get(1).ok_or(ChatParseError::MissingArgument {
            command: "/run resume",
            arg: "run_id",
        })?;
        return Ok(ParsedChatCommand::ResumeRun {
            run_id: RunId::from(run_id.as_str()),
        });
    }

    let slice = if first == "start" { &args[1..] } else { args };
    let Some(task_id) = slice.first() else {
        return Err(ChatParseError::MissingArgument {
            command: "/run",
            arg: "task_id",
        });
    };
    let mut prompt_parts = Vec::new();
    let mut idx = 1;
    while idx < slice.len() {
        if slice[idx] == "--prompt" {
            let val = slice.get(idx + 1).ok_or(ChatParseError::MissingArgument {
                command: "/run",
                arg: "--prompt value",
            })?;
            prompt_parts.push(val.clone());
            idx += 2;
        } else if slice[idx].starts_with("--") {
            return Err(ChatParseError::InvalidFlag {
                command: "/run",
                flag: slice[idx].clone(),
            });
        } else {
            prompt_parts.push(slice[idx].clone());
            idx += 1;
        }
    }
    Ok(ParsedChatCommand::StartRun {
        task_id: TaskId::from(task_id.as_str()),
        prompt: join_non_empty(&prompt_parts),
    })
}

fn parse_pr_cmd(args: &[String]) -> Result<ParsedChatCommand, ChatParseError> {
    let slice = if args.first().map(String::as_str) == Some("create") {
        &args[1..]
    } else {
        args
    };
    let Some(run_id) = slice.first() else {
        return Err(ChatParseError::MissingArgument {
            command: "/pr",
            arg: "run_id",
        });
    };
    let mut title: Option<String> = None;
    let mut draft = false;
    let mut idx = 1;
    while idx < slice.len() {
        match slice[idx].as_str() {
            "--draft" => {
                draft = true;
                idx += 1;
            }
            "--title" => {
                let val = slice.get(idx + 1).ok_or(ChatParseError::MissingArgument {
                    command: "/pr",
                    arg: "--title value",
                })?;
                title = Some(val.clone());
                idx += 2;
            }
            flag if flag.starts_with("--") => {
                return Err(ChatParseError::InvalidFlag {
                    command: "/pr",
                    flag: flag.to_string(),
                });
            }
            other => {
                return Err(ChatParseError::InvalidFlag {
                    command: "/pr",
                    flag: other.to_string(),
                });
            }
        }
    }
    Ok(ParsedChatCommand::CreatePr {
        run_id: RunId::from(run_id.as_str()),
        title,
        draft,
    })
}

fn parse_move_cmd(
    command: &'static str,
    args: &[String],
) -> Result<ParsedChatCommand, ChatParseError> {
    let task_id = args.first().ok_or(ChatParseError::MissingArgument {
        command,
        arg: "task_id",
    })?;
    let stage = args.get(1).ok_or(ChatParseError::MissingArgument {
        command,
        arg: "stage",
    })?;
    Ok(ParsedChatCommand::MoveTask {
        task_id: TaskId::from(task_id.as_str()),
        stage: parse_stage(stage)?,
    })
}

fn parse_accept_cmd(args: &[String]) -> Result<ParsedChatCommand, ChatParseError> {
    let target_id = args.first().ok_or(ChatParseError::MissingArgument {
        command: "/accept",
        arg: "target_id",
    })?;
    Ok(ParsedChatCommand::Accept {
        target_id: target_id.clone(),
        comment: join_non_empty(&args[1..]),
    })
}

fn parse_reject_cmd(args: &[String]) -> Result<ParsedChatCommand, ChatParseError> {
    let target_id = args.first().ok_or(ChatParseError::MissingArgument {
        command: "/reject",
        arg: "target_id",
    })?;
    let reason = join_non_empty(&args[1..]).ok_or(ChatParseError::MissingArgument {
        command: "/reject",
        arg: "reason",
    })?;
    Ok(ParsedChatCommand::Reject {
        target_id: target_id.clone(),
        reason,
    })
}

/// Parses a raw chat message or slash command into a structured [`ParsedChatCommand`].
///
/// # Errors
/// Returns [`ChatParseError`] on empty input, malformed quotes/escapes, unknown slash commands,
/// invalid flags/stages, or missing required arguments.
pub fn parse_chat_command(input: &str) -> Result<ParsedChatCommand, ChatParseError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(ChatParseError::EmptyInput);
    }

    if !trimmed.starts_with('/') {
        return Ok(ParsedChatCommand::Prompt {
            text: trimmed.to_string(),
        });
    }

    let tokens = tokenize_args(trimmed)?;
    let Some(cmd_raw) = tokens.first() else {
        return Err(ChatParseError::EmptyInput);
    };
    let cmd = cmd_raw.trim_start_matches('/').to_ascii_lowercase();
    let args = &tokens[1..];

    match cmd.as_str() {
        "task" => parse_task_cmd(args),
        "fix" => Ok(ParsedChatCommand::CreateTask {
            title: join_non_empty(args).ok_or(ChatParseError::MissingArgument {
                command: "/fix",
                arg: "title",
            })?,
            kind: TaskKind::QuickFix,
        }),
        "move" => parse_move_cmd("/move", args),
        "plan" => parse_plan_cmd(args),
        "run" => parse_run_cmd(args),
        "stop" => Ok(ParsedChatCommand::StopRun {
            run_id: RunId::from(
                args.first()
                    .ok_or(ChatParseError::MissingArgument {
                        command: "/stop",
                        arg: "run_id",
                    })?
                    .as_str(),
            ),
            reason: join_non_empty(&args[1..]),
        }),
        "resume" => Ok(ParsedChatCommand::ResumeRun {
            run_id: RunId::from(
                args.first()
                    .ok_or(ChatParseError::MissingArgument {
                        command: "/resume",
                        arg: "run_id",
                    })?
                    .as_str(),
            ),
        }),
        "answer" => Ok(ParsedChatCommand::AnswerQuestion {
            question_id: QuestionId::from(
                args.first()
                    .ok_or(ChatParseError::MissingArgument {
                        command: "/answer",
                        arg: "question_id",
                    })?
                    .as_str(),
            ),
            answer: join_non_empty(&args[1..]).ok_or(ChatParseError::MissingArgument {
                command: "/answer",
                arg: "answer",
            })?,
        }),
        "accept" | "approve" => parse_accept_cmd(args),
        "reject" => parse_reject_cmd(args),
        "pr" => parse_pr_cmd(args),
        "status" => Ok(ParsedChatCommand::Status {
            target_id: args.first().cloned(),
        }),
        "board" => Ok(ParsedChatCommand::Board {
            project_id: args.first().map(|s| ProjectId::from(s.as_str())),
        }),
        "budget" => Ok(ParsedChatCommand::Budget {
            target_id: args.first().cloned(),
        }),
        "help" => Ok(ParsedChatCommand::Help),
        other => Err(ChatParseError::UnknownCommand(other.to_string())),
    }
}
