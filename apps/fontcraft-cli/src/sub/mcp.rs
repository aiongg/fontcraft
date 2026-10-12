//! `fontcraft-cli mcp`.

use super::Subcommand;
use crate::io::{Failure, Outcome, usage};
use fontcraft_mcp::{Backend, DEFAULT_ADDR, Headless, Policy, Remote, Server};

pub const SUBCOMMAND: Subcommand =
    Subcommand { name: "mcp", summary: "Run the MCP server on stdio", usage: USAGE, options: &[], actions: &[], run: mcp };

const USAGE: &str = "\
  fontcraft-cli mcp [--connect ADDR | --headless] [--root DIR]... [--read-only]
                    [--allow PATTERN]... [--deny PATTERN]...
      Run the MCP server on stdio. Default: attach to a running app at 127.0.0.1:7985
      (fontcraft --control 7985), falling back to a headless session with a new font.
      --root DIR: file parameters must lie inside DIR (repeatable; default the working
      directory; --root / admits every path). --read-only: only commands that read run.
      --allow PATTERN: only commands whose id matches a pattern run; --deny PATTERN: commands
      that match are refused, even when allowed. Patterns are shell-style (* and ?) over whole
      command ids, for example 'glyph.*'. Refused commands are tool errors; they change nothing.
";

fn mcp(args: &[String]) -> Outcome {
    let mut connect = None;
    let mut headless = false;
    let mut policy = Policy::unrestricted();
    let mut roots = 0usize;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--connect" => connect = Some(it.next().ok_or_else(|| usage("--connect needs an address"))?.clone()),
            "--headless" => headless = true,
            "--read-only" => policy = policy.read_only(true),
            "--allow" => {
                policy = policy.allow(it.next().ok_or_else(|| usage("--allow needs a pattern"))?).map_err(|e| usage(format!("--allow: {e}")))?
            }
            "--deny" => policy = policy.deny(it.next().ok_or_else(|| usage("--deny needs a pattern"))?).map_err(|e| usage(format!("--deny: {e}")))?,
            "--root" => {
                let dir = it.next().ok_or_else(|| usage("--root needs a folder"))?;
                policy = policy.root(std::path::Path::new(dir)).map_err(|e| usage(format!("--root: {e}")))?;
                roots += 1;
            }
            other => return Err(usage(format!("unknown mcp option `{other}`"))),
        }
    }
    if roots == 0 {
        let cwd = std::env::current_dir().map_err(|e| Failure::Failed(format!("cannot read the working directory: {e}")))?;
        policy = policy.root(&cwd).map_err(|e| Failure::Failed(format!("the working directory as root: {e}")))?;
    }
    if headless && connect.is_some() {
        return Err(usage("use either --connect or --headless"));
    }
    // A headless session is an editing session like the app's: changed glyphs get Last Changed
    // stamps. The other subcommands leave them out, so their output does not depend on the time.
    let headless_session = || {
        let mut h = Headless::with_document();
        h.0.set_clock(Some(fontcraft_engine::clock::system_clock()));
        h
    };
    let backend: Box<dyn Backend> = if headless {
        Box::new(headless_session())
    } else if let Some(addr) = connect {
        // An explicit address: fail loudly if the app is not there.
        Box::new(Remote::connect(&addr).map_err(|e| Failure::Failed(format!("cannot connect to {addr}: {e}")))?)
    } else {
        match Remote::connect(DEFAULT_ADDR) {
            Ok(remote) => Box::new(remote),
            Err(_) => {
                eprintln!("fontcraft-cli: no app at {DEFAULT_ADDR}; running a headless session");
                Box::new(headless_session())
            }
        }
    };
    // stdout carries only protocol messages; everything else goes to stderr.
    Server::with_policy(backend, policy)
        .serve(std::io::stdin().lock(), std::io::stdout().lock())
        .map_err(|e| Failure::Failed(format!("MCP transport: {e}")))
}
