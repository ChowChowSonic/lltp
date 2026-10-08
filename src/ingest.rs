//! Textual `.ll` is the primary format; bitcode `.bc` is also accepted.

use std::error::Error;
use std::fmt;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use inkwell::context::Context;
use inkwell::memory_buffer::MemoryBuffer;
use inkwell::module::Module;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IrFormat {
    /// Textual IR (`.ll`).
    Text,
    /// Bitcode (`.bc`), raw or in the wrapper container.
    Bitcode,
}

impl fmt::Display for IrFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Text => write!(f, "LLVM IR text"),
            Self::Bitcode => write!(f, "LLVM bitcode"),
        }
    }
}

#[derive(Debug)]
pub enum IngestError {
    /// The file could not be opened or read, or was empty.
    Read { path: PathBuf, detail: String },
    /// LLVM rejected the contents (syntax error, truncated bitcode, ...).
    Parse {
        origin: String,
        format: IrFormat,
        detail: String,
    },
}

impl fmt::Display for IngestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, detail } => write!(f, "cannot read `{}`: {detail}", path.display()),
            Self::Parse {
                origin,
                format,
                detail,
            } => write!(f, "cannot parse {origin} as {format}: {detail}"),
        }
    }
}

impl Error for IngestError {}

/// Guess the format from a file's first bytes: raw bitcode starts with `BC C0 DE`,
/// the wrapper container with `DE C0 17 0B`. Anything else is treated as text.
pub fn sniff_format(header: &[u8]) -> IrFormat {
    const RAW_MAGIC: [u8; 4] = [0x42, 0x43, 0xC0, 0xDE];
    const WRAPPER_MAGIC: [u8; 4] = [0xDE, 0xC0, 0x17, 0x0B];
    if header.starts_with(&RAW_MAGIC) || header.starts_with(&WRAPPER_MAGIC) {
        IrFormat::Bitcode
    } else {
        IrFormat::Text
    }
}

/// Parse textual IR held in memory.
pub fn parse_ir<'ctx>(ctxt: &'ctx Context, ir: &str) -> Result<Module<'ctx>, IngestError> {
    let mut bytes = ir.as_bytes().to_vec();
    if bytes.last() != Some(&0) {
        bytes.push(0);
    }
    let mem_buf = MemoryBuffer::create_from_memory_range(&bytes, "test_module");
    ctxt.create_module_from_ir(mem_buf)
        .map_err(|e| IngestError::Parse {
            origin: "<memory>".to_string(),
            format: IrFormat::Text,
            detail: e.to_string(),
        })
}

/// Load a `.ll` or `.bc` file into a module.
pub fn load_module<'ctx>(ctxt: &'ctx Context, path: &Path) -> Result<Module<'ctx>, IngestError> {
    let read_err = |detail: String| IngestError::Read {
        path: path.to_path_buf(),
        detail,
    };

    let mut head = Vec::with_capacity(4);
    File::open(path)
        .and_then(|f| f.take(4).read_to_end(&mut head))
        .map_err(|e| read_err(e.to_string()))?;
    if head.is_empty() {
        return Err(read_err("file is empty".to_string()));
    }
    let format = sniff_format(&head);

    let buf = MemoryBuffer::create_from_file(path).map_err(|e| read_err(e.to_string()))?;
    let parsed = match format {
        IrFormat::Text => ctxt.create_module_from_ir(buf),
        IrFormat::Bitcode => Module::parse_bitcode_from_buffer(&buf, ctxt),
    };
    parsed.map_err(|e| IngestError::Parse {
        origin: path.display().to_string(),
        format,
        detail: e.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniff_detects_bitcode_magics() {
        assert_eq!(
            sniff_format(&[0x42, 0x43, 0xC0, 0xDE, 0x35]),
            IrFormat::Bitcode
        );
        assert_eq!(
            sniff_format(&[0xDE, 0xC0, 0x17, 0x0B, 0x00]),
            IrFormat::Bitcode
        );
    }

    #[test]
    fn sniff_defaults_to_text() {
        assert_eq!(sniff_format(b"; ModuleID = 'x'"), IrFormat::Text);
        assert_eq!(sniff_format(b"BC"), IrFormat::Text);
        assert_eq!(sniff_format(&[]), IrFormat::Text);
    }

    #[test]
    fn parse_ir_accepts_valid_text() {
        let ctxt = Context::create();
        let res = parse_ir(&ctxt, "define void @f() {\nentry:\n  ret void\n}\n");
        assert!(res.is_ok());
    }

    #[test]
    fn parse_ir_rejects_garbage_without_panicking() {
        let ctxt = Context::create();
        let res = parse_ir(&ctxt, "this is not llvm ir");
        assert!(matches!(
            res,
            Err(IngestError::Parse {
                format: IrFormat::Text,
                ..
            })
        ));
    }
}
