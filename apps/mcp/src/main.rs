//! `bettercut-mcp`: the MCP server an assistant starts. Messages arrive on
//! standard input, one per line, and replies leave on standard output;
//! anything else — logs, warnings — goes to standard error, so it can never
//! be mistaken for a reply.

use std::io::{BufRead, Write};

fn main() {
    let mut server = bettercut_mcp::Server::new();
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else {
            break; // the client went away
        };
        if line.trim().is_empty() {
            continue;
        }
        if let Some(reply) = server.handle_line(&line)
            && writeln!(stdout, "{reply}")
                .and_then(|()| stdout.flush())
                .is_err()
        {
            break;
        }
    }
}
