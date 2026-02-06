//! Persway daemon module.
//!
//! Manages:
//! - A Unix socket for receiving CLI commands.
//! - Sway IPC event subscription and handling.
//! - Signal handling for graceful shutdown.
//! - Per‑workspace layout management via `MessageHandler`.

use super::message_handler::MessageHandler;
use crate::Args;
use crate::commands::PerswayCommand;
use crate::layout::WorkspaceLayout;
use crate::{commands::DaemonArgs, utils};
use anyhow::Result;
use clap::Parser;
use futures::SinkExt;
use futures::channel::mpsc;
use futures::{select, stream::StreamExt};
use signal_hook::consts::signal::{SIGHUP, SIGINT, SIGQUIT, SIGTERM};
use signal_hook_tokio::Signals;
use std::process::exit;
use swayipc_async::{Connection, Event, EventType};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::oneshot;

/// Generic sender type for cross‑task messaging.
pub type Sender<T> = mpsc::UnboundedSender<T>;

/// Message type sent over the internal channel.
///
/// Currently only used for CLI commands coming from the Unix socket.
#[derive(Debug)]
pub enum Message {
    /// A command received from the `persway` CLI client.
    CommandEvent(PerswayCommand, oneshot::Sender<anyhow::Result<()>>),
}

/// Persway daemon state.
///
/// Runs in the background and:
/// - Listens for Sway events.
/// - Handles Unix socket commands.
/// - Responds to signals (SIGHUP, SIGINT, SIGQUIT, SIGTERM).
pub struct Daemon {
    /// Optional command to run when the daemon exits (e.g., reset opacity).
    on_exit: Option<String>,
    /// Path to the Unix socket used for CLI IPC.
    socket_path: String,
    /// Message handler that manages workspaces and layouts.
    ///
    /// Wrapped in `Option` to allow async initialization in `run()`.
    message_handler: Option<MessageHandler>,
    /// Temporary storage of constructor arguments until `run()` is called.
    ///
    /// Holds:
    /// - The default layout for new workspaces.
    /// - Whether workspace renaming is enabled.
    /// - Focus/leave hooks for opacity or marking.
    init_args: Option<(WorkspaceLayout, bool, Option<String>, Option<String>)>,
}

impl Daemon {
    /// Construct a new `Daemon` from CLI arguments.
    ///
    /// The `message_handler` is left uninitialized; it will be created in `run()`.
    pub fn new(args: DaemonArgs, socket_path: Option<String>) -> Self {
        let socket_path = utils::get_socket_path(socket_path);
        let DaemonArgs {
            default_layout,
            stack_main_default_size,
            stack_main_default_stack_layout,
            workspace_renaming,
            on_window_focus,
            on_window_focus_leave,
            on_exit,
            ..
        } = args;

        let final_layout = match default_layout {
            WorkspaceLayout::StackMain { .. } => WorkspaceLayout::StackMain {
                size: stack_main_default_size,
                stack_layout: stack_main_default_stack_layout,
            },
            _ => default_layout,
        };

        Self {
            socket_path,
            on_exit,
            message_handler: None,
            init_args: Some((
                final_layout,
                workspace_renaming,
                on_window_focus,
                on_window_focus_leave,
            )),
        }
    }

    /// Handle Unix signals and run the `on_exit` command when triggered.
    ///
    /// Waits for the first of `SIGHUP`, `SIGINT`, `SIGQUIT`, or `SIGTERM`,
    /// then runs the configured `on_exit` command via Sway IPC before exiting.
    async fn handle_signals(mut signals: Signals, on_exit: Option<String>) {
        if let Some(_signal) = signals.next().await {
            if let Ok(mut commands) = Connection::new().await
                && let Some(exit_cmd) = on_exit
            {
                log::debug!("Executing exit command: {exit_cmd}");
                let _ = commands.run_command(exit_cmd).await;
            }
            exit(0);
        }
    }

    /// Run the daemon’s main loop.
    ///
    /// This async method:
    /// - Initializes the `MessageHandler`.
    /// - Sets up signal handling.
    /// - Binds a Unix socket and spawns an acceptor task.
    /// - Subscribes to Sway `Window` and `Workspace` events.
    /// - Runs a `select!` loop that dispatches:
    ///   - Sway events to `message_handler.handle_event`.
    ///   - New socket connections to `connection_loop`.
    ///   - CLI commands to `message_handler.handle_command`.
    pub async fn run(&mut self) -> Result<()> {
        // Initialize MessageHandler asynchronously (it needs a connection)
        if let Some((layout, renaming, focus, leave)) = self.init_args.take() {
            self.message_handler = Some(MessageHandler::new(layout, renaming, focus, leave).await?);
        }

        let signals = Signals::new([SIGHUP, SIGINT, SIGQUIT, SIGTERM])?;
        tokio::spawn(Self::handle_signals(signals, self.on_exit.clone()));

        // Subscribe to Window AND Workspace events
        let subs = [EventType::Window, EventType::Workspace];
        let mut sway_events = Connection::new().await?.subscribe(&subs).await?.fuse();

        match tokio::fs::remove_file(&self.socket_path).await {
            Ok(()) => log::debug!("Removed stale socket {}", &self.socket_path),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => log::error!("Unable to remove stale socket: {e}"),
        }

        let listener = UnixListener::bind(&self.socket_path)?;

        // Channel for CLI commands only
        let (sender, receiver) = mpsc::unbounded();
        let mut receiver = receiver.fuse();

        let (incoming_tx, incoming_rx) = mpsc::unbounded();
        let mut incoming_rx = incoming_rx.fuse();

        // Socket Acceptor Task
        tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((stream, _)) => {
                        if incoming_tx.unbounded_send(stream).is_err() {
                            break;
                        }
                    }
                    Err(e) => log::error!("Accept error: {e}"),
                }
            }
        });

        log::info!("Persway daemon started");

        loop {
            select! {
                            // 1. DIRECT EVENT HANDLING (Low Latency)
                            event = sway_events.select_next_some() => {
                                match event {
                                    Ok(Event::Window(event)) => {
                                        if let Some(handler) = &mut self.message_handler &&
                                            let Err(e) = handler.handle_event(event).await {
                                            log::error!("Error handling window event: {e}");
                                        }
                                    },
                                    Ok(Event::Workspace(_event)) => {
                                    }
                                    Err(e) => log::error!("Sway IPC event error: {e}"),
                                    _ => {} // Ignore other events
                                }
                            },

                            // 2. Accept new socket connections
                            stream = incoming_rx.select_next_some() => {
                                let sender = sender.clone();
                                tokio::spawn(async move {
                                    if let Err(e) = Self::connection_loop(stream, sender).await {
                                        log::error!("Connection loop error: {e}");
                                    }
                                });
                            },

                            // 3. Handle CLI commands
            message = receiver.select_next_some() => {
                match message {
                    Message::CommandEvent(command, reply_tx) => {
                        let res: anyhow::Result<()> = if let Some(handler) = &mut self.message_handler {
                            log::debug!("Executing CLI command: {command:?}");
                            handler.handle_command(command).await
                        } else {
                            Err(anyhow::anyhow!("daemon not initialized"))
                        };

                        let _ = reply_tx.send(res);
                    }
                }
            }

                        }
        }
    }

    /// Per‑connection loop that reads a single line command from a Unix socket.
    ///
    /// Parses the command via `clap::Parser` on `Args`, then sends the resulting
    /// `PerswayCommand` over `sender` as a `Message::CommandEvent`.
    ///
    /// # Behavior
    /// - On readable line: splits into `Vec<&str>`, parses as `Args`, sends command.
    /// - On EOF (0 bytes): returns `Ok(())` (connection closed).
    /// - On invalid command: logs an error and sends `fail: invalid command`.
    /// - On read/write error: logs an error (no return; caller exits).
    async fn connection_loop(stream: UnixStream, mut sender: Sender<Message>) -> Result<()> {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);
        let mut line = String::new();

        match reader.read_line(&mut line).await {
            Ok(0) => return Ok(()), // EOF
            Ok(_) => {
                let argv = line.trim().split_ascii_whitespace().collect::<Vec<_>>();

                match Args::try_parse_from(argv) {
                    Ok(myargs) => {
                        let (reply_tx, reply_rx) = oneshot::channel::<anyhow::Result<()>>();

                        if sender
                            .send(Message::CommandEvent(myargs.command, reply_tx))
                            .await
                            .is_err()
                        {
                            writer.write_all(b"fail: daemon unavailable\n").await?;
                            return Ok(());
                        }

                        match reply_rx.await {
                            Ok(Ok(())) => writer.write_all(b"success\n").await?,
                            Ok(Err(e)) => {
                                writer.write_all(format!("fail: {e}\n").as_bytes()).await?;
                            }
                            Err(_) => writer.write_all(b"fail: daemon dropped response\n").await?,
                        }
                    }
                    Err(e) => {
                        // Optional: include clap's error text
                        log::error!("Invalid command: {e}");
                        writer.write_all(b"fail: invalid command\n").await?;
                    }
                }
            }
            Err(e) => {
                log::error!("Socket read error: {e}");
            }
        }

        Ok(())
    }
}
