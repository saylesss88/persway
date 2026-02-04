use super::super::traits::WindowEventHandler;
use anyhow::Result;
use swayipc_async::{Connection, WindowChange, WindowEvent};

#[allow(clippy::struct_field_names)]
#[derive(Debug)]
pub struct WindowFocus {
    connection: Connection,
    window_focus_cmd: Option<String>,
    window_focus_leave_cmd: Option<String>,
    previously_focused_id: Option<i64>,
}

impl WindowFocus {
    /// Constructor: Call this ONCE when initializing your application
    pub async fn new(
        window_focus_cmd: Option<String>,
        window_focus_leave_cmd: Option<String>,
    ) -> Result<Self> {
        // We create the connection here, just once.
        let connection = Connection::new().await?;
        Ok(Self {
            connection,
            window_focus_cmd,
            window_focus_leave_cmd,
            previously_focused_id: None,
        })
    }

    /// Private helper to execute commands
    async fn run_cmd(&mut self, cmd: Option<String>, context: &str, id: Option<i64>) {
        let Some(cmd_str) = cmd else { return };

        // If we have a specific ID, target it. Otherwise, run on the currently focused window.
        let final_cmd = match id {
            Some(i) => format!("[con_id={i}] {cmd_str}"),
            None => cmd_str,
        };

        if let Err(e) = self.connection.run_command(final_cmd).await {
            // Note: Errors here are expected if the window was just closed (id no longer exists)
            log::debug!("workspace window focus manager {context}, err: {e}");
        }
    }
}

impl WindowEventHandler for WindowFocus {
    async fn handle(&mut self, event: Box<WindowEvent>) {
        match event.change {
            WindowChange::Focus => {
                let leave_cmd = self.window_focus_leave_cmd.clone();
                let focus_cmd = self.window_focus_cmd.clone();

                // 1. Apply 'leave' command to the PREVIOUS window
                // This now works because self.previously_focused_id persists!
                if let Some(prev_id) = self.previously_focused_id {
                    // optimization: don't run leave if focusing the same window
                    if prev_id != event.container.id {
                        self.run_cmd(leave_cmd, "on_window_focus_leave", Some(prev_id))
                            .await;
                    }
                }

                // 2. Apply 'focus' command to the NEW window
                // passing None targets the currently focused window (event.container.id)
                self.run_cmd(focus_cmd, "on_window_focus", None).await;

                // 3. Update state for next time
                self.previously_focused_id = Some(event.container.id);
            }
            WindowChange::Close => {
                // If the closed window was the one we were tracking, clear it
                // so we don't try to run commands on a dead ID later.
                if let Some(prev_id) = self.previously_focused_id &&
                    prev_id == event.container.id {
                        self.previously_focused_id = None;
                    }
                
            }
            _ => log::debug!(
                "workspace name manager, not handling event: {:?}",
                event.change
            ),
        }
    }
}
