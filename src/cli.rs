use std::path::PathBuf;

use clap::{Args, CommandFactory as _, Parser, Subcommand};
use clap_complete::Shell;
use tracing::{Dispatch, Level, dispatcher};
use tracing_subscriber::{
    EnvFilter,
    fmt::SubscriberBuilder,
    layer::SubscriberExt as _,
    util::SubscriberInitExt as _,
};

use crate::{
    commands::{status::StatusCommandConfig, submit::SubmitCommandConfig},
    error::{Error, Result},
    output::{BufferedOutput, FlatOutput, InteractiveOutput, SyncOutput},
    tracing_formatter::PlainFormatter,
};

#[derive(Parser)]
#[command(name = "jj-vine")]
#[command(about = "GitLab MR submission tool for Jujutsu workflows", long_about = None, version)]
pub struct Cli {
    /// Repository path (defaults to current directory).
    #[arg(short = 'R', long, global = true)]
    pub repository: Option<PathBuf>,

    /// Enable verbose logging.
    #[arg(short = 'v', long, global = true, default_value_t = Cli::default_verbosity())]
    pub verbose: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    #[command(about = SubmitCommandConfig::help_long())]
    Submit(SubmitCommandConfig),

    /// Initialize jj-vine configuration for this repository.
    Init,

    /// Show status of tracked bookmarks and their MRs/PRs.
    Status(StatusCommandConfig),

    /// Generate shell completion scripts.
    Completions(CompletionsCommandConfig),
}

#[derive(Args)]
pub struct CompletionsCommandConfig {
    /// The shell to generate completions for (defaults to `$SHELL`).
    #[arg(value_enum)]
    pub shell: Option<Shell>,
}

impl CompletionsCommandConfig {
    fn shell(&self) -> Result<Shell> {
        self.shell.or_else(Shell::from_env).ok_or_else(|| {
            Error::new(
                "Could not determine the current shell from $SHELL. \
                 Specify one of: bash, elvish, fish, powershell, zsh.",
            )
        })
    }

    /// # Errors
    ///
    /// Returns an error if writing the completion script fails.
    pub fn generate(&self, buf: &mut dyn std::io::Write) -> Result<()> {
        let shell = self.shell()?;
        let mut command = Cli::command();
        command.set_bin_name("jj-vine");
        command.build();

        match clap_complete::Generator::try_generate(&shell, &command, buf) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

#[expect(clippy::module_name_repetitions, reason = "it's fine")]
pub struct CliConfig<'a> {
    /// Repository path (defaults to current directory).
    pub repository: PathBuf,

    /// Output formatter.
    pub output: &'a SyncOutput,
}

impl Cli {
    #[must_use]
    pub fn default_verbosity() -> bool {
        std::env::var("RUST_LOG").is_ok_and(|v| !v.is_empty())
    }

    pub async fn run_stdout(&self) -> Result<()> {
        let can_have_interactive_output = match self.command {
            Commands::Submit(_) | Commands::Status(_) => true,
            Commands::Init | Commands::Completions(_) => false,
        };

        let output: SyncOutput = if self.verbose || !can_have_interactive_output {
            SyncOutput::Flat(FlatOutput::new())
        } else {
            SyncOutput::Interactive(InteractiveOutput::new())
        };

        let filter = EnvFilter::builder()
            .with_default_directive(Level::INFO.into())
            .from_env_lossy();

        if self.verbose {
            tracing_subscriber::registry()
                .with(filter)
                .with(
                    tracing_subscriber::fmt::layer()
                        .event_format(PlainFormatter::new().with_level(true).with_timestamp(true)),
                )
                .init();
        } else {
            tracing_subscriber::registry()
                .with(filter)
                .with(tracing_subscriber::fmt::layer().event_format(PlainFormatter::new()))
                .init();
        }

        self.run(&output).await
    }

    pub async fn run_captured(&self) -> Result<String> {
        let buffered_output = Box::new(SyncOutput::Buffered(BufferedOutput::new()));

        // with_writer requires something with 'static, dunno how else to do this
        let buffered_output = &*Box::leak(buffered_output);

        let subscriber = SubscriberBuilder::default()
            .with_writer(move || buffered_output)
            .with_max_level(Level::INFO)
            .event_format(PlainFormatter::new())
            .finish();
        let dispatch = Dispatch::new(subscriber);

        // TODO doesn't really work with concurrency
        let _ = dispatcher::set_default(&dispatch);
        self.run(buffered_output).await?;

        let buffer = if let SyncOutput::Buffered(buffered) = buffered_output {
            buffered.get_buffer()
        } else {
            unreachable!();
        };

        Ok(strip_ansi::strip_str(&buffer).to_string())
    }

    /// # Panics
    ///
    /// Panics if the current directory is inaccessible.
    pub async fn run(&self, output: &SyncOutput) -> Result<()> {
        let repo_path = self.repository.as_ref().map_or_else(
            || std::env::current_dir().expect("Failed to get current directory"),
            Into::into,
        );

        let main_config = CliConfig {
            repository: repo_path.clone(),
            output,
        };

        match &self.command {
            Commands::Submit(options) => {
                crate::commands::submit::submit(options, &main_config).await?;
                Ok(())
            }
            Commands::Init => {
                crate::commands::init::init(&main_config)?;
                Ok(())
            }
            Commands::Status(options) => {
                crate::commands::status::status(options, &main_config).await?;
                Ok(())
            }
            Commands::Completions(options) => {
                options.generate(&mut std::io::stdout().lock())?;
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::ValueEnum as _;

    use super::*;

    #[test]
    fn completions_generate_for_every_shell() {
        for shell in Shell::value_variants() {
            let cli = Cli::try_parse_from(["jj-vine", "completions", &shell.to_string()])
                .expect("completions should parse");
            let Commands::Completions(options) = &cli.command else {
                panic!("expected completions command");
            };
            assert_eq!(options.shell().expect("shell is explicit"), *shell);

            let mut output = Vec::new();
            options.generate(&mut output).expect("stdout is infallible");

            let output = String::from_utf8(output).expect("completion script is UTF-8");
            assert!(
                output.contains("jj-vine") || output.contains("jj_vine"),
                "{shell} completions should reference the binary"
            );
        }
    }

    #[test]
    fn completions_parses_without_a_shell() {
        let cli = Cli::try_parse_from(["jj-vine", "completions"])
            .expect("completions should parse without a shell");
        let Commands::Completions(options) = &cli.command else {
            panic!("expected completions command");
        };

        assert_eq!(options.shell, None);
    }
}
