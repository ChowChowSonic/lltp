//! Frontend driver: run the source language's native toolchain and return unoptimized LLVM IR as text.

use std::error::Error;
use std::fmt;
use std::io;
use std::path::Path;
use std::process::Command;

/// How to invoke the native compiler.
#[derive(Debug, Clone)]
pub struct FrontendConfig {
    /// Compiler executable. Defaults to `$LLTP_CLANG`, else `clang`.
    /// Point this at the clang that matches your pinned LLVM (e.g. `clang-22`).
    pub clang: String,
    /// Pass `-g` so the IR carries debug info (source variable names, lines).
    pub debug_info: bool,
}

impl Default for FrontendConfig {
    fn default() -> Self {
        Self {
            clang: std::env::var("LLTP_CLANG").unwrap_or_else(|_| "clang".to_string()),
            debug_info: true,
        }
    }
}

/// A successful compile: the IR plus anything clang printed to stderr
#[derive(Debug, Clone)]
pub struct Compiled {
    pub ir: String,
    pub diagnostics: String,
}

#[derive(Debug)]
pub enum FrontendError {
    /// The compiler executable could not be started.
    Spawn {
        clang: String,
        source: io::Error,
    },
    /// clang ran but exited non-zero (e.g. a C syntax error).
    Compile {
        stderr: String,
    },
    NotUtf8,
}

impl fmt::Display for FrontendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn { clang, source } => write!(
                f,
                "could not run `{clang}` ({source}); is it installed? set LLTP_CLANG to override"
            ),
            Self::Compile { stderr } => write!(f, "clang failed:\n{stderr}"),
            Self::NotUtf8 => write!(f, "clang produced non-UTF-8 output"),
        }
    }
}

impl Error for FrontendError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Spawn { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Compile a C file to unoptimized textual LLVM IR.
/// `-fno-discard-value-names` keeps names like `%alpha` / `%add` in the IR
pub fn compile_c_to_ir(cfg: &FrontendConfig, src: &Path) -> Result<Compiled, FrontendError> {
    let mut cmd = Command::new(&cfg.clang);
    cmd.args([
        "-S",
        "-emit-llvm",
        "-O0",
        "-fno-discard-value-names",
        "-x",
        "c",
    ]);
    if cfg.debug_info {
        cmd.arg("-g");
    }
    cmd.arg(src).args(["-o", "-"]);

    let out = cmd.output().map_err(|source| FrontendError::Spawn {
        clang: cfg.clang.clone(),
        source,
    })?;

    let diagnostics = String::from_utf8_lossy(&out.stderr).into_owned();
    if !out.status.success() {
        return Err(FrontendError::Compile {
            stderr: diagnostics,
        });
    }
    let ir = String::from_utf8(out.stdout).map_err(|_| FrontendError::NotUtf8)?;
    Ok(Compiled { ir, diagnostics })
}

/// First line of `<clang> --version`, if the compiler can be run. Handy for
/// spotting a clang/LLVM version mismatch.
pub fn clang_version(clang: &str) -> Option<String> {
    let out = Command::new(clang).arg("--version").output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.lines().next().map(str::to_owned)
}
