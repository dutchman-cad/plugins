use std::{
    io::{self, BufRead, Write},
    sync::mpsc::{self, Receiver, RecvTimeoutError, Sender},
    time::Duration,
};

/// Read only on request: never consume a physical confirmation as a panel event.
/// A blocked terminal read stays off the refresh thread on every platform.
pub struct Lines {
    request: Sender<()>,
    response: Receiver<io::Result<String>>,
    pending: bool,
}
impl Lines {
    pub fn new() -> io::Result<Self> {
        let (request, requests) = mpsc::channel();
        let (response, responses) = mpsc::channel();
        std::thread::Builder::new()
            .name("panel-input".into())
            .spawn(move || {
                while requests.recv().is_ok() {
                    let mut line = String::new();
                    let result = io::stdin().read_line(&mut line).map(|_| line);
                    if response.send(result).is_err() {
                        break;
                    }
                }
            })?;
        Ok(Self {
            request,
            response: responses,
            pending: false,
        })
    }
    pub fn next(&mut self, timeout: Duration) -> io::Result<Option<String>> {
        if !self.pending {
            self.request
                .send(())
                .map_err(|_| io::Error::other("Input reader stopped"))?;
            self.pending = true;
        }
        match self.response.recv_timeout(timeout) {
            Ok(line) => {
                self.pending = false;
                line.map(Some)
            }
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err(io::Error::other("Input reader stopped")),
        }
    }
}

pub fn confirm(action: &str) -> io::Result<String> {
    // Windows console output handles need read access as well as write access.
    let mut output = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(if cfg!(windows) { "CONOUT$" } else { "/dev/tty" })?;
    let input = std::fs::File::open(if cfg!(windows) { "CONIN$" } else { "/dev/tty" })?;
    write!(output, "Press {action}? Type y to confirm: ")?;
    output.flush()?;
    let mut answer = String::new();
    io::BufReader::new(input).read_line(&mut answer)?;
    Ok(answer)
}
