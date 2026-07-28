use crate::lexer::TokenKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub line: usize,
    pub column: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Literal {
    Int(i64),
    Float(f64),
    String(String),
    Bool(bool),
    Null,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TypeRef {
    Named(String, Span),
    Applied(String, Vec<TypeRef>, Span),
    Array(Box<TypeRef>, Span),
    Nullable(Box<TypeRef>, Span),
    Result(Box<TypeRef>, Box<TypeRef>, Span),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    pub name: String,
    pub ty: Option<TypeRef>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TypeParam {
    pub name: String,
    pub constraint: Option<String>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FieldDef {
    pub name: String,
    pub ty: TypeRef,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VariantDef {
    pub name: String,
    pub payload: Option<TypeRef>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProtocolMember {
    pub name: String,
    pub params: Vec<Param>,
    pub return_type: TypeRef,
    pub needs: Vec<String>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AdoptionMember {
    pub member: String,
    pub implementation: String,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MatchArm {
    pub variant: String,
    pub binding: Option<String>,
    pub value: Expr,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Literal(Literal, Span),
    Variable(String, Span),
    Assign(String, Box<Expr>, Span),
    Unary(TokenKind, Box<Expr>, Span),
    Binary(Box<Expr>, TokenKind, Box<Expr>, Span),
    Logical(Box<Expr>, TokenKind, Box<Expr>, Span),
    Call(Box<Expr>, Vec<Expr>, Span),
    Array(Vec<Expr>, Span),
    Index(Box<Expr>, Box<Expr>, Span),
    Coalesce(Box<Expr>, Box<Expr>, Span),
    Propagate(Box<Expr>, Span),
    Get(Box<Expr>, String, Span),
    Match(Box<Expr>, Vec<MatchArm>, Span),
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Self::Literal(_, span)
            | Self::Variable(_, span)
            | Self::Assign(_, _, span)
            | Self::Unary(_, _, span)
            | Self::Binary(_, _, _, span)
            | Self::Logical(_, _, _, span)
            | Self::Call(_, _, span)
            | Self::Array(_, span)
            | Self::Index(_, _, span)
            | Self::Coalesce(_, _, span)
            | Self::Propagate(_, span) => *span,
            Self::Get(_, _, span) | Self::Match(_, _, span) => *span,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Stmt {
    Let {
        name: String,
        mutable: bool,
        annotation: Option<TypeRef>,
        initializer: Expr,
        span: Span,
    },
    Expression(Expr),
    Print(Expr, Span),
    Block(Vec<Stmt>, Span),
    If {
        condition: Expr,
        then_branch: Box<Stmt>,
        else_branch: Option<Box<Stmt>>,
        span: Span,
    },
    While {
        condition: Expr,
        body: Box<Stmt>,
        span: Span,
    },
    For {
        name: String,
        iterable: Expr,
        body: Box<Stmt>,
        span: Span,
    },
    Using {
        name: String,
        resource: Expr,
        body: Box<Stmt>,
        span: Span,
    },
    Function {
        name: String,
        type_params: Vec<TypeParam>,
        params: Vec<Param>,
        return_type: Option<TypeRef>,
        needs: Vec<String>,
        body: Vec<Stmt>,
        span: Span,
    },
    Return(Option<Expr>, Span),
    Record {
        name: String,
        type_params: Vec<TypeParam>,
        fields: Vec<FieldDef>,
        derives: Vec<String>,
        span: Span,
    },
    Enum {
        name: String,
        type_params: Vec<TypeParam>,
        variants: Vec<VariantDef>,
        span: Span,
    },
    Protocol {
        name: String,
        members: Vec<ProtocolMember>,
        span: Span,
    },
    Adoption {
        protocol: String,
        ty: TypeRef,
        members: Vec<AdoptionMember>,
        span: Span,
    },
    Import {
        path: String,
        span: Span,
    },
    Export {
        names: Vec<String>,
        span: Span,
    },
    Module {
        name: String,
        body: Vec<Stmt>,
        exports: Vec<String>,
        span: Span,
    },
}
