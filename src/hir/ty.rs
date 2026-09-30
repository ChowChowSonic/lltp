use inkwell::{
    types::{AnyTypeEnum, BasicType, BasicTypeEnum},
    values::BasicValueEnum,
};
use tracing::warn;
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ty {
    Void,
    Bool,
    Int(usize /*# of bits*/, bool /*is_signed*/),
    Float(usize /*bits*/),
    Ptr(Box<Ty>),
    Array(Box<Ty>, usize),
    Opaque(String),
}
impl<'c> From<BasicTypeEnum<'c>> for Ty {
    fn from(ty: BasicTypeEnum) -> Self {
        match ty {
            BasicTypeEnum::IntType(i) => {
                let bits = i.get_bit_width() as usize;
                if bits == 1 {
                    Ty::Bool
                } else {
                    Ty::Int(bits, true)
                }
            }
            BasicTypeEnum::FloatType(f) => Ty::Float(f.get_bit_width() as usize),
            BasicTypeEnum::PointerType(_p) => {
                Ty::Ptr(Box::new(Ty::Opaque("unresolved pointee".to_string())))
            }
            BasicTypeEnum::ArrayType(a) => {
                Ty::Array(Box::new(Ty::from(a.get_element_type())), a.len() as usize)
            }
            BasicTypeEnum::VectorType(v) => {
                let size_res = match v.get_element_type().size_of() {
                    Some(x) => {
                        if let Some(y) = x.get_zero_extended_constant() {
                            y
                        } else {
                            unreachable!("Error unpacking type size")
                        }
                    }
                    None => unreachable!("Error unpacking type size"),
                };
                let array_size = v.get_size();

                Ty::Array(
                    Box::new(Ty::from(v.get_element_type())),
                    (array_size as u64 / size_res as u64) as usize,
                )
            }
            BasicTypeEnum::StructType(_) => {
                warn!("unsupported type: struct coerced to `Ty::Void`");
                Ty::Opaque("Unsupported type".to_string())
            }
            BasicTypeEnum::ScalableVectorType(_) => {
                warn!("unsupported type: scalable vector coerced to `Ty::Void`");
                Ty::Opaque("Unsupported type".to_string())
            }
        }
    }
}

impl<'c> From<BasicValueEnum<'c>> for Ty {
    fn from(ty: BasicValueEnum) -> Self {
        Ty::from(ty.get_type())
    }
}
impl<'c> TryFrom<AnyTypeEnum<'c>> for Ty {
    type Error = ();
    fn try_from(any: AnyTypeEnum<'c>) -> Result<Self, ()> {
        match any {
            AnyTypeEnum::IntType(t) => Ok(Ty::from(BasicTypeEnum::IntType(t))),
            AnyTypeEnum::FloatType(t) => Ok(Ty::from(BasicTypeEnum::FloatType(t))),
            AnyTypeEnum::PointerType(t) => Ok(Ty::from(BasicTypeEnum::PointerType(t))),
            AnyTypeEnum::ArrayType(t) => Ok(Ty::from(BasicTypeEnum::ArrayType(t))),
            AnyTypeEnum::VectorType(t) => Ok(Ty::from(BasicTypeEnum::VectorType(t))),
            AnyTypeEnum::StructType(t) => Ok(Ty::from(BasicTypeEnum::StructType(t))),
            AnyTypeEnum::ScalableVectorType(t) => {
                Ok(Ty::from(BasicTypeEnum::ScalableVectorType(t)))
            }
            // Void/Function: no value-producing SSA result to bind via `Let`.
            AnyTypeEnum::FunctionType(_) | AnyTypeEnum::VoidType(_) => Err(()),
        }
    }
}
