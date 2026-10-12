//! `fontcraft-cli mcp`.

use super::Subcommand;
use crate::io::{Failure, Outcome, usage};
use std::path::Path;

use fontcraft_engine::file_access::AutomationRoots;
use fontcraft_mcp::{Backend, DEFAULT_ADDR, Headless, Policy, Remote, Server};

pub const SUBCOMMAND: Subcommand =
    Subcommand { name: "mcp", summary: "Run the MCP server on stdio", usage: USAGE, options: &[], actions: &[], run: mcp };

const USAGE: &str = "\
  fontcraft-cli mcp [--connect ADDR | --headless] [--automation-read-root DIR]
                    [--automation-write-root DIR] [--read-only] [--allow PATTERN]... [--deny PATTERN]...
      Run the MCP server on stdio. Default: attach to a running app at 127.0.0.1:7985
      (fontcraft --control 7985), falling back to a headless session with a new font.
      --automation-read-root DIR: the files commands read must lie inside DIR;
      --automation-write-root DIR: the files they write must lie inside DIR. Each is given once
      (also as --flag=DIR) and must be an existing folder. A root left out grants none of its
      access; without either, files are not confined. The roots confine the headless server and
      imply --headless; a running app confines itself (fontcraft --control PORT
      --automation-read-root DIR ...). --read-only: only commands that read and write no files
      run. --allow PATTERN: only commands whose id matches a pattern run; --deny PATTERN:
      commands that match are refused, even when allowed. Patterns are shell-style (* and ?)
      over whole command ids, for example 'glyph.*'. With roots or any of these options,
      macros are refused. Refused commands are tool errors; they change nothing.
";

fn mcp(args: &[String]) -> Outcome {
    let mut connect = None;
    let mut headless = false;
    let mut policy = Policy::unrestricted();
    let (mut read_root, mut write_root): (Option<String>, Option<String>) = (None, None);
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        // The roots also come as `--flag=DIR`.
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) if f.starts_with("--automation-") => (f, Some(v.to_string())),
            _ => (arg.as_str(), None),
        };
        match flag {
            "--connect" => connect = Some(it.next().ok_or_else(|| usage("--connect needs an address"))?.clone()),
            "--headless" => headless = true,
            "--read-only" => policy = policy.read_only(true),
            "--allow" => {
                policy = policy.allow(it.next().ok_or_else(|| usage("--allow needs a pattern"))?).map_err(|e| usage(format!("--allow: {e}")))?
            }
            "--deny" => policy = policy.deny(it.next().ok_or_else(|| usage("--deny needs a pattern"))?).map_err(|e| usage(format!("--deny: {e}")))?,
            "--automation-read-root" | "--automation-write-root" => {
                let dir = inline.or_else(|| it.next().cloned()).ok_or_else(|| usage(format!("{flag} needs a folder")))?;
                let slot = if flag == "--automation-read-root" { &mut read_root } else { &mut write_root };
                if slot.replace(dir).is_some() {
                    return Err(usage(format!("{flag} is given twice: it takes one folder")));
                }
            }
            other => return Err(usage(format!("unknown mcp option `{other}`"))),
        }
    }
    if headless && connect.is_some() {
        return Err(usage("use either --connect or --headless"));
    }
    let roots = AutomationRoots::new(read_root.as_deref().map(Path::new), write_root.as_deref().map(Path::new)).map_err(Failure::Failed)?;
    if roots.is_some() && connect.is_some() {
        return Err(usage(
            "--automation-read-root and --automation-write-root confine the headless server; a running app confines itself: start it with `fontcraft --control <port> --automation-read-root <dir> --automation-write-root <dir>`",
        ));
    }
    // A headless session is an editing session like the app's: changed glyphs get Last Changed
    // stamps. The other subcommands leave them out, so their output does not depend on the time.
    let headless_session = || {
        let mut h = Headless::with_document();
        h.0.set_clock(Some(fontcraft_engine::clock::system_clock()));
        h
    };
    let backend: Box<dyn Backend> = if let Some(roots) = &roots {
        eprintln!("fontcraft-cli: {}", roots.describe());
        Box::new(headless_session())
    } else if headless {
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
        .with_roots(roots)
        .serve(std::io::stdin().lock(), std::io::stdout().lock())
        .map_err(|e| Failure::Failed(format!("MCP transport: {e}")))
}
