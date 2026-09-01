use handl_kit_canonical::Kit;
use handl_language::LangError;
use handl_platform::{CapabilityRequest, DefaultHost, PlatformError};
use handl_plugin_applier::BootstrapPlugin;
use handl_plugin_ext_cli::BasicCli;
use serde_json::{Value, json};

fn main() -> Result<(), PlatformError> {
    let args = std::env::args().collect::<Vec<_>>();
    let (applier_id, config_path, kit_name, command, command_args) = parse_args(&args)?;

    // Handle gen-grammar before loading the plugin host — it only needs the kit.
    if command == "gen-grammar" {
        return gen_grammar(&kit_name, command_args.first().copied());
    }

    // Handle fmt before loading the plugin host — it only needs the kit's lexer.
    if command == "fmt" {
        return fmt_file(command_args.first().copied());
    }

    // Load the kit via the plugin system. This registers ModuleRuntime,
    // plugin-lib-* (which contribute modules to it), and all language
    // operations (lexer, parser, formatter, etc.).
    let mut host = DefaultHost::new();
    host.register_plugin(BootstrapPlugin::new(config_path, kit_name, applier_id));
    host.register_plugin(BasicCli::new());
    host.load()?;

    // Validate the composition before running any command.
    if let Err(e) = host.resolve_composition() {
        eprintln!("composition error: {e}");
        return Err(PlatformError::Plugin(e.to_string()));
    }

    // Dispatch: run/eval use the data-driven Kit directly (ADR 0044).
    // Other commands go through the `cli` capability as before.
    match command {
        "run" => run_file(command_args.first().copied()),
        "eval" => eval_expression(command_args.first().copied()),
        "typecheck" => typecheck_expression(command_args.first().copied()),
        _ => {
            let resp = host.run(&CapabilityRequest {
                capability: "cli".into(),
                payload: json!({
                    "command": command,
                    "args": command_args,
                }),
            })?;

            if let Some(s) = resp.0.as_str() {
                println!("{s}");
            }
            Ok(())
        }
    }
}

fn run_file(path: Option<&str>) -> Result<(), PlatformError> {
    let path =
        path.ok_or_else(|| PlatformError::InvalidMetadata("usage: hnl run <file.hnl>".into()))?;

    let source = std::fs::read_to_string(path)
        .map_err(|e| PlatformError::InvalidMetadata(format!("failed to read {path}: {e}")))?;

    let kit = Kit::new().map_err(lang_to_platform)?;
    let value = kit.run(&source).map_err(lang_to_platform)?;
    print_value(&value);
    Ok(())
}

fn eval_expression(expr: Option<&str>) -> Result<(), PlatformError> {
    let expr_str =
        expr.ok_or_else(|| PlatformError::InvalidMetadata("usage: hnl eval <expression>".into()))?;

    let kit = Kit::new().map_err(lang_to_platform)?;
    let value = kit.eval(expr_str).map_err(lang_to_platform)?;
    print_value(&value);
    Ok(())
}

fn typecheck_expression(expr: Option<&str>) -> Result<(), PlatformError> {
    let expr_str = expr.ok_or_else(|| {
        PlatformError::InvalidMetadata("usage: hnl typecheck <expression>".into())
    })?;

    let kit = Kit::new().map_err(lang_to_platform)?;
    let typ = kit.typecheck(expr_str).map_err(lang_to_platform)?;
    print_value(&typ);
    Ok(())
}

/// Format a `.hnl` file using a token-based pretty-printer.
///
/// Lexes the source into tokens, then re-emits them with consistent
/// indentation: increases after `{` / `(` / `[`, decreases before the
/// matching close, newlines after `,`, and leading-operator style for `>>>`.
fn fmt_file(path: Option<&str>) -> Result<(), PlatformError> {
    let path =
        path.ok_or_else(|| PlatformError::InvalidMetadata("usage: hnl fmt <file.hnl>".into()))?;

    let source = std::fs::read_to_string(path)
        .map_err(|e| PlatformError::InvalidMetadata(format!("failed to read {path}: {e}")))?;

    let kit = Kit::new().map_err(lang_to_platform)?;
    let tokens = kit.lex(&source).map_err(lang_to_platform)?;
    let formatted = format_tokens(tokens.as_slice());
    println!("{formatted}");
    Ok(())
}

/// Token-based pretty-printer.
///
/// Walks the token stream and emits formatted output with:
/// - Indentation after `{`, `(`, `[` (2 spaces per level) — but only when
///   the block is "complex" (has commas or nested delimiters)
/// - Simple blocks like `(s)`, `{ out: v }`, `[1, 2, 3]` stay inline
/// - Dedent before the matching `}`, `)`, `]`
/// - Newline after `,` (inside multi-line blocks)
/// - Leading-operator style for `>>>` (newline + indent)
/// - No trailing whitespace
fn format_tokens(tokens: &[handl_language::Token]) -> String {
    let mut out = String::new();
    let mut indent: usize = 0;
    let mut at_line_start = true;
    // Stack tracking whether each open delimiter is "broken" (multi-line).
    let mut broken_stack: Vec<bool> = Vec::new();

    for (i, token) in tokens.iter().enumerate() {
        if token.kind == "token.eof" {
            break;
        }

        let text = &token.text;

        // Closing delimiters.
        if text == "}" || text == ")" || text == "]" {
            let was_broken = broken_stack.pop().unwrap_or(false);
            if was_broken {
                if indent > 0 {
                    indent -= 1;
                }
                if !at_line_start {
                    out.push('\n');
                }
                out.push_str(&"  ".repeat(indent));
            } else if text == "}" {
                // Space before inline `}` for readability.
                if let Some(last) = out.chars().last() {
                    if last != ' ' && last != '{' && last != '\n' {
                        out.push(' ');
                    }
                }
            }
            out.push_str(text);
            at_line_start = false;
            continue;
        }

        // Leading-operator style for `>>>`: newline + indent + space.
        if text == ">>>" {
            out.push('\n');
            out.push_str(&"  ".repeat(indent));
            out.push_str(text);
            out.push(' ');
            at_line_start = false;
            continue;
        }

        // Indentation at line start.
        if at_line_start {
            out.push_str(&"  ".repeat(indent));
            at_line_start = false;
        }

        // Comma: emit + newline (only if inside a broken block).
        if text == "," {
            out.push_str(text);
            if *broken_stack.last().unwrap_or(&false) {
                out.push('\n');
                at_line_start = true;
            } else {
                out.push(' ');
            }
            continue;
        }

        // Opening delimiters.
        if text == "{" || text == "(" || text == "[" {
            let close = if text == "{" {
                "}"
            } else if text == "(" {
                ")"
            } else {
                "]"
            };
            // Scan ahead to matching close to decide whether to break.
            let should_break = should_break_block(tokens, i, close);
            // Space before `{` when it follows a non-delimiter token
            // (e.g. `match x { ... }`, `loop state { ... }`).
            if text == "{" {
                if let Some(last) = out.chars().last() {
                    if last != '(' && last != '[' && last != '{' && last != ' ' && last != '\n' {
                        out.push(' ');
                    }
                }
            }
            // `(` following `}` → keep on same line (e.g. `loop { ... }(body)`).
            // No newline needed — the `(` is the body application.
            out.push_str(text);
            if should_break {
                broken_stack.push(true);
                indent += 1;
                out.push('\n');
                at_line_start = true;
            } else {
                broken_stack.push(false);
                // Add space inside inline `{ }` for readability.
                if text == "{" {
                    let next = tokens.get(i + 1).map(|t| t.text.as_str()).unwrap_or("");
                    if next != close {
                        out.push(' ');
                    }
                }
            }
            continue;
        }

        // Keywords: `if`/`then`/`else`/`in`/`match`/`loop`/`proc`/...
        let is_keyword = token.kind.starts_with("token.keyword.");
        if is_keyword {
            // `else` begins a new line in an if-chain
            // (`if c then a \n else if c2 then b \n else c`).
            if text == "else" {
                out.push('\n');
                out.push_str(&"  ".repeat(indent));
                out.push_str("else ");
                continue;
            }
            // Leading space before a keyword when it follows a value or a
            // closing delimiter (e.g. `n == 0 then`, `} in`, `y in`). Without
            // this, `kthen` would lex as a single identifier.
            if let Some(last) = out.chars().last() {
                if last != ' ' && last != '\n' && last != '(' && last != '[' && last != '{' {
                    out.push(' ');
                }
            }
            out.push_str(text);
            // `in` ends a bind clause; break to a new line so chained
            // bindings (`x = y in \n body`) read top-to-bottom.
            if text == "in" {
                out.push('\n');
                at_line_start = true;
            } else {
                out.push(' ');
            }
            continue;
        }

        // Semicolon: emit + newline.
        if text == ";" {
            out.push_str(text);
            out.push('\n');
            at_line_start = true;
            continue;
        }

        // Colon in match arms / object entries: emit + space.
        if text == ":" {
            out.push_str(text);
            out.push(' ');
            continue;
        }

        // Arrow: emit + space.
        if text == "->" {
            out.push(' ');
            out.push_str(text);
            out.push(' ');
            continue;
        }

        // Binary operators: space around — except unary minus/plus.
        let is_binop = matches!(
            token.kind.as_str(),
            "token.plus"
                | "token.minus"
                | "token.star"
                | "token.slash"
                | "token.percent"
                | "token.star.star"
                | "token.lt"
                | "token.gt"
                | "token.lte"
                | "token.gte"
                | "token.eqeq"
                | "token.equal"
                | "token.neq"
                | "token.and"
                | "token.or"
                | "token.spaceship"
        );
        if is_binop {
            // Detect unary minus/plus: previous output char is an operator,
            // delimiter, comma, colon, arrow, or start of line.
            let is_unary = (token.kind == "token.minus" || token.kind == "token.plus")
                && {
                    let last = out.chars().last();
                    matches!(
                        last,
                        None | Some('(') | Some('[') | Some('{') | Some(',') | Some(':')
                        | Some('>') | Some('<') | Some('=') | Some('!') | Some('&')
                        | Some('|') | Some('+') | Some('-') | Some('*') | Some('/')
                        | Some('%') | Some(' ')
                    )
                };
            if is_unary {
                out.push_str(text);
            } else {
                out.push(' ');
                out.push_str(text);
                out.push(' ');
            }
            continue;
        }

        // Default: emit as-is.
        out.push_str(text);
    }

    // Trim trailing whitespace per line.
    out.lines()
        .map(|l| l.trim_end())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Decide whether a block starting at `tokens[start]` (an opening delimiter)
/// should be broken into multiple lines.
///
/// Breaks when the block contains:
/// - Any nested `{` or `match` keyword at depth 0 (complex content)
/// - Any `>>>` at depth 0 (composition chain)
/// - Commas at depth 0 AND the total token count exceeds a threshold
///   (short comma-separated lists like `[1, 2, 3]` stay inline)
///
/// Stays inline for simple blocks like `(s)`, `{ out: v }`, `[1, 2, 3]`.
fn should_break_block(tokens: &[handl_language::Token], start: usize, _close: &str) -> bool {
    let mut depth: usize = 0;
    let mut depth0_tokens: usize = 0;
    let mut depth0_commas: usize = 0;
    let mut depth0_colons: usize = 0;
    let mut i = start + 1;
    while i < tokens.len() {
        let token = &tokens[i];
        if token.kind == "token.eof" {
            break;
        }
        let t = &token.text;
        if t == "{" || t == "(" || t == "[" {
            if depth == 0 {
                // Nested delimiter at depth 0 — check if *it* would break.
                let nested_close = if t == "{" { "}" } else if t == "(" { ")" } else { "]" };
                if should_break_block(tokens, i, nested_close) {
                    return true;
                }
            }
            depth += 1;
            i += 1;
            continue;
        }
        if t == "}" || t == ")" || t == "]" {
            if depth == 0 {
                break;
            }
            depth -= 1;
            i += 1;
            continue;
        }
        if depth == 0 {
            depth0_tokens += 1;
            if t == "," {
                depth0_commas += 1;
            }
            if t == ":" {
                depth0_colons += 1;
            }
            // `match` or `>>>` at depth 0 → always break.
            if t == "match" || t == ">>>" {
                return true;
            }
        }
        i += 1;
    }
    // Match arms / object fields: break when there are 2+ commas (3+ arms/
    // fields), or 1 comma with enough content to warrant it.
    if depth0_commas >= 2 && depth0_colons >= 2 {
        return true;
    }
    if depth0_commas >= 1 && depth0_colons >= 2 && depth0_tokens > 8 {
        return true;
    }
    // Long comma-separated blocks → break.
    if depth0_commas >= 1 && depth0_tokens > 12 {
        return true;
    }
    false
}

/// Convert a [`LangError`] to a [`PlatformError`].
fn lang_to_platform(e: LangError) -> PlatformError {
    PlatformError::Plugin(e.to_string())
}

/// Print a `serde_json::Value` the way the CLI presents eval results: bare
/// strings are printed without quotes, everything else as pretty-printed JSON.
fn print_value(value: &Value) {
    if let Some(s) = value.as_str() {
        println!("{s}");
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(value).unwrap_or_default()
        );
    }
}

fn gen_grammar(_kit_name: &str, output_path: Option<&str>) -> Result<(), PlatformError> {
    let kit = Kit::new().map_err(lang_to_platform)?;
    // Grammar rules are no longer collected from the legacy token factories.
    // The data-driven textmate emitter will provide rule data in a future
    // extension; for now we emit an empty grammar skeleton.
    let rules: Vec<handl_host_tooling::GrammarRule> = Vec::new();
    let grammar =
        handl_host_tooling::generate_textmate(&rules, kit.scope_name(), &kit.file_extensions());

    let json = serde_json::to_string_pretty(&grammar)
        .map_err(|e| PlatformError::InvalidMetadata(e.to_string()))?;

    match output_path {
        Some(path) => {
            std::fs::write(path, &json).map_err(|e| {
                PlatformError::InvalidMetadata(format!("failed to write {path}: {e}"))
            })?;
            eprintln!("Generated TextMate grammar: {path}");
        }
        None => {
            println!("{json}");
        }
    }
    Ok(())
}

fn parse_args(args: &[String]) -> Result<(String, String, String, &str, Vec<&str>), PlatformError> {
    let mut applier_id = "applier.default".to_string();
    let mut config_path = ".handl.toml".to_string();
    let mut kit_name = std::env::var("HANDL_KIT").unwrap_or_else(|_| "hello".to_string());
    let mut command = "";
    let mut command_args = Vec::new();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--applier" => {
                i += 1;
                applier_id = args
                    .get(i)
                    .ok_or_else(|| {
                        PlatformError::InvalidMetadata("missing --applier value".into())
                    })?
                    .to_string();
            }
            "--config" => {
                i += 1;
                config_path = args
                    .get(i)
                    .ok_or_else(|| PlatformError::InvalidMetadata("missing --config value".into()))?
                    .to_string();
            }
            "--kit" => {
                i += 1;
                kit_name = args
                    .get(i)
                    .ok_or_else(|| PlatformError::InvalidMetadata("missing --kit value".into()))?
                    .to_string();
            }
            "--" => {
                // Separator: following args are the command and its args.
            }
            _ if command.is_empty() => {
                command = &args[i];
            }
            _ => {
                command_args.push(args[i].as_str());
            }
        }
        i += 1;
    }
    if command.is_empty() {
        return Err(PlatformError::InvalidMetadata("missing command".into()));
    }
    Ok((applier_id, config_path, kit_name, command, command_args))
}
