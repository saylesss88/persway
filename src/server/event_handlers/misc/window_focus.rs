use super::super::traits::WindowEventHandler;
use anyhow::Result;
use swayipc_async::{Connection, WindowChange, WindowEvent};

#[allow(clippy::struct_field_names)]
pub struct WindowFocus {
    connection: Connection,
    window_focus_cmd: Option<String>,
    window_focus_leave_cmd: Option<String>,
    previously_focused_id: Option<i64>,
}

impl WindowFocus {
    /// Static entry point to create a manager and handle a single event
    pub async fn run(
        event: Box<WindowEvent>,
        window_focus_cmd: Option<String>,
        window_focus_leave_cmd: Option<String>,
    ) {
        if let Ok(mut manager) = Self::new(window_focus_cmd, window_focus_leave_cmd).await {
            manager.handle(event).await;
        }
    }

    /// Constructor to initialize the connection and state
    pub async fn new(
        window_focus_cmd: Option<String>,
        window_focus_leave_cmd: Option<String>,
    ) -> Result<Self> {
        let connection = Connection::new().await?;
        Ok(Self {
            connection,
            window_focus_cmd,
            window_focus_leave_cmd,
            previously_focused_id: None,
        })
    }

    /// Private helper to execute commands and handle errors
    async fn run_cmd(&mut self, cmd: Option<String>, context: &str, id: Option<i64>) {
        if let Some(cmd_str) = cmd {
            let final_cmd = match id {
                Some(i) => format!("[con_id={i}] {cmd_str}"),
                None => cmd_str,
            };

            if let Err(e) = self.connection.run_command(final_cmd).await {
                log::error!("workspace window focus manager {context}, err: {e}");
            }
        }
    }
}

impl WindowEventHandler for WindowFocus {
    async fn handle(&mut self, event: Box<WindowEvent>) {
        match event.change {
            WindowChange::Focus => {
                let leave_cmd = self.window_focus_leave_cmd.clone();
                let focus_cmd = self.window_focus_cmd.clone();

                // 1. Leave the old window
                self.run_cmd(
                    leave_cmd,
                    "on_window_focus_leave",
                    self.previously_focused_id,
                )
                .await;

                // 2. Focus the new window
                self.run_cmd(focus_cmd, "on_window_focus", None).await;

                self.previously_focused_id = Some(event.container.id);
            }
            WindowChange::Close => {
                let leave_cmd = self.window_focus_leave_cmd.clone();
                self.run_cmd(
                    leave_cmd,
                    "on_window_focus_leave",
                    self.previously_focused_id,
                )
                .await;

                self.previously_focused_id = Some(event.container.id);
            }
            _ => log::debug!(
                "workspace name manager, not handling event: {:?}",
                event.change
            ),
        }
    }
}
