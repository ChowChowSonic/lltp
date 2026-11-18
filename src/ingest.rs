//! Textual `.ll` is the primary format; bitcode `.bc` is also accepted.

use crate::errors::IngestError;
use inkwell::context::Context;
use inkwell::memory_buffer::MemoryBuffer;
use inkwell::module::Module as IrModule;
use std::fmt;
use std::fs::File;
use std::io::Read;
use std::path::Path;

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

/// Parse a buffer into a module. `origin` names where the bytes came from, so a
/// parse failure reports its source rather than a fixed string, and `format`
/// decides which parser runs. Textual IR needs the NUL terminator
/// `create_module_from_ir` reads.
pub fn parse<'ctx>(
    ctxt: &'ctx Context,
    buf: MemoryBuffer,
    origin: String,
    format: IrFormat,
) -> Result<IrModule<'ctx>, IngestError> {
    let parsed = match format {
        IrFormat::Text => ctxt.create_module_from_ir(buf),
        IrFormat::Bitcode => IrModule::parse_bitcode_from_buffer(&buf, ctxt),
    };
    parsed.map_err(|e| IngestError::Parse {
        origin,
        format,
        detail: e.to_string(),
    })
}

/// Parse textual IR held in memory. This is the single parse path the library
/// uses, so `build_module` and `load_module` report the same error shape.
pub fn parse_text<'ctx>(
    ctxt: &'ctx Context,
    ir: &str,
    origin: impl Into<String>,
) -> Result<IrModule<'ctx>, IngestError> {
    let origin = origin.into();
    let mut bytes = ir.as_bytes().to_vec();
    if bytes.last() != Some(&0) {
        bytes.push(0);
    }
    let buf = MemoryBuffer::create_from_memory_range(&bytes, &origin);
    parse(ctxt, buf, origin, IrFormat::Text)
}

/// Load a `.ll` or `.bc` file into a module.
pub fn load_module<'ctx>(ctxt: &'ctx Context, path: &Path) -> Result<IrModule<'ctx>, IngestError> {
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
    parse(ctxt, buf, path.display().to_string(), format)
}

#[cfg(test)]
mod tests {
    use crate::build_module;

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
        let res = build_module(&ctxt, "define void @f() {\nentry:\n  ret void\n}\n");
        assert!(res.is_ok());
    }

    #[test]
    fn parse_ir_rejects_garbage_without_panicking() {
        let ctxt = Context::create();
        let res = build_module(&ctxt, "this is not llvm ir");
        assert!(matches!(
            res,
            Err(IngestError::Parse {
                format: IrFormat::Text,
                ..
            })
        ));
    }
}
