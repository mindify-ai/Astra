//! The in-app terminal: a login shell in a pseudo-terminal per tab. Output streams to the webview as
//! `astra://terminal/<id>/data` events (UTF-8 text) and `astra://terminal/<id>/exit` when the shell ends.
//! The webview picks the id, so it can listen before the shell starts and miss nothing.

#[cfg(desktop)]
pub use imp::*;
#[cfg(mobile)]
pub use stub::*;

#[cfg(desktop)]
mod imp {
    use std::collections::HashMap;
    use std::io::{Read, Write};
    use std::path::PathBuf;
    use std::sync::Mutex;

    use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
    use tauri::{AppHandle, Emitter, Manager, State};

    struct Term {
        master: Box<dyn MasterPty + Send>,
        writer: Box<dyn Write + Send>,
        child: Box<dyn Child + Send + Sync>,
    }

    #[derive(Default)]
    pub struct Terminals(Mutex<HashMap<String, Term>>);

    fn size(cols: u16, rows: u16) -> PtySize {
        PtySize {
            rows: rows.max(1),
            cols: cols.max(1),
            pixel_width: 0,
            pixel_height: 0,
        }
    }

    fn shell() -> CommandBuilder {
        #[cfg(windows)]
        {
            CommandBuilder::new(
                std::env::var("COMSPEC").unwrap_or_else(|_| "powershell.exe".into()),
            )
        }
        #[cfg(not(windows))]
        {
            let mut cmd =
                CommandBuilder::new(std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into()));
            cmd.arg("-l");
            cmd
        }
    }

    #[tauri::command]
    pub fn terminal_open(
        app: AppHandle,
        terminals: State<'_, Terminals>,
        id: String,
        cwd: Option<String>,
        cols: u16,
        rows: u16,
    ) -> Result<(), String> {
        if !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return Err("invalid terminal id".into());
        }
        let pair = native_pty_system()
            .openpty(size(cols, rows))
            .map_err(|e| e.to_string())?;
        let mut cmd = shell();
        let dir = cwd
            .map(PathBuf::from)
            .filter(|p| p.is_dir())
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from));
        if let Some(dir) = dir {
            cmd.cwd(dir);
        }
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        let child = pair.slave.spawn_command(cmd).map_err(|e| e.to_string())?;
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
        let writer = pair.master.take_writer().map_err(|e| e.to_string())?;
        terminals.0.lock().map_err(|e| e.to_string())?.insert(
            id.clone(),
            Term {
                master: pair.master,
                writer,
                child,
            },
        );

        std::thread::spawn(move || {
            let data_event = format!("astra://terminal/{id}/data");
            let mut buf = [0u8; 8192];
            let mut pending: Vec<u8> = Vec::new();
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        pending.extend_from_slice(&buf[..n]);
                        let text = take_utf8(&mut pending);
                        if !text.is_empty() {
                            let _ = app.emit(&data_event, text);
                        }
                    }
                }
            }
            let code = app
                .state::<Terminals>()
                .0
                .lock()
                .ok()
                .and_then(|mut t| t.remove(&id))
                .and_then(|mut t| t.child.wait().ok())
                .map(|s| s.exit_code());
            let _ = app.emit(&format!("astra://terminal/{id}/exit"), code);
        });
        Ok(())
    }

    #[tauri::command]
    pub fn terminal_write(
        terminals: State<'_, Terminals>,
        id: String,
        data: String,
    ) -> Result<(), String> {
        let mut t = terminals.0.lock().map_err(|e| e.to_string())?;
        let term = t.get_mut(&id).ok_or("terminal closed")?;
        term.writer
            .write_all(data.as_bytes())
            .and_then(|_| term.writer.flush())
            .map_err(|e| e.to_string())
    }

    #[tauri::command]
    pub fn terminal_resize(
        terminals: State<'_, Terminals>,
        id: String,
        cols: u16,
        rows: u16,
    ) -> Result<(), String> {
        let t = terminals.0.lock().map_err(|e| e.to_string())?;
        let term = t.get(&id).ok_or("terminal closed")?;
        term.master
            .resize(size(cols, rows))
            .map_err(|e| e.to_string())
    }

    #[tauri::command]
    pub fn terminal_close(terminals: State<'_, Terminals>, id: String) -> Result<(), String> {
        // Killing the shell ends the reader thread, which removes the entry and reports the exit.
        if let Some(term) = terminals.0.lock().map_err(|e| e.to_string())?.get_mut(&id) {
            let _ = term.child.kill();
        }
        Ok(())
    }

    /// Takes the longest valid UTF-8 prefix of `pending`, keeping an incomplete trailing character for
    /// the next read. Invalid bytes become U+FFFD.
    pub(crate) fn take_utf8(pending: &mut Vec<u8>) -> String {
        match std::str::from_utf8(pending) {
            Ok(s) => {
                let s = s.to_string();
                pending.clear();
                s
            }
            Err(e) if e.error_len().is_none() => {
                let valid = e.valid_up_to();
                let s = String::from_utf8_lossy(&pending[..valid]).into_owned();
                pending.drain(..valid);
                s
            }
            Err(_) => {
                let s = String::from_utf8_lossy(pending).into_owned();
                pending.clear();
                s
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::take_utf8;

        #[test]
        fn keeps_split_characters_for_the_next_read() {
            let bytes = "é漢".as_bytes();
            let mut pending = bytes[..3].to_vec(); // "é" and the first byte of "漢"
            assert_eq!(take_utf8(&mut pending), "é");
            pending.extend_from_slice(&bytes[3..]);
            assert_eq!(take_utf8(&mut pending), "漢");
            assert!(pending.is_empty());
        }

        #[test]
        fn replaces_invalid_bytes() {
            let mut pending = vec![b'a', 0xff, b'b'];
            assert_eq!(take_utf8(&mut pending), "a\u{fffd}b");
        }
    }
}

#[cfg(mobile)]
mod stub {
    const UNAVAILABLE: &str = "the terminal is available in the desktop app";

    #[derive(Default)]
    pub struct Terminals;

    #[tauri::command]
    pub fn terminal_open(
        id: String,
        cwd: Option<String>,
        cols: u16,
        rows: u16,
    ) -> Result<(), String> {
        let _ = (id, cwd, cols, rows);
        Err(UNAVAILABLE.into())
    }
    #[tauri::command]
    pub fn terminal_write(id: String, data: String) -> Result<(), String> {
        let _ = (id, data);
        Err(UNAVAILABLE.into())
    }
    #[tauri::command]
    pub fn terminal_resize(id: String, cols: u16, rows: u16) -> Result<(), String> {
        let _ = (id, cols, rows);
        Err(UNAVAILABLE.into())
    }
    #[tauri::command]
    pub fn terminal_close(id: String) -> Result<(), String> {
        let _ = id;
        Err(UNAVAILABLE.into())
    }
}
