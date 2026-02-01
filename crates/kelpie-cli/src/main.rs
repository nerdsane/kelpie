//! Kelpie CLI
//!
//! TigerStyle: Command-line tools for Kelpie with explicit error handling.

mod client;
mod repl;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use client::{CreateAgentRequest, KelpieClient, DEFAULT_SERVER_URL};
use colored::Colorize;
use tracing_subscriber::EnvFilter;

/// Kelpie CLI
#[derive(Parser, Debug)]
#[command(name = "kelpie")]
#[command(about = "Kelpie distributed virtual actor system CLI")]
#[command(version)]
struct Cli {
    /// Enable verbose logging
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    /// Server URL (default: http://localhost:8283)
    #[arg(short, long, default_value = DEFAULT_SERVER_URL, global = true)]
    server: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Show server status
    Status,

    /// Agent management commands
    #[command(subcommand)]
    Agents(AgentsCommands),

    /// Interactive chat with an agent
    Chat {
        /// Agent ID to chat with
        agent_id: String,

        /// Disable streaming (receive full response at once)
        #[arg(long)]
        no_stream: bool,
    },

    /// Send a single message to an agent
    Invoke {
        /// Agent ID
        agent_id: String,

        /// Message to send
        message: String,

        /// Output raw JSON response
        #[arg(long)]
        json: bool,
    },

    /// Run diagnostics
    Doctor,
}

#[derive(Subcommand, Debug)]
enum AgentsCommands {
    /// List all agents
    List {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Get agent details
    Get {
        /// Agent ID
        agent_id: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Create a new agent
    Create {
        /// Agent name
        name: String,

        /// Agent type (memgpt, react, letta_v1)
        #[arg(short, long, default_value = "memgpt")]
        agent_type: String,

        /// LLM model to use
        #[arg(short, long, default_value = "claude-sonnet-4-20250514")]
        model: String,

        /// System prompt
        #[arg(long)]
        system: Option<String>,

        /// Description
        #[arg(short, long)]
        description: Option<String>,
    },

    /// Delete an agent
    Delete {
        /// Agent ID
        agent_id: String,

        /// Force delete without confirmation
        #[arg(short, long)]
        force: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize logging
    let filter = match cli.verbose {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| filter.into()))
        .init();

    // Create client
    let client = KelpieClient::new(&cli.server).context("Failed to create client")?;

    match cli.command {
        Commands::Status => cmd_status(client).await,
        Commands::Agents(sub) => match sub {
            AgentsCommands::List { json } => cmd_agents_list(client, json).await,
            AgentsCommands::Get { agent_id, json } => cmd_agents_get(client, &agent_id, json).await,
            AgentsCommands::Create {
                name,
                agent_type,
                model,
                system,
                description,
            } => cmd_agents_create(client, name, agent_type, model, system, description).await,
            AgentsCommands::Delete { agent_id, force } => {
                cmd_agents_delete(client, &agent_id, force).await
            }
        },
        Commands::Chat {
            agent_id,
            no_stream,
        } => cmd_chat(client, agent_id, !no_stream).await,
        Commands::Invoke {
            agent_id,
            message,
            json,
        } => cmd_invoke(client, &agent_id, &message, json).await,
        Commands::Doctor => cmd_doctor(client).await,
    }
}

/// Show server status
async fn cmd_status(client: KelpieClient) -> Result<()> {
    println!("{}", "Checking server status...".dimmed());

    match client.health().await {
        Ok(health) => {
            println!();
            println!("{} {}", "Status:".bold(), health.status.green());
            if !health.version.is_empty() {
                println!("{} {}", "Version:".bold(), health.version);
            }
            if let Some(count) = health.agent_count {
                println!("{} {}", "Agents:".bold(), count);
            }
            println!();
            Ok(())
        }
        Err(e) => {
            eprintln!();
            eprintln!("{} {}", "Failed to connect:".red().bold(), e);
            eprintln!();
            eprintln!(
                "{}",
                "Make sure the Kelpie server is running and accessible.".dimmed()
            );
            eprintln!("  Server URL: {}", client::DEFAULT_SERVER_URL);
            eprintln!();
            Err(e)
        }
    }
}

/// List agents
async fn cmd_agents_list(client: KelpieClient, json_output: bool) -> Result<()> {
    let response = client
        .list_agents()
        .await
        .context("Failed to list agents")?;

    if json_output {
        println!("{}", serde_json::to_string_pretty(&response.agents)?);
        return Ok(());
    }

    if response.agents.is_empty() {
        println!("{}", "No agents found.".dimmed());
        println!();
        println!("Create one with: {} create <name>", "kelpie agents".bold());
        return Ok(());
    }

    println!();
    println!("{} ({} total)", "Agents".bold(), response.agents.len());
    println!("{}", "-".repeat(60));

    for agent in &response.agents {
        println!("  {} {}", agent.id.cyan(), agent.name.bold());
        if !agent.agent_type.is_empty() {
            print!("    Type: {}", agent.agent_type);
        }
        if let Some(desc) = &agent.description {
            print!("  | {}", desc.chars().take(40).collect::<String>());
        }
        println!();
    }

    println!();
    Ok(())
}

/// Get agent details
async fn cmd_agents_get(client: KelpieClient, agent_id: &str, json_output: bool) -> Result<()> {
    let agent = client
        .get_agent(agent_id)
        .await
        .context("Failed to get agent")?;

    if json_output {
        println!("{}", serde_json::to_string_pretty(&agent)?);
        return Ok(());
    }

    println!();
    println!("{}", "Agent Details".bold());
    println!("{}", "-".repeat(40));
    println!("  {} {}", "ID:".bold(), agent.id);
    println!("  {} {}", "Name:".bold(), agent.name);
    println!("  {} {}", "Type:".bold(), agent.agent_type);
    println!("  {} {}", "Model:".bold(), agent.model);
    if let Some(desc) = &agent.description {
        println!("  {} {}", "Description:".bold(), desc);
    }
    if let Some(sys) = &agent.system {
        let preview: String = sys.chars().take(100).collect();
        println!(
            "  {} {}{}",
            "System:".bold(),
            preview,
            if sys.len() > 100 { "..." } else { "" }
        );
    }
    println!("  {} {}", "Created:".bold(), agent.created_at);
    println!();
    Ok(())
}

/// Create an agent
async fn cmd_agents_create(
    client: KelpieClient,
    name: String,
    agent_type: String,
    model: String,
    system: Option<String>,
    description: Option<String>,
) -> Result<()> {
    let request = CreateAgentRequest {
        name: name.clone(),
        agent_type: Some(agent_type),
        model: Some(model),
        system,
        description,
        memory_blocks: None,
    };

    let agent = client
        .create_agent(&request)
        .await
        .context("Failed to create agent")?;

    println!();
    println!(
        "{} Agent '{}' created with ID: {}",
        "Success!".green().bold(),
        name,
        agent.id.cyan()
    );
    println!();
    println!("Start chatting with: {} {}", "kelpie chat".bold(), agent.id);
    println!();
    Ok(())
}

/// Delete an agent
async fn cmd_agents_delete(client: KelpieClient, agent_id: &str, force: bool) -> Result<()> {
    if !force {
        // Confirm deletion
        println!(
            "{} Are you sure you want to delete agent '{}'? [y/N] ",
            "Warning:".yellow().bold(),
            agent_id
        );

        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;

        if !input.trim().eq_ignore_ascii_case("y") {
            println!("{}", "Cancelled.".dimmed());
            return Ok(());
        }
    }

    client
        .delete_agent(agent_id)
        .await
        .context("Failed to delete agent")?;

    println!(
        "{} Agent '{}' deleted.",
        "Success!".green().bold(),
        agent_id
    );
    Ok(())
}

/// Interactive chat
async fn cmd_chat(client: KelpieClient, agent_id: String, use_streaming: bool) -> Result<()> {
    let mut repl = repl::Repl::new(client, agent_id, use_streaming)
        .await
        .context("Failed to initialize chat")?;

    repl.run().await
}

/// Send single message
async fn cmd_invoke(
    client: KelpieClient,
    agent_id: &str,
    message: &str,
    json_output: bool,
) -> Result<()> {
    let response = client
        .send_message(agent_id, message)
        .await
        .context("Failed to send message")?;

    if json_output {
        println!("{}", serde_json::to_string_pretty(&response)?);
        return Ok(());
    }

    // Find the last assistant message
    let assistant_msg = response
        .messages
        .iter()
        .rev()
        .find(|m| m.role == "assistant");

    if let Some(msg) = assistant_msg {
        println!("{}", msg.content);
    } else {
        println!("{}", "No response from agent".yellow());
    }

    Ok(())
}

/// Run diagnostics
async fn cmd_doctor(client: KelpieClient) -> Result<()> {
    println!();
    println!("{}", "Kelpie CLI Diagnostics".bold());
    println!("{}", "=".repeat(40));
    println!();

    // Version info
    println!("{}", "Version Information".bold());
    println!("  CLI Version:  {}", env!("CARGO_PKG_VERSION"));
    println!();

    // Server connectivity
    println!("{}", "Server Connectivity".bold());
    print!("  Connecting to {}... ", DEFAULT_SERVER_URL);

    match client.health().await {
        Ok(health) => {
            println!("{}", "OK".green());
            println!("    Status:  {}", health.status);
            if !health.version.is_empty() {
                println!("    Version: {}", health.version);
            }
        }
        Err(e) => {
            println!("{}", "FAILED".red());
            println!("    Error: {}", e);
        }
    }
    println!();

    // Agent count
    println!("{}", "Agent Status".bold());
    match client.list_agents().await {
        Ok(response) => {
            println!("  Total agents: {}", response.agents.len());
            if !response.agents.is_empty() {
                println!("  First 5:");
                for agent in response.agents.iter().take(5) {
                    println!("    - {} ({})", agent.name, agent.id);
                }
            }
        }
        Err(e) => {
            println!("  {} {}", "Failed to list agents:".red(), e);
        }
    }
    println!();

    // Environment
    println!("{}", "Environment".bold());
    if std::env::var("ANTHROPIC_API_KEY").is_ok() {
        println!("  ANTHROPIC_API_KEY: {}", "Set".green());
    } else {
        println!("  ANTHROPIC_API_KEY: {}", "Not set".yellow());
    }
    println!();

    println!("{}", "Diagnostics complete!".green());
    println!();
    Ok(())
}
