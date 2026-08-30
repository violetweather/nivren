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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapabilityNeed {
    pub capability: String,
    pub boundary: Option<String>,
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

/// One piece of a `text "…"` literal: fixed text or one hole expression.
#[derive(Clone, Debug, PartialEq)]
pub enum TextPiece {
    Literal(String),
    Hole(Expr),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Literal(Literal, Span),
    /// A formatted `text "…"` literal; evaluation renders every piece and
    /// joins them into one string.
    Text(Vec<TextPiece>, Span),
    Variable(String, Span),
    Assign(String, Box<Expr>, Span),
    Unary(TokenKind, Box<Expr>, Span),
    Binary(Box<Expr>, TokenKind, Box<Expr>, Span),
    Logical(Box<Expr>, TokenKind, Box<Expr>, Span),
    Call(Box<Expr>, Vec<Expr>, Option<Vec<String>>, Span),
    Array(Vec<Expr>, Span),
    Index(Box<Expr>, Box<Expr>, Span),
    Coalesce(Box<Expr>, Box<Expr>, Span),
    Propagate(Box<Expr>, Span),
    /// An explicit external-effect boundary. The wrapped expression is kept in
    /// the tree so intent analysis can prove ordering and authority before the
    /// bytecode compiler lowers it without allocating a runtime plan.
    Perform(Box<Expr>, Span),
    /// A source pipeline. Keeping the stage separate lets intent analysis fuse
    /// pure stages while effectful stages retain source order.
    Through(Box<Expr>, Box<Expr>, Span),
    Get(Box<Expr>, String, Span),
    Match(Box<Expr>, Vec<MatchArm>, Span),
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Self::Literal(_, span)
            | Self::Text(_, span)
            | Self::Variable(_, span)
            | Self::Assign(_, _, span)
            | Self::Unary(_, _, span)
            | Self::Binary(_, _, _, span)
            | Self::Logical(_, _, _, span)
            | Self::Call(_, _, _, span)
            | Self::Array(_, span)
            | Self::Index(_, _, span)
            | Self::Coalesce(_, _, span)
            | Self::Propagate(_, span)
            | Self::Perform(_, span)
            | Self::Through(_, _, span) => *span,
            Self::Get(_, _, span) | Self::Match(_, _, span) => *span,
        }
    }
}

/// Produces the ordinary call represented by a `through` expression. Keeping
/// this lowering in one place guarantees the checker, tree interpreter, and
/// bytecode compiler observe identical argument ordering.
#[must_use]
pub fn lower_through(input: &Expr, stage: &Expr, span: Span) -> Expr {
    match stage {
        Expr::Call(callee, arguments, labels, _) => {
            let mut arguments = arguments.clone();
            arguments.insert(0, input.clone());
            let labels = labels.clone().map(|mut labels| {
                if let Some(path) = expression_path(callee)
                    && let Some(expected) = crate::call_labels::get(&path)
                    && expected.len() == labels.len() + 1
                {
                    labels.insert(0, expected[0].clone());
                }
                labels
            });
            Expr::Call(callee.clone(), arguments, labels, span)
        }
        Expr::Variable(_, _) | Expr::Get(_, _, _) => {
            Expr::Call(Box::new(stage.clone()), vec![input.clone()], None, span)
        }
        _ => unreachable!("the parser only constructs callable through stages"),
    }
}

fn expression_path(expression: &Expr) -> Option<String> {
    match expression {
        Expr::Variable(name, _) => Some(name.clone()),
        Expr::Get(parent, name, _) => Some(format!("{}.{}", expression_path(parent)?, name)),
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Stmt {
    /// An immutable, explicitly stored plan. `initializer` remains the typed
    /// value representation used by Edition 3's runtime; the intent layer owns
    /// the plan metadata and serialization decision.
    Prepare {
        name: String,
        plan_type: String,
        initializer: Expr,
        span: Span,
    },
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
    /// `when subject carries name { … } otherwise { … }`: tests a `maybe`
    /// value once, binding the present value immutably in the matched branch.
    IfCarries {
        subject: Expr,
        binding: String,
        then_branch: Box<Stmt>,
        else_branch: Option<Box<Stmt>>,
        span: Span,
    },
    While {
        condition: Expr,
        body: Box<Stmt>,
        span: Span,
    },
    /// `stop` ends the nearest enclosing loop; `skip` ends only the current
    /// pass. The checker guarantees both appear inside a loop without crossing
    /// a function, task, or `using` boundary.
    Stop(Span),
    Skip(Span),
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
        capability_needs: Vec<CapabilityNeed>,
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
