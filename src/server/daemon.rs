use super::message_handler::MessageHandler;
use crate::commands::PerswayCommand;
use crate::layout::WorkspaceLayout;
use crate::Args;
use crate::{commands::DaemonArgs, utils};
use anyhow::Result;
use clap::Parser;
use futures::channel::mpsc;
use futures::SinkExt;
use futures::{select, stream::StreamExt};
use signal_hook::consts::signal::{SIGHUP, SIGINT, SIGQUIT, SIGTERM};
use signal_hook_tokio::Signals;
use std::process::exit;
use swayipc_async::{Connection, Event, EventType};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

pub type Sender<T> = mpsc::UnboundedSender<T>;

// Only use the channel for external CLI commands
pub enum Message {
    CommandEvent(PerswayCommand),
}

pub struct Daemon {
    on_exit: Option<String>,
    socket_path: String,
    // Option allows us to take it out if needed, but mainly for async init
    message_handler: Option<MessageHandler>,
    // Store init args temporarily until run() is called
    init_args: Option<(WorkspaceLayout, bool, Option<String>, Option<String>)>,
}

impl Daemon {
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

    async fn handle_signals(mut signals: Signals, on_exit: Option<String>) {
        if let Some(_signal) = signals.next().await {
            if let Ok(mut commands) = Connection::new().await &&
                let Some(exit_cmd) = on_exit {
                    log::debug!("Executing exit command: {exit_cmd}");
                    let _ = commands.run_command(exit_cmd).await;
                
            }
            exit(0)
        }
    }

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
                            // Add workspace event handling
                            // if let Some(handler) = &mut self.message_handler {
                            //    handler.handle_workspace_event(event).await?;
                            // }
                        }
                        Err(e) => log::error!("Sway IPC event error: {e}"),
                        _ => {} // Ignore other events
                    }
                },

                // 2. Accept new socket connections
                stream = incoming_rx.select_next_some() => {
                    tokio::spawn(Self::connection_loop(stream, sender.clone()));
                },

                // 3. Handle CLI commands
                message = receiver.select_next_some() => {
                    match message {
                        Message::CommandEvent(command) => {
                            if let Some(handler) = &mut self.message_handler {
                                log::debug!("Executing CLI command: {command:?}");
                                if let Err(e) = handler.handle_command(command).await {
                                    log::error!("Command execution failed: {e}");
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    async fn connection_loop(mut stream: UnixStream, mut sender: Sender<Message>) -> Result<()> {
        let mut message = String::new();
        match stream.read_to_string(&mut message).await {
            Ok(_) => match Args::try_parse_from(message.split_ascii_whitespace()) {
                Ok(args) => {
                    sender.send(Message::CommandEvent(args.command)).await?;
                    let _ = stream.write_all(b"success\n").await;
                }
                Err(e) => {
                    log::error!("Invalid command: {e}");
                    let _ = stream.write_all(b"fail: invalid command\n").await;
                }
            },
            Err(e) => log::error!("Socket read error: {e}"),
        }
        Ok(())
    }
}
