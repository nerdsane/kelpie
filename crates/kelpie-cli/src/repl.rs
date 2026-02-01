//! Interactive REPL for chatting with agents
//!
//! TigerStyle: Explicit state machine for user interaction.

use crate::client::KelpieClient;
use anyhow::{Context, Result};
use colored::Colorize;
use futures::StreamExt;
use rustyline::error::ReadlineError;
use rustyline::history::FileHistory;
use rustyline::Editor;
use std::io::{self, Write};
use std::path::PathBuf;

/// History file name
const HISTORY_FILE: &str = ".kelpie_history";

/// Maximum history entries
const HISTORY_MAX_ENTRIES: usize = 1000;

/// REPL state
pub struct Repl {
    client: KelpieClient,
    agent_id: String,
    agent_name: String,
    editor: Editor<(), FileHistory>,
    use_streaming: bool,
}

impl Repl {
    /// Create a new REPL for the given agent
    pub async fn new(client: KelpieClient, agent_id: String, use_streaming: bool) -> Result<Self> {
        // Get agent info
        let agent = client
            .get_agent(&agent_id)
            .await
            .context("Failed to get agent info")?;

        // Create editor with history
        let config = rustyline::Config::builder()
            .history_ignore_space(true)
            .max_history_size(HISTORY_MAX_ENTRIES)
            .context("Failed to build editor config")?
            .build();

        let mut editor = Editor::with_config(config).context("Failed to create editor")?;

        // Load history
        let history_path = Self::history_path();
        if history_path.exists() {
            let _ = editor.load_history(&history_path);
        }

        Ok(Self {
            client,
            agent_id,
            agent_name: agent.name,
            editor,
            use_streaming,
        })
    }

    /// Get history file path
    fn history_path() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(HISTORY_FILE)
    }

    /// Run the REPL loop
    pub async fn run(&mut self) -> Result<()> {
        println!();
        println!(
            "{}",
            format!(
                "Connected to agent: {} ({})",
                self.agent_name.bold(),
                self.agent_id
            )
            .green()
        );
        println!(
            "{}",
            "Type your message, /help for commands, or /quit to exit.".dimmed()
        );
        if self.use_streaming {
            println!("{}", "Streaming mode enabled.".dimmed());
        }
        println!();

        let prompt = format!("{} ", "You:".blue().bold());

        loop {
            match self.editor.readline(&prompt) {
                Ok(line) => {
                    let input = line.trim();

                    // Skip empty lines
                    if input.is_empty() {
                        continue;
                    }

                    // Add to history
                    let _ = self.editor.add_history_entry(input);

                    // Handle commands
                    if input.starts_with('/') {
                        match input {
                            "/quit" | "/exit" | "/q" => {
                                println!("{}", "Goodbye!".dimmed());
                                break;
                            }
                            "/help" | "/h" | "/?" => {
                                self.print_help();
                            }
                            "/clear" => {
                                print!("\x1B[2J\x1B[1;1H");
                                io::stdout().flush().ok();
                            }
                            "/stream" => {
                                self.use_streaming = !self.use_streaming;
                                println!(
                                    "Streaming mode: {}",
                                    if self.use_streaming {
                                        "enabled"
                                    } else {
                                        "disabled"
                                    }
                                );
                            }
                            "/info" => {
                                self.print_agent_info().await?;
                            }
                            _ => {
                                println!("{} {}", "Unknown command:".red(), input);
                            }
                        }
                        continue;
                    }

                    // Send message
                    if self.use_streaming {
                        self.send_streaming(input).await?;
                    } else {
                        self.send_message(input).await?;
                    }
                }
                Err(ReadlineError::Interrupted) => {
                    println!("{}", "^C".dimmed());
                    continue;
                }
                Err(ReadlineError::Eof) => {
                    println!("{}", "Goodbye!".dimmed());
                    break;
                }
                Err(err) => {
                    eprintln!("{} {:?}", "Error:".red(), err);
                    break;
                }
            }
        }

        // Save history
        let history_path = Self::history_path();
        if let Err(e) = self.editor.save_history(&history_path) {
            eprintln!("{} {:?}", "Failed to save history:".yellow(), e);
        }

        Ok(())
    }

    /// Send a message and print response
    async fn send_message(&self, content: &str) -> Result<()> {
        print!("{}", "Thinking...".dimmed());
        io::stdout().flush().ok();

        let response = self.client.send_message(&self.agent_id, content).await;

        // Clear "Thinking..." text
        print!("\r                \r");
        io::stdout().flush().ok();

        match response {
            Ok(resp) => {
                // Find the last assistant message
                let assistant_msg = resp.messages.iter().rev().find(|m| m.role == "assistant");

                if let Some(msg) = assistant_msg {
                    println!("{} {}", "Agent:".green().bold(), msg.content);
                } else {
                    println!("{}", "No response from agent".yellow());
                }

                // Show usage stats
                if let Some(usage) = resp.usage {
                    println!(
                        "{}",
                        format!(
                            "[tokens: {} prompt, {} completion]",
                            usage.prompt_tokens, usage.completion_tokens
                        )
                        .dimmed()
                    );
                }
            }
            Err(e) => {
                eprintln!("{} {}", "Error:".red(), e);
            }
        }

        println!();
        Ok(())
    }

    /// Send a message with streaming response
    async fn send_streaming(&self, content: &str) -> Result<()> {
        print!("{} ", "Agent:".green().bold());
        io::stdout().flush().ok();

        let response = self
            .client
            .send_message_stream(&self.agent_id, content)
            .await;

        match response {
            Ok(resp) => {
                let mut stream = resp.bytes_stream();
                let mut buffer = String::new();

                while let Some(chunk_result) = stream.next().await {
                    match chunk_result {
                        Ok(bytes) => {
                            buffer.push_str(&String::from_utf8_lossy(&bytes));

                            // Process complete SSE events
                            while let Some(event_end) = buffer.find("\n\n") {
                                let event = buffer[..event_end].to_string();
                                buffer = buffer[event_end + 2..].to_string();

                                // Parse SSE data lines
                                for line in event.lines() {
                                    if let Some(data) = line.strip_prefix("data: ") {
                                        if data == "[DONE]" {
                                            continue;
                                        }

                                        // Try to parse as JSON
                                        if let Ok(json) =
                                            serde_json::from_str::<serde_json::Value>(data)
                                        {
                                            // Extract content delta
                                            if let Some(text) = json
                                                .get("choices")
                                                .and_then(|c| c.get(0))
                                                .and_then(|c| c.get("delta"))
                                                .and_then(|d| d.get("content"))
                                                .and_then(|c| c.as_str())
                                            {
                                                print!("{}", text);
                                                io::stdout().flush().ok();
                                            }

                                            // Or check for message content directly
                                            if let Some(text) =
                                                json.get("content").and_then(|c| c.as_str())
                                            {
                                                print!("{}", text);
                                                io::stdout().flush().ok();
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("\n{} {}", "Stream error:".red(), e);
                            break;
                        }
                    }
                }

                println!();
            }
            Err(e) => {
                println!();
                eprintln!("{} {}", "Error:".red(), e);
            }
        }

        println!();
        Ok(())
    }

    /// Print help information
    fn print_help(&self) {
        println!();
        println!("{}", "Commands:".bold());
        println!("  /help, /h, /?   Show this help");
        println!("  /quit, /exit, /q  Exit the chat");
        println!("  /clear          Clear the screen");
        println!("  /stream         Toggle streaming mode");
        println!("  /info           Show agent information");
        println!();
    }

    /// Print agent information
    async fn print_agent_info(&self) -> Result<()> {
        match self.client.get_agent(&self.agent_id).await {
            Ok(agent) => {
                println!();
                println!("{}", "Agent Information:".bold());
                println!("  ID:          {}", agent.id);
                println!("  Name:        {}", agent.name);
                println!("  Type:        {}", agent.agent_type);
                println!("  Model:       {}", agent.model);
                if let Some(desc) = agent.description {
                    println!("  Description: {}", desc);
                }
                if let Some(sys) = agent.system {
                    let truncated: String = sys.chars().take(100).collect();
                    println!(
                        "  System:      {}{}",
                        truncated,
                        if sys.len() > 100 { "..." } else { "" }
                    );
                }
                println!("  Created:     {}", agent.created_at);
                println!();
            }
            Err(e) => {
                eprintln!("{} {}", "Failed to get agent info:".red(), e);
            }
        }
        Ok(())
    }
}
