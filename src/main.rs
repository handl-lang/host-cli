use std::error::Error;
use std::io::Read;
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use handl_kit_canonical::CanonicalKit;
use handl_plugin_diag_expression::ExpressionDiagnostics;
use handl_plugin_fmt_expression::ExpressionFormatter;
use handl_plugin_lsp_completion::BasicCompletion;
use handl_plugin_lsp_goto::BasicGotoDef;
use handl_plugin_lsp_symbols::BasicDocumentSymbols;
use handl_platform::{DefaultHost, Kit, KitManifest, LanguageHost};
use handl_plugin_comments::CommentsPlugin;
use handl_plugin_doc_comments::DocCommentsPlugin;
use handl_tooling::ToolingHost;

use handl_ext_build::BuildPlugin;
use handl_ext_compile::CompilePlugin;
use handl_ext_eval::EvalPlugin;
use handl_ext_extension::ExtensionManagerPlugin;
use handl_ext_fmt::FmtPlugin;
use handl_ext_kit::KitManagerPlugin;
use handl_ext_lint::LintPlugin;
use handl_ext_plugin::PluginManagerPlugin;
use handl_ext_typecheck::TypecheckPlugin;

type DynErr = Box<dyn Error + Sync + Send>;

#[derive(Parser)]
#[command(name = "handl", about = "Handl language CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    #[command(about = "Format a Handl source file")]
    Fmt { file: Option<PathBuf> },
    #[command(about = "Evaluate a Handl expression")]
    Eval { expr: Vec<String> },
    #[command(about = "Typecheck a Handl source file")]
    Typecheck { file: Option<PathBuf> },
    #[command(about = "Start the stdio LSP server")]
    Lsp,
    #[command(external_subcommand)]
    Other(Vec<String>),
}

#[derive(Parser)]
#[command(name = "handl compose")]
struct Compose {
    #[arg(long)]
    kit: std::path::PathBuf,
    #[arg(long)]
    source: Option<String>,
    #[arg(long)]
    output: std::path::PathBuf,
}

fn run_compose(args: &[String]) -> Result<(), DynErr> {
    let compose = Compose::try_parse_from(args.to_vec())?;

    let source = match compose.source {
        Some(s) => s,
        None => {
            let mut buf = String::new();
            std::io::stdin().read_to_string(&mut buf)?;
            buf
        }
    };

    let manifest = KitManifest::load(&compose.kit)?;
    let kit = Kit {
        manifest,
        available_plugins: vec![
            CommentsPlugin::new().manifest(),
            DocCommentsPlugin::new().manifest(),
        ],
    };

    let mut host = LanguageHost::new();
    host.register_plugin("comments", CommentsPlugin::register_all);
    host.register_plugin("doc-comments", DocCommentsPlugin::register_all);

    let output = host.run_language_pipeline(&kit, &source)?;

    for diag in &output.diagnostics {
        eprintln!("{diag:?}");
    }

    std::fs::write(&compose.output, &output.formatted)?;
    println!("wrote {}", compose.output.display());
    Ok(())
}

fn read_source_or_stdin(file: &Option<PathBuf>) -> Result<String, DynErr> {
    match file.as_ref() {
        Some(p) if p.as_os_str() == "-" || p.as_os_str() == "" => {
            let mut buf = String::new();
            std::io::stdin().read_to_string(&mut buf)?;
            Ok(buf)
        }
        Some(p) => Ok(std::fs::read_to_string(p)?),
        None => {
            let mut buf = String::new();
            std::io::stdin().read_to_string(&mut buf)?;
            Ok(buf)
        }
    }
}

fn make_tooling_host() -> ToolingHost {
    ToolingHost::new(
        CanonicalKit::new(),
        Box::new(ExpressionFormatter::new()),
        Box::new(ExpressionDiagnostics::new()),
    )
    .with_completion_plugin(Box::new(BasicCompletion::new()))
    .with_goto_def_plugin(Box::new(BasicGotoDef::new()))
    .with_document_symbols_plugin(Box::new(BasicDocumentSymbols::new()))
}

fn run_fmt(file: &Option<PathBuf>) -> Result<(), DynErr> {
    let source = read_source_or_stdin(file)?;
    let host = make_tooling_host();
    let formatted = host.format(&source)?;
    print!("{}", formatted);
    Ok(())
}

fn run_eval(expr: &[String]) -> Result<(), DynErr> {
    if expr.is_empty() {
        return Err(Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "eval requires an expression",
        )));
    }
    let source = expr.join(" ");
    let kit = CanonicalKit::new();
    let output = kit.parse_module_recovered(&source);
    if output
        .diagnostics
        .iter()
        .any(|d| matches!(d.level, handl_expr::DiagnosticLevel::Error))
    {
        for d in &output.diagnostics {
            eprintln!("{}: {}", diagnostic_label(d.level), d.message);
        }
        return Err(Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "parse error",
        )));
    }
    let result = kit.eval(&output.stmts)?;
    let values: Vec<serde_json::Value> = result
        .values
        .into_iter()
        .map(|v| v.into_inner())
        .collect();
    println!("{}", serde_json::to_string_pretty(&values)?);
    Ok(())
}

fn run_typecheck(file: &Option<PathBuf>) -> Result<(), DynErr> {
    let source = read_source_or_stdin(file)?;
    let kit = CanonicalKit::new();
    let host = make_tooling_host();
    let parsed = kit.parse_module_recovered(&source);
    let mut diagnostics = parsed.diagnostics.clone();
    diagnostics.extend(host.diagnose(&source));

    let mut has_errors = diagnostics
        .iter()
        .any(|d| matches!(d.level, handl_expr::DiagnosticLevel::Error));

    for d in &diagnostics {
        eprintln!("{}: {}", diagnostic_label(d.level), d.message);
    }

    match kit.typecheck(&parsed.stmts) {
        Ok(out) => {
            if !out.success {
                has_errors = true;
            }
        }
        Err(e) => {
            has_errors = true;
            eprintln!("error: {}", e);
        }
    }

    if has_errors {
        return Err(Box::new(std::io::Error::new(
            std::io::ErrorKind::Other,
            "typecheck failed",
        )));
    }
    Ok(())
}

fn run_lsp() -> Result<(), DynErr> {
    let host = make_tooling_host();
    handl_host_lsp::run_stdio(host)?;
    Ok(())
}

fn diagnostic_label(level: handl_expr::DiagnosticLevel) -> &'static str {
    match level {
        handl_expr::DiagnosticLevel::Error => "error",
        handl_expr::DiagnosticLevel::Warning => "warning",
        handl_expr::DiagnosticLevel::Info => "info",
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 && args[1] == "compose" {
        if let Err(e) = run_compose(&args[1..]) {
            eprintln!("{e}");
            std::process::exit(1);
        }
        std::process::exit(0);
    }

    let cli = Cli::parse();

    let result: Result<(), DynErr> = match cli.command {
        Command::Fmt { file } => run_fmt(&file),
        Command::Eval { expr } => run_eval(&expr),
        Command::Typecheck { file } => run_typecheck(&file),
        Command::Lsp => run_lsp(),
        Command::Other(words) => run_capability(&words),
    };

    if let Err(e) = result {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

fn run_capability(words: &[String]) -> Result<(), DynErr> {
    let mut host = DefaultHost::new();
    host.register_plugin(FmtPlugin::new());
    host.register_plugin(LintPlugin::new());
    host.register_plugin(TypecheckPlugin::new());
    host.register_plugin(EvalPlugin::new());
    host.register_plugin(CompilePlugin::new());
    host.register_plugin(BuildPlugin::new());
    host.register_plugin(ExtensionManagerPlugin::new());
    host.register_plugin(KitManagerPlugin::new());
    host.register_plugin(PluginManagerPlugin::new());

    let (code, out) = match handl_host_cli::run(words.iter().cloned(), &mut host) {
        Ok((0, out)) => (0, out),
        Ok((c, out)) => (c, out),
        Err(e) => (1, e.to_string()),
    };

    if code == 0 {
        println!("{out}");
    } else {
        eprintln!("{out}");
    }

    if code != 0 {
        return Err(Box::new(std::io::Error::new(
            std::io::ErrorKind::Other,
            "capability failed",
        )));
    }
    Ok(())
}
