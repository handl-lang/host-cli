use std::error::Error;
use std::io::Read;

use clap::Parser;
use handl_kit_canonical::CanonicalKit;
use handl_plugin_diag_expression::ExpressionDiagnostics;
use handl_plugin_fmt_expression::ExpressionFormatter;
use handl_plugin_lsp_completion::BasicCompletion;
use handl_plugin_lsp_goto::BasicGotoDef;
use handl_plugin_lsp_symbols::BasicDocumentSymbols;
use handl_platform::{Kit, KitManifest, LanguageHost};
use handl_plugin_comments::CommentsPlugin;
use handl_plugin_doc_comments::DocCommentsPlugin;
use handl_tooling::{ArgSpec, CommandSpec, ToolingHost};

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

fn make_tooling_host() -> ToolingHost {
    ToolingHost::new(
        CanonicalKit::new(),
        Box::new(ExpressionFormatter::new()),
        Box::new(ExpressionDiagnostics::new()),
    )
    .with_completion_plugin(Box::new(BasicCompletion::new()))
    .with_goto_def_plugin(Box::new(BasicGotoDef::new()))
    .with_document_symbols_plugin(Box::new(BasicDocumentSymbols::new()))
    .with_command_provider(Box::new(FmtPlugin::new()))
    .with_command_provider(Box::new(EvalPlugin::new()))
    .with_command_provider(Box::new(TypecheckPlugin::new()))
    .with_command_provider(Box::new(BuildPlugin::new()))
    .with_command_provider(Box::new(CompilePlugin::new()))
    .with_command_provider(Box::new(LintPlugin::new()))
    .with_command_provider(Box::new(KitManagerPlugin::new()))
    .with_command_provider(Box::new(PluginManagerPlugin::new()))
    .with_command_provider(Box::new(ExtensionManagerPlugin::new()))
    .with_command_provider(Box::new(handl_host_lsp::LspCommandProvider::new()))
}

fn build_cli(host: &ToolingHost) -> clap::Command {
    fn leak(s: String) -> &'static str {
        &*Box::leak(s.into_boxed_str())
    }

    let mut cmd = clap::Command::new("hnl")
        .about("Handl language CLI")
        .subcommand_required(true)
        .arg_required_else_help(true);

    for spec in host.cli_commands() {
        let CommandSpec {
            name,
            description,
            aliases,
            args,
        } = spec;

        let mut sub = clap::Command::new(leak(name))
            .about(leak(description))
            .subcommand_required(false)
            .arg_required_else_help(false);

        for alias in aliases {
            sub = sub.visible_alias(leak(alias));
        }

        for arg in args {
            let ArgSpec {
                name,
                description,
                required,
                multiple,
                allow_hyphen_values,
            } = arg;

            let a = clap::Arg::new(leak(name))
                .help(leak(description))
                .required(required)
                .allow_hyphen_values(allow_hyphen_values);
            let a = if multiple {
                a.num_args(0..)
            } else {
                a.num_args(0..=1)
            };
            sub = sub.arg(a);
        }

        cmd = cmd.subcommand(sub);
    }

    cmd
}

fn main() -> Result<(), DynErr> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 && args[1] == "compose" {
        if let Err(e) = run_compose(&args[1..]) {
            eprintln!("{e}");
            std::process::exit(1);
        }
        std::process::exit(0);
    }

    let host = make_tooling_host();
    let cli = build_cli(&host);
    let matches = cli.get_matches_from(&args);

    let (sub_name, sub_matches) = matches
        .subcommand()
        .ok_or_else(|| -> DynErr { Box::new(std::io::Error::new(std::io::ErrorKind::Other, "no subcommand")) })?;

    let commands = host.cli_commands();
    let spec = commands
        .iter()
        .find(|s| s.name == sub_name || s.aliases.iter().any(|a| a == sub_name))
        .ok_or_else(|| -> DynErr { Box::new(std::io::Error::new(std::io::ErrorKind::Other, "unknown subcommand")) })?;

    let arg_values: Vec<String> = spec
        .args
        .first()
        .and_then(|arg| {
            sub_matches
                .get_raw(&arg.name)
                .map(|vals| vals.map(|v| v.to_string_lossy().into_owned()).collect())
        })
        .unwrap_or_default();

    if sub_name == "lsp" {
        handl_host_lsp::run_stdio(host)?;
    } else {
        let result = host
            .run_cli(sub_name, &arg_values)
            .map_err(|e| -> DynErr { Box::new(e) })?;
        println!("{}", serde_json::to_string_pretty(&result).map_err(|e| -> DynErr { Box::new(e) })?);
    }

    Ok(())
}
