//! WASM sandbox using Wasmtime + WASI for isolated execution.

#[cfg(feature = "wasm")]
use std::{collections::HashMap, path::PathBuf, sync::Arc};

#[cfg(feature = "wasm")]
use super::platform::parse_memory_limit;
#[cfg(feature = "wasm")]
use super::types::{
    HomePersistence, Sandbox, SandboxConfig, SandboxId, sanitize_path_component,
    truncate_output_for_display,
};
#[cfg(feature = "wasm")]
use crate::error::{Context, Error, Result};
#[cfg(feature = "wasm")]
use crate::exec::{ExecOpts, ExecResult};
#[cfg(feature = "wasm")]
use crate::wasm_engine::WasmComponentEngine;
#[cfg(feature = "wasm")]
use async_trait::async_trait;

// ---------------------------------------------------------------------------
// WASM sandbox (real Wasmtime + WASI isolation)
// ---------------------------------------------------------------------------

/// Real WASM sandbox that uses Wasmtime + WASI for isolated execution.
///
/// Two execution tiers:
/// - **Built-in commands** (~20 common coreutils): echo, cat, ls, mkdir, rm,
///   cp, mv, pwd, env, head, tail, wc, sort, touch, which, true, false,
///   test/[, basename, dirname.  These operate on a sandboxed directory tree.
/// - **WASM module execution**: `.wasm` files run via Wasmtime + WASI with
///   preopened dirs, fuel metering, epoch interruption, and captured I/O.
///
/// Unknown commands return exit code 127.
#[cfg(feature = "wasm")]
pub struct WasmSandbox {
    config: SandboxConfig,
    wasm_engine: Arc<WasmComponentEngine>,
}

#[cfg(feature = "wasm")]
impl WasmSandbox {
    pub fn new(config: SandboxConfig) -> Result<Self> {
        let memory_reservation = config
            .resource_limits
            .memory_limit
            .as_deref()
            .and_then(parse_memory_limit);
        let wasm_engine =
            Arc::new(WasmComponentEngine::new(memory_reservation).context("wasm engine init")?);
        Ok(Self {
            config,
            wasm_engine,
        })
    }

    /// Default fuel limit: 1 billion instructions.
    pub(crate) fn fuel_limit(&self) -> u64 {
        self.config.wasm_fuel_limit.unwrap_or(1_000_000_000)
    }

    /// Default epoch interval: 100ms.
    pub(crate) fn epoch_interval_ms(&self) -> u64 {
        self.config.wasm_epoch_interval_ms.unwrap_or(100)
    }

    /// Root directory for this sandbox instance's isolated filesystem.
    pub(crate) fn sandbox_root(&self, id: &SandboxId) -> PathBuf {
        match self.config.home_persistence {
            HomePersistence::Shared => {
                let base = self.config.shared_home_dir.clone().unwrap_or_else(|| {
                    moltis_config::data_dir()
                        .join("sandbox")
                        .join("home")
                        .join("shared")
                });
                base.join("wasm")
            },
            HomePersistence::Session => moltis_config::data_dir()
                .join("sandbox")
                .join("wasm")
                .join(sanitize_path_component(&id.key)),
            HomePersistence::Off => moltis_config::data_dir()
                .join("sandbox")
                .join("wasm")
                .join(sanitize_path_component(&id.key)),
        }
    }

    /// Guest home directory inside the sandboxed filesystem.
    pub(crate) fn home_dir(&self, id: &SandboxId) -> PathBuf {
        self.sandbox_root(id).join("home")
    }

    /// Guest tmp directory inside the sandboxed filesystem.
    pub(crate) fn tmp_dir(&self, id: &SandboxId) -> PathBuf {
        self.sandbox_root(id).join("tmp")
    }

    /// Execute a `.wasm` module via Wasmtime + WASI with full isolation.
    async fn exec_wasm_module(
        &self,
        wasm_path: &std::path::Path,
        args: &[String],
        id: &SandboxId,
        opts: &ExecOpts,
    ) -> Result<ExecResult> {
        let wasm_engine = Arc::clone(&self.wasm_engine);
        let fuel_limit = self.fuel_limit();
        let epoch_interval_ms = self.epoch_interval_ms();
        let home_dir = self.home_dir(id);
        let tmp_dir = self.tmp_dir(id);
        let wasm_bytes = tokio::fs::read(wasm_path).await?;
        let args = args.to_vec();
        let timeout = opts.timeout;
        let max_output_bytes = opts.max_output_bytes;
        let env_vars: Vec<(String, String)> = opts.env.clone().into_iter().collect();

        let result = tokio::task::spawn_blocking(move || -> Result<ExecResult> {
            use wasmtime_wasi::p2::pipe::MemoryOutputPipe;

            let engine = wasm_engine.engine().clone();
            let stdout_pipe = MemoryOutputPipe::new(max_output_bytes);
            let stderr_pipe = MemoryOutputPipe::new(max_output_bytes);

            let mut wasi_builder = wasmtime_wasi::WasiCtxBuilder::new();
            wasi_builder.stdout(stdout_pipe.clone());
            wasi_builder.stderr(stderr_pipe.clone());
            wasi_builder.args(&args);

            // Minimal safe environment.
            wasi_builder.env("PATH", "/usr/local/bin:/usr/bin:/bin");
            wasi_builder.env("HOME", "/home/sandbox");
            wasi_builder.env("LANG", "C.UTF-8");
            for (k, v) in &env_vars {
                wasi_builder.env(k, v);
            }

            // Preopened directories for filesystem isolation.
            wasi_builder
                .preopened_dir(
                    &home_dir,
                    "/home/sandbox",
                    wasmtime_wasi::DirPerms::all(),
                    wasmtime_wasi::FilePerms::all(),
                )
                .map_err(|e| Error::message(format!("failed to preopen /home/sandbox: {e}")))?;
            wasi_builder
                .preopened_dir(
                    &tmp_dir,
                    "/tmp",
                    wasmtime_wasi::DirPerms::all(),
                    wasmtime_wasi::FilePerms::all(),
                )
                .map_err(|e| Error::message(format!("failed to preopen /tmp: {e}")))?;

            // Build preview1-compatible context for core WASM modules.
            let wasi_p1 = wasi_builder.build_p1();

            let mut store = wasmtime::Store::new(&engine, wasi_p1);
            store.set_fuel(fuel_limit).context("set wasm fuel")?;
            store.set_epoch_deadline(1);

            // Background epoch ticker for timeout enforcement.
            let engine_clone = engine.clone();
            let epoch_handle = std::thread::spawn(move || {
                let interval = std::time::Duration::from_millis(epoch_interval_ms);
                let deadline = std::time::Instant::now() + timeout;
                while std::time::Instant::now() < deadline {
                    std::thread::sleep(interval);
                    engine_clone.increment_epoch();
                }
            });

            let module = wasm_engine
                .compile_module(&wasm_bytes)
                .context("compile wasm module")?;
            let mut linker = wasmtime::Linker::new(&engine);
            wasmtime_wasi::preview1::add_to_linker_sync(&mut linker, |ctx| ctx)
                .context("link wasi preview1")?;

            let instance = linker
                .instantiate(&mut store, &module)
                .context("instantiate wasm module")?;
            let func = instance
                .get_typed_func::<(), ()>(&mut store, "_start")
                .map_err(|e| Error::message(format!("WASM module missing _start: {e}")))?;

            let collect_pipe = |pipe: MemoryOutputPipe| -> String {
                let b: bytes::Bytes = pipe.try_into_inner().unwrap_or_default().into();
                String::from_utf8_lossy(&b).into_owned()
            };

            let exit_code: i32 = match func.call(&mut store, ()) {
                Ok(()) => 0,
                Err(e) => {
                    if let Some(exit) = e.downcast_ref::<wasmtime_wasi::I32Exit>() {
                        exit.0
                    } else {
                        let msg = format!("{e:#}");
                        let mut stdout_str = collect_pipe(stdout_pipe);
                        let mut stderr_str = collect_pipe(stderr_pipe);
                        truncate_output_for_display(&mut stdout_str, max_output_bytes);

                        if msg.contains("fuel") || msg.contains("epoch") {
                            stderr_str.push_str(&format!("\nWASM execution limit exceeded: {msg}"));
                            truncate_output_for_display(&mut stderr_str, max_output_bytes);
                            drop(epoch_handle);
                            return Ok(ExecResult {
                                stdout: stdout_str,
                                stderr: stderr_str,
                                exit_code: 137,
                            });
                        }

                        stderr_str.push_str(&format!("\nWASM error: {msg}"));
                        truncate_output_for_display(&mut stderr_str, max_output_bytes);
                        drop(epoch_handle);
                        return Ok(ExecResult {
                            stdout: stdout_str,
                            stderr: stderr_str,
                            exit_code: 1,
                        });
                    }
                },
            };

            drop(epoch_handle);
            let mut stdout = collect_pipe(stdout_pipe);
            let mut stderr = collect_pipe(stderr_pipe);
            truncate_output_for_display(&mut stdout, max_output_bytes);
            truncate_output_for_display(&mut stderr, max_output_bytes);

            Ok(ExecResult {
                stdout,
                stderr,
                exit_code,
            })
        })
        .await??;

        Ok(result)
    }
}

#[cfg(feature = "wasm")]
#[async_trait]
impl Sandbox for WasmSandbox {
    fn backend_name(&self) -> &'static str {
        "wasm"
    }

    fn is_real(&self) -> bool {
        true
    }

    async fn ensure_ready(&self, id: &SandboxId, _image_override: Option<&str>) -> Result<()> {
        let home = self.home_dir(id);
        let tmp = self.tmp_dir(id);
        tokio::fs::create_dir_all(&home).await?;
        tokio::fs::create_dir_all(&tmp).await?;
        Ok(())
    }

    async fn exec(&self, id: &SandboxId, command: &str, opts: &ExecOpts) -> Result<ExecResult> {
        let sandbox_root = self.sandbox_root(id);
        let env_map: HashMap<String, String> = opts.env.iter().cloned().collect();

        // Parse the command string.
        let segments = WasmBuiltins::parse_command_line(command);

        let mut last_result = ExecResult {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: 0,
        };

        for segment in &segments {
            let should_run = match segment.connector {
                CommandConnector::First | CommandConnector::Sequence => true,
                CommandConnector::And => last_result.exit_code == 0,
                CommandConnector::Or => last_result.exit_code != 0,
            };

            if !should_run {
                continue;
            }

            let expanded = WasmBuiltins::expand_vars(&segment.args, &env_map);
            if expanded.is_empty() {
                continue;
            }

            let empty = String::new();
            let (cmd_name, cmd_args) = expanded.split_first().unwrap_or((&empty, &[]));

            // Check for output redirect.
            let (cmd_args, redirect) = WasmBuiltins::extract_redirect(cmd_args);

            // Check if this is a .wasm file reference.
            if cmd_name.ends_with(".wasm") {
                let wasm_path =
                    WasmBuiltins::resolve_guest_path(&sandbox_root, cmd_name, "/home/sandbox");
                if let Some(wasm_path) = wasm_path {
                    last_result = self
                        .exec_wasm_module(&wasm_path, &cmd_args, id, opts)
                        .await?;
                } else {
                    last_result = ExecResult {
                        stdout: String::new(),
                        stderr: format!("{cmd_name}: path outside sandbox or not found\n"),
                        exit_code: 1,
                    };
                }
            } else {
                // Try built-in commands.
                last_result = WasmBuiltins::execute(cmd_name, &cmd_args, &sandbox_root, &env_map);
            }

            // Handle redirects.
            if let Some(ref redir) = redirect {
                let resolved =
                    WasmBuiltins::resolve_guest_path(&sandbox_root, &redir.target, "/home/sandbox");
                if let Some(host_path) = resolved {
                    if let Some(parent) = host_path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    let write_result = if redir.append {
                        use std::io::Write;
                        std::fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(&host_path)
                            .and_then(|mut f| f.write_all(last_result.stdout.as_bytes()))
                    } else {
                        std::fs::write(&host_path, &last_result.stdout)
                    };
                    if let Err(e) = write_result {
                        last_result.stderr.push_str(&format!("redirect: {e}\n"));
                        last_result.exit_code = 1;
                    } else {
                        last_result.stdout.clear();
                    }
                } else {
                    last_result.stderr.push_str(&format!(
                        "redirect: path outside sandbox: {}\n",
                        redir.target
                    ));
                    last_result.exit_code = 1;
                }
            }
        }

        Ok(last_result)
    }

    async fn cleanup(&self, id: &SandboxId) -> Result<()> {
        if self.config.home_persistence == HomePersistence::Off {
            let root = self.sandbox_root(id);
            if root.exists() {
                tokio::fs::remove_dir_all(&root).await.ok();
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// WASM built-in command interpreter
// ---------------------------------------------------------------------------

#[cfg(feature = "wasm")]
#[derive(Debug)]
enum CommandConnector {
    First,
    Sequence, // ;
    And,      // &&
    Or,       // ||
}

#[cfg(feature = "wasm")]
#[derive(Debug)]
struct CommandSegment {
    connector: CommandConnector,
    args: Vec<String>,
}

#[cfg(feature = "wasm")]
struct OutputRedirect {
    target: String,
    append: bool,
}

#[cfg(feature = "wasm")]
struct WasmBuiltins;

#[cfg(feature = "wasm")]
impl WasmBuiltins {
    /// Parse a command line into segments separated by `&&`, `||`, and `;`.
    fn parse_command_line(input: &str) -> Vec<CommandSegment> {
        let mut segments = Vec::new();
        let mut current = String::new();
        let mut connector = CommandConnector::First;
        let chars: Vec<char> = input.chars().collect();
        let mut i = 0;

        while i < chars.len() {
            if chars[i] == '&' && i + 1 < chars.len() && chars[i + 1] == '&' {
                if !current.trim().is_empty()
                    && let Ok(args) = shell_words::split(current.trim())
                {
                    segments.push(CommandSegment { connector, args });
                }
                connector = CommandConnector::And;
                current.clear();
                i += 2;
                continue;
            }
            if chars[i] == '|' && i + 1 < chars.len() && chars[i + 1] == '|' {
                if !current.trim().is_empty()
                    && let Ok(args) = shell_words::split(current.trim())
                {
                    segments.push(CommandSegment { connector, args });
                }
                connector = CommandConnector::Or;
                current.clear();
                i += 2;
                continue;
            }
            if chars[i] == ';' {
                if !current.trim().is_empty()
                    && let Ok(args) = shell_words::split(current.trim())
                {
                    segments.push(CommandSegment { connector, args });
                }
                connector = CommandConnector::Sequence;
                current.clear();
                i += 1;
                continue;
            }
            current.push(chars[i]);
            i += 1;
        }

        if !current.trim().is_empty()
            && let Ok(args) = shell_words::split(current.trim())
        {
            segments.push(CommandSegment { connector, args });
        }

        segments
    }

    /// Expand `$VAR` references in arguments.
    fn expand_vars(args: &[String], env: &HashMap<String, String>) -> Vec<String> {
        args.iter()
            .map(|arg| {
                let mut result = arg.clone();
                for (key, val) in env {
                    result = result.replace(&format!("${key}"), val);
                    result = result.replace(&format!("${{{key}}}"), val);
                }
                // Expand well-known vars.
                result = result.replace("$HOME", "/home/sandbox");
                result = result.replace("${HOME}", "/home/sandbox");
                result
            })
            .collect()
    }

    /// Extract `>` or `>>` redirect from args, returning remaining args + redirect info.
    fn extract_redirect(args: &[String]) -> (Vec<String>, Option<OutputRedirect>) {
        let mut remaining = Vec::new();
        let mut redirect = None;

        let mut i = 0;
        while i < args.len() {
            if args[i] == ">>" && i + 1 < args.len() {
                redirect = Some(OutputRedirect {
                    target: args[i + 1].clone(),
                    append: true,
                });
                i += 2;
            } else if args[i] == ">" && i + 1 < args.len() {
                redirect = Some(OutputRedirect {
                    target: args[i + 1].clone(),
                    append: false,
                });
                i += 2;
            } else if args[i].starts_with(">>") {
                redirect = Some(OutputRedirect {
                    target: args[i][2..].to_string(),
                    append: true,
                });
                i += 1;
            } else if args[i].starts_with('>') && args[i].len() > 1 {
                redirect = Some(OutputRedirect {
                    target: args[i][1..].to_string(),
                    append: false,
                });
                i += 1;
            } else {
                remaining.push(args[i].clone());
                i += 1;
            }
        }

        (remaining, redirect)
    }

    /// Resolve a guest path to a host path within the sandbox root.
    /// Returns `None` if the path escapes the sandbox.
    fn resolve_guest_path(
        sandbox_root: &std::path::Path,
        guest_path: &str,
        guest_cwd: &str,
    ) -> Option<PathBuf> {
        let logical = if guest_path.starts_with('/') {
            PathBuf::from(guest_path)
        } else {
            PathBuf::from(guest_cwd).join(guest_path)
        };

        // Map guest paths to host sandbox paths.
        let host_path = if let Ok(rest) = logical.strip_prefix("/home/sandbox") {
            sandbox_root.join("home").join(rest)
        } else if let Ok(rest) = logical.strip_prefix("/tmp") {
            sandbox_root.join("tmp").join(rest)
        } else {
            // Path outside known sandbox mounts.
            return None;
        };

        // Canonicalize the parent to check for symlink escapes.
        // The file itself may not exist yet (e.g. for write targets).
        let check_path = if host_path.exists() {
            host_path.canonicalize().ok()?
        } else if let Some(parent) = host_path.parent() {
            if parent.exists() {
                let canonical_parent = parent.canonicalize().ok()?;
                canonical_parent.join(host_path.file_name()?)
            } else {
                host_path.clone()
            }
        } else {
            host_path.clone()
        };

        let canonical_root = if sandbox_root.exists() {
            sandbox_root.canonicalize().ok()?
        } else {
            sandbox_root.to_path_buf()
        };

        if check_path.starts_with(&canonical_root) {
            Some(host_path)
        } else {
            None
        }
    }

    /// Execute a built-in command. Returns exit code 127 for unknown commands.
    fn execute(
        name: &str,
        args: &[String],
        sandbox_root: &std::path::Path,
        env: &HashMap<String, String>,
    ) -> ExecResult {
        match name {
            "echo" => Self::cmd_echo(args),
            "cat" => Self::cmd_cat(args, sandbox_root),
            "ls" => Self::cmd_ls(args, sandbox_root),
            "mkdir" => Self::cmd_mkdir(args, sandbox_root),
            "rm" => Self::cmd_rm(args, sandbox_root),
            "cp" => Self::cmd_cp(args, sandbox_root),
            "mv" => Self::cmd_mv(args, sandbox_root),
            "pwd" => ExecResult {
                stdout: "/home/sandbox\n".into(),
                stderr: String::new(),
                exit_code: 0,
            },
            "env" => Self::cmd_env(env),
            "head" => Self::cmd_head(args, sandbox_root),
            "tail" => Self::cmd_tail(args, sandbox_root),
            "wc" => Self::cmd_wc(args, sandbox_root),
            "sort" => Self::cmd_sort(args, sandbox_root),
            "touch" => Self::cmd_touch(args, sandbox_root),
            "which" => Self::cmd_which(args),
            "true" => ExecResult {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: 0,
            },
            "false" => ExecResult {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: 1,
            },
            "test" | "[" => Self::cmd_test(args, sandbox_root),
            "basename" => Self::cmd_basename(args),
            "dirname" => Self::cmd_dirname(args),
            _ => ExecResult {
                stdout: String::new(),
                stderr: format!("{name}: command not found in WASM sandbox\n"),
                exit_code: 127,
            },
        }
    }

    // --- Built-in command implementations ---

    fn cmd_echo(args: &[String]) -> ExecResult {
        // Handle -n flag.
        let (no_newline, text_args) = if args.first().is_some_and(|a| a == "-n") {
            (true, &args[1..])
        } else {
            (false, args)
        };
        let text = text_args.join(" ");
        let stdout = if no_newline {
            text
        } else {
            format!("{text}\n")
        };
        ExecResult {
            stdout,
            stderr: String::new(),
            exit_code: 0,
        }
    }

    fn cmd_cat(args: &[String], sandbox_root: &std::path::Path) -> ExecResult {
        let mut stdout = String::new();
        let mut stderr = String::new();
        let mut exit_code = 0;

        for arg in args {
            match Self::resolve_guest_path(sandbox_root, arg, "/home/sandbox") {
                Some(path) => match std::fs::read_to_string(&path) {
                    Ok(content) => stdout.push_str(&content),
                    Err(e) => {
                        stderr.push_str(&format!("cat: {arg}: {e}\n"));
                        exit_code = 1;
                    },
                },
                None => {
                    stderr.push_str(&format!("cat: {arg}: path outside sandbox\n"));
                    exit_code = 1;
                },
            }
        }

        ExecResult {
            stdout,
            stderr,
            exit_code,
        }
    }

    fn cmd_ls(args: &[String], sandbox_root: &std::path::Path) -> ExecResult {
        let mut show_long = false;
        let mut show_all = false;
        let mut paths = Vec::new();

        for arg in args {
            if arg.starts_with('-') {
                if arg.contains('l') {
                    show_long = true;
                }
                if arg.contains('a') {
                    show_all = true;
                }
            } else {
                paths.push(arg.as_str());
            }
        }

        if paths.is_empty() {
            paths.push("/home/sandbox");
        }

        let mut stdout = String::new();
        let mut stderr = String::new();
        let mut exit_code = 0;

        for path in &paths {
            match Self::resolve_guest_path(sandbox_root, path, "/home/sandbox") {
                Some(host_path) => {
                    if !host_path.exists() {
                        stderr.push_str(&format!("ls: {path}: No such file or directory\n"));
                        exit_code = 1;
                        continue;
                    }
                    if host_path.is_file() {
                        if let Some(name) = host_path.file_name() {
                            stdout.push_str(&format!("{}\n", name.to_string_lossy()));
                        }
                        continue;
                    }
                    match std::fs::read_dir(&host_path) {
                        Ok(entries) => {
                            let mut names: Vec<String> = entries
                                .filter_map(|e| e.ok())
                                .filter_map(|e| {
                                    let name = e.file_name().to_string_lossy().into_owned();
                                    if !show_all && name.starts_with('.') {
                                        None
                                    } else {
                                        Some(name)
                                    }
                                })
                                .collect();
                            names.sort();
                            if show_long {
                                for name in &names {
                                    let full = host_path.join(name);
                                    let meta = std::fs::metadata(&full);
                                    let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
                                    let kind = if full.is_dir() {
                                        "d"
                                    } else {
                                        "-"
                                    };
                                    stdout.push_str(&format!("{kind}rw-r--r-- {size:>8} {name}\n"));
                                }
                            } else {
                                for name in &names {
                                    stdout.push_str(&format!("{name}\n"));
                                }
                            }
                        },
                        Err(e) => {
                            stderr.push_str(&format!("ls: {path}: {e}\n"));
                            exit_code = 1;
                        },
                    }
                },
                None => {
                    stderr.push_str(&format!("ls: {path}: path outside sandbox\n"));
                    exit_code = 1;
                },
            }
        }

        ExecResult {
            stdout,
            stderr,
            exit_code,
        }
    }

    fn cmd_mkdir(args: &[String], sandbox_root: &std::path::Path) -> ExecResult {
        let mut stderr = String::new();
        let mut exit_code = 0;
        let create_parents = args.iter().any(|a| a == "-p");

        for arg in args.iter().filter(|a| !a.starts_with('-')) {
            match Self::resolve_guest_path(sandbox_root, arg, "/home/sandbox") {
                Some(path) => {
                    let result = if create_parents {
                        std::fs::create_dir_all(&path)
                    } else {
                        std::fs::create_dir(&path)
                    };
                    if let Err(e) = result {
                        stderr.push_str(&format!("mkdir: {arg}: {e}\n"));
                        exit_code = 1;
                    }
                },
                None => {
                    stderr.push_str(&format!("mkdir: {arg}: path outside sandbox\n"));
                    exit_code = 1;
                },
            }
        }

        ExecResult {
            stdout: String::new(),
            stderr,
            exit_code,
        }
    }

    fn cmd_rm(args: &[String], sandbox_root: &std::path::Path) -> ExecResult {
        let mut stderr = String::new();
        let mut exit_code = 0;
        let recursive = args.iter().any(|a| a == "-r" || a == "-rf" || a == "-fr");
        let force = args.iter().any(|a| a.contains('f'));

        for arg in args.iter().filter(|a| !a.starts_with('-')) {
            match Self::resolve_guest_path(sandbox_root, arg, "/home/sandbox") {
                Some(path) => {
                    if !path.exists() {
                        if !force {
                            stderr.push_str(&format!("rm: {arg}: No such file or directory\n"));
                            exit_code = 1;
                        }
                        continue;
                    }
                    let result = if path.is_dir() && recursive {
                        std::fs::remove_dir_all(&path)
                    } else if path.is_dir() {
                        stderr.push_str(&format!("rm: {arg}: is a directory\n"));
                        exit_code = 1;
                        continue;
                    } else {
                        std::fs::remove_file(&path)
                    };
                    if let Err(e) = result {
                        stderr.push_str(&format!("rm: {arg}: {e}\n"));
                        exit_code = 1;
                    }
                },
                None => {
                    stderr.push_str(&format!("rm: {arg}: path outside sandbox\n"));
                    exit_code = 1;
                },
            }
        }

        ExecResult {
            stdout: String::new(),
            stderr,
            exit_code,
        }
    }

    fn cmd_cp(args: &[String], sandbox_root: &std::path::Path) -> ExecResult {
        let non_flag_args: Vec<&String> = args.iter().filter(|a| !a.starts_with('-')).collect();
        if non_flag_args.len() < 2 {
            return ExecResult {
                stdout: String::new(),
                stderr: "cp: missing operand\n".into(),
                exit_code: 1,
            };
        }

        let src_path = non_flag_args[0];
        let dst_path = non_flag_args[1];

        let src = match Self::resolve_guest_path(sandbox_root, src_path, "/home/sandbox") {
            Some(p) => p,
            None => {
                return ExecResult {
                    stdout: String::new(),
                    stderr: format!("cp: {src_path}: path outside sandbox\n"),
                    exit_code: 1,
                };
            },
        };
        let dst = match Self::resolve_guest_path(sandbox_root, dst_path, "/home/sandbox") {
            Some(p) => p,
            None => {
                return ExecResult {
                    stdout: String::new(),
                    stderr: format!("cp: {dst_path}: path outside sandbox\n"),
                    exit_code: 1,
                };
            },
        };

        let actual_dst = if dst.is_dir() {
            if let Some(name) = src.file_name() {
                dst.join(name)
            } else {
                dst
            }
        } else {
            dst
        };

        match std::fs::copy(&src, &actual_dst) {
            Ok(_) => ExecResult {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: 0,
            },
            Err(e) => ExecResult {
                stdout: String::new(),
                stderr: format!("cp: {e}\n"),
                exit_code: 1,
            },
        }
    }

    fn cmd_mv(args: &[String], sandbox_root: &std::path::Path) -> ExecResult {
        let non_flag_args: Vec<&String> = args.iter().filter(|a| !a.starts_with('-')).collect();
        if non_flag_args.len() < 2 {
            return ExecResult {
                stdout: String::new(),
                stderr: "mv: missing operand\n".into(),
                exit_code: 1,
            };
        }

        let src_path = non_flag_args[0];
        let dst_path = non_flag_args[1];

        let src = match Self::resolve_guest_path(sandbox_root, src_path, "/home/sandbox") {
            Some(p) => p,
            None => {
                return ExecResult {
                    stdout: String::new(),
                    stderr: format!("mv: {src_path}: path outside sandbox\n"),
                    exit_code: 1,
                };
            },
        };
        let dst = match Self::resolve_guest_path(sandbox_root, dst_path, "/home/sandbox") {
            Some(p) => p,
            None => {
                return ExecResult {
                    stdout: String::new(),
                    stderr: format!("mv: {dst_path}: path outside sandbox\n"),
                    exit_code: 1,
                };
            },
        };

        let actual_dst = if dst.is_dir() {
            if let Some(name) = src.file_name() {
                dst.join(name)
            } else {
                dst
            }
        } else {
            dst
        };

        match std::fs::rename(&src, &actual_dst) {
            Ok(()) => ExecResult {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: 0,
            },
            Err(e) => ExecResult {
                stdout: String::new(),
                stderr: format!("mv: {e}\n"),
                exit_code: 1,
            },
        }
    }

    fn cmd_env(env: &HashMap<String, String>) -> ExecResult {
        let mut stdout = String::new();
        stdout.push_str("PATH=/usr/local/bin:/usr/bin:/bin\n");
        stdout.push_str("HOME=/home/sandbox\n");
        stdout.push_str("LANG=C.UTF-8\n");
        let mut keys: Vec<&String> = env.keys().collect();
        keys.sort();
        for key in keys {
            if let Some(val) = env.get(key) {
                stdout.push_str(&format!("{key}={val}\n"));
            }
        }
        ExecResult {
            stdout,
            stderr: String::new(),
            exit_code: 0,
        }
    }

    fn cmd_head(args: &[String], sandbox_root: &std::path::Path) -> ExecResult {
        let mut lines = 10usize;
        let mut files = Vec::new();

        let mut i = 0;
        while i < args.len() {
            if args[i] == "-n" && i + 1 < args.len() {
                lines = args[i + 1].parse().unwrap_or(10);
                i += 2;
            } else if args[i].starts_with('-') && args[i][1..].parse::<usize>().is_ok() {
                lines = args[i][1..].parse().unwrap_or(10);
                i += 1;
            } else {
                files.push(&args[i]);
                i += 1;
            }
        }

        let mut stdout = String::new();
        let mut stderr = String::new();
        let mut exit_code = 0;

        for file in &files {
            match Self::resolve_guest_path(sandbox_root, file, "/home/sandbox") {
                Some(path) => match std::fs::read_to_string(&path) {
                    Ok(content) => {
                        for line in content.lines().take(lines) {
                            stdout.push_str(line);
                            stdout.push('\n');
                        }
                    },
                    Err(e) => {
                        stderr.push_str(&format!("head: {file}: {e}\n"));
                        exit_code = 1;
                    },
                },
                None => {
                    stderr.push_str(&format!("head: {file}: path outside sandbox\n"));
                    exit_code = 1;
                },
            }
        }

        ExecResult {
            stdout,
            stderr,
            exit_code,
        }
    }

    fn cmd_tail(args: &[String], sandbox_root: &std::path::Path) -> ExecResult {
        let mut lines = 10usize;
        let mut files = Vec::new();

        let mut i = 0;
        while i < args.len() {
            if args[i] == "-n" && i + 1 < args.len() {
                lines = args[i + 1].parse().unwrap_or(10);
                i += 2;
            } else if args[i].starts_with('-') && args[i][1..].parse::<usize>().is_ok() {
                lines = args[i][1..].parse().unwrap_or(10);
                i += 1;
            } else {
                files.push(&args[i]);
                i += 1;
            }
        }

        let mut stdout = String::new();
        let mut stderr = String::new();
        let mut exit_code = 0;

        for file in &files {
            match Self::resolve_guest_path(sandbox_root, file, "/home/sandbox") {
                Some(path) => match std::fs::read_to_string(&path) {
                    Ok(content) => {
                        let all_lines: Vec<&str> = content.lines().collect();
                        let start = all_lines.len().saturating_sub(lines);
                        for line in &all_lines[start..] {
                            stdout.push_str(line);
                            stdout.push('\n');
                        }
                    },
                    Err(e) => {
                        stderr.push_str(&format!("tail: {file}: {e}\n"));
                        exit_code = 1;
                    },
                },
                None => {
                    stderr.push_str(&format!("tail: {file}: path outside sandbox\n"));
                    exit_code = 1;
                },
            }
        }

        ExecResult {
            stdout,
            stderr,
            exit_code,
        }
    }

    fn cmd_wc(args: &[String], sandbox_root: &std::path::Path) -> ExecResult {
        let files: Vec<&String> = args.iter().filter(|a| !a.starts_with('-')).collect();
        let mut stdout = String::new();
        let mut stderr = String::new();
        let mut exit_code = 0;

        for file in &files {
            match Self::resolve_guest_path(sandbox_root, file, "/home/sandbox") {
                Some(path) => match std::fs::read_to_string(&path) {
                    Ok(content) => {
                        let line_count = content.lines().count();
                        let word_count = content.split_whitespace().count();
                        let byte_count = content.len();
                        stdout.push_str(&format!(
                            "{line_count:>8} {word_count:>8} {byte_count:>8} {file}\n"
                        ));
                    },
                    Err(e) => {
                        stderr.push_str(&format!("wc: {file}: {e}\n"));
                        exit_code = 1;
                    },
                },
                None => {
                    stderr.push_str(&format!("wc: {file}: path outside sandbox\n"));
                    exit_code = 1;
                },
            }
        }

        ExecResult {
            stdout,
            stderr,
            exit_code,
        }
    }

    fn cmd_sort(args: &[String], sandbox_root: &std::path::Path) -> ExecResult {
        let files: Vec<&String> = args.iter().filter(|a| !a.starts_with('-')).collect();
        let reverse = args.iter().any(|a| a == "-r");
        let mut all_lines = Vec::new();
        let mut stderr = String::new();
        let mut exit_code = 0;

        for file in &files {
            match Self::resolve_guest_path(sandbox_root, file, "/home/sandbox") {
                Some(path) => match std::fs::read_to_string(&path) {
                    Ok(content) => {
                        all_lines.extend(content.lines().map(ToOwned::to_owned));
                    },
                    Err(e) => {
                        stderr.push_str(&format!("sort: {file}: {e}\n"));
                        exit_code = 1;
                    },
                },
                None => {
                    stderr.push_str(&format!("sort: {file}: path outside sandbox\n"));
                    exit_code = 1;
                },
            }
        }

        all_lines.sort();
        if reverse {
            all_lines.reverse();
        }

        let mut stdout = String::new();
        for line in &all_lines {
            stdout.push_str(line);
            stdout.push('\n');
        }

        ExecResult {
            stdout,
            stderr,
            exit_code,
        }
    }

    fn cmd_touch(args: &[String], sandbox_root: &std::path::Path) -> ExecResult {
        let mut stderr = String::new();
        let mut exit_code = 0;

        for arg in args.iter().filter(|a| !a.starts_with('-')) {
            match Self::resolve_guest_path(sandbox_root, arg, "/home/sandbox") {
                Some(path) => {
                    if !path.exists() {
                        if let Some(parent) = path.parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        if let Err(e) = std::fs::write(&path, "") {
                            stderr.push_str(&format!("touch: {arg}: {e}\n"));
                            exit_code = 1;
                        }
                    }
                    // If file exists, we'd update mtime but that's not critical.
                },
                None => {
                    stderr.push_str(&format!("touch: {arg}: path outside sandbox\n"));
                    exit_code = 1;
                },
            }
        }

        ExecResult {
            stdout: String::new(),
            stderr,
            exit_code,
        }
    }

    fn cmd_which(args: &[String]) -> ExecResult {
        let builtins = [
            "echo", "cat", "ls", "mkdir", "rm", "cp", "mv", "pwd", "env", "head", "tail", "wc",
            "sort", "touch", "which", "true", "false", "test", "[", "basename", "dirname",
        ];
        let mut stdout = String::new();
        let mut stderr = String::new();
        let mut exit_code = 0;

        for arg in args {
            if builtins.contains(&arg.as_str()) {
                stdout.push_str(&format!("{arg}: WASM sandbox built-in\n"));
            } else {
                stderr.push_str(&format!("{arg} not found\n"));
                exit_code = 1;
            }
        }

        ExecResult {
            stdout,
            stderr,
            exit_code,
        }
    }

    fn cmd_test(args: &[String], sandbox_root: &std::path::Path) -> ExecResult {
        // Strip trailing ] if present (for [ ... ] syntax).
        let args: Vec<&String> = if args.last().is_some_and(|a| a == "]") {
            args[..args.len() - 1].iter().collect()
        } else {
            args.iter().collect()
        };

        let result = match args.len() {
            0 => false,
            1 => !args[0].is_empty(),
            2 => {
                let op = args[0].as_str();
                let operand = args[1].as_str();
                match op {
                    "-f" => Self::resolve_guest_path(sandbox_root, operand, "/home/sandbox")
                        .is_some_and(|p| p.is_file()),
                    "-d" => Self::resolve_guest_path(sandbox_root, operand, "/home/sandbox")
                        .is_some_and(|p| p.is_dir()),
                    "-e" => Self::resolve_guest_path(sandbox_root, operand, "/home/sandbox")
                        .is_some_and(|p| p.exists()),
                    "-z" => operand.is_empty(),
                    "-n" => !operand.is_empty(),
                    _ => false,
                }
            },
            3 => {
                let left = args[0].as_str();
                let op = args[1].as_str();
                let right = args[2].as_str();
                match op {
                    "=" | "==" => left == right,
                    "!=" => left != right,
                    "-eq" => left.parse::<i64>().ok() == right.parse::<i64>().ok(),
                    "-ne" => left.parse::<i64>().ok() != right.parse::<i64>().ok(),
                    "-lt" => left
                        .parse::<i64>()
                        .ok()
                        .zip(right.parse::<i64>().ok())
                        .is_some_and(|(l, r)| l < r),
                    "-gt" => left
                        .parse::<i64>()
                        .ok()
                        .zip(right.parse::<i64>().ok())
                        .is_some_and(|(l, r)| l > r),
                    _ => false,
                }
            },
            _ => false,
        };

        ExecResult {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: if result {
                0
            } else {
                1
            },
        }
    }

    fn cmd_basename(args: &[String]) -> ExecResult {
        if args.is_empty() {
            return ExecResult {
                stdout: String::new(),
                stderr: "basename: missing operand\n".into(),
                exit_code: 1,
            };
        }
        let path = std::path::Path::new(&args[0]);
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        ExecResult {
            stdout: format!("{name}\n"),
            stderr: String::new(),
            exit_code: 0,
        }
    }

    fn cmd_dirname(args: &[String]) -> ExecResult {
        if args.is_empty() {
            return ExecResult {
                stdout: String::new(),
                stderr: "dirname: missing operand\n".into(),
                exit_code: 1,
            };
        }
        let path = std::path::Path::new(&args[0]);
        let parent = path
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| ".".into());
        ExecResult {
            stdout: format!("{parent}\n"),
            stderr: String::new(),
            exit_code: 0,
        }
    }
}
