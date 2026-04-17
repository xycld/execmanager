use std::io::IsTerminal;

use execmanager_cli::{commands::Command, run_current_user_command, CliError};

#[tokio::main]
async fn main() -> Result<(), CliError> {
    let command = Command::from_env(std::env::args())?;

    let interactive = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    let output = run_current_user_command(command, interactive).await?;

    if !output.is_empty() {
        println!("{output}");
    }

    Ok(())
}
