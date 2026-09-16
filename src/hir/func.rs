use super::{Ty, stmt::Stmt};

#[derive(Debug, Clone)]
pub struct Function {
    pub name: String,
    pub params: Vec<Ty>,
    pub return_ty: Ty,
    pub body: Vec<Stmt>,
}
