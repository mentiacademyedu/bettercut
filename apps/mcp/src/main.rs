//! `bettercut-mcp`: the MCP server an assistant starts. Messages arrive on
//! standard input, one per line, and replies leave on standard output;
//! anything else — logs, warnings — goes to standard error, so it can never
//! be mistaken for a reply.

fn main() {
    bettercut_mcp::serve_stdio();
}
