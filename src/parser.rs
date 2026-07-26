use crate::ast::{Expr, FieldDef, Literal, MatchArm, Param, Span, Stmt, TypeRef};
use crate::error::NivError;
use crate::lexer::{Token, TokenKind};

pub fn parse(tokens: Vec<Token>) -> Result<Vec<Stmt>, Vec<NivError>> {
    Parser {
        tokens,
        current: 0,
        errors: vec![],
    }
    .program()
}

struct Parser {
    tokens: Vec<Token>,
    current: usize,
    errors: Vec<NivError>,
}

impl Parser {
    fn program(mut self) -> Result<Vec<Stmt>, Vec<NivError>> {
        let mut statements = vec![];
        while !self.is_at_end() {
            match self.declaration() {
                Ok(statement) => statements.push(statement),
                Err(error) => {
                    self.errors.push(error);
                    self.synchronize();
                }
            }
        }
        if self.errors.is_empty() {
            Ok(statements)
        } else {
            Err(self.errors)
        }
    }

    fn declaration(&mut self) -> Result<Stmt, NivError> {
        if self.matches(&[TokenKind::Let]) {
            return self.binding(false);
        }
        if self.matches(&[TokenKind::Var]) {
            return self.binding(true);
        }
        if self.matches(&[TokenKind::Fun]) {
            return self.function();
        }
        if self.matches(&[TokenKind::Record]) {
            return self.record();
        }
        if self.matches(&[TokenKind::Enum]) {
            return self.enum_declaration();
        }
        if self.matches(&[TokenKind::Import]) {
            return self.import_declaration();
        }
        if self.matches(&[TokenKind::Export]) {
            return self.export_declaration();
        }
        self.statement()
    }

    fn binding(&mut self, mutable: bool) -> Result<Stmt, NivError> {
        let span = self.previous_span();
        let name = self.consume_identifier("expected a name after binding keyword")?;
        let annotation = if self.matches(&[TokenKind::Colon]) {
            Some(self.type_ref()?)
        } else {
            None
        };
        self.consume(&TokenKind::Equal, "expected '=' after binding name")?;
        let initializer = self.expression()?;
        self.optional_semicolon();
        Ok(Stmt::Let {
            name,
            mutable,
            annotation,
            initializer,
            span,
        })
    }

    fn function(&mut self) -> Result<Stmt, NivError> {
        let span = self.previous_span();
        let name = self.consume_identifier("expected function name")?;
        self.consume(&TokenKind::LeftParen, "expected '(' after function name")?;
        let mut params = vec![];
        if !self.check(&TokenKind::RightParen) {
            loop {
                if params.len() >= 255 {
                    return Err(self.error_here("functions may have at most 255 parameters"));
                }
                let param_span = Span {
                    line: self.peek().line,
                    column: self.peek().column,
                };
                let param_name = self.consume_identifier("expected parameter name")?;
                let ty = if self.matches(&[TokenKind::Colon]) {
                    Some(self.type_ref()?)
                } else {
                    None
                };
                params.push(Param {
                    name: param_name,
                    ty,
                    span: param_span,
                });
                if !self.matches(&[TokenKind::Comma]) {
                    break;
                }
            }
        }
        self.consume(&TokenKind::RightParen, "expected ')' after parameters")?;
        let return_type = if self.matches(&[TokenKind::Arrow]) {
            Some(self.type_ref()?)
        } else {
            None
        };
        self.consume(&TokenKind::LeftBrace, "expected '{' before function body")?;
        let body = self.block_contents()?;
        Ok(Stmt::Function {
            name,
            params,
            return_type,
            body,
            span,
        })
    }

    fn record(&mut self) -> Result<Stmt, NivError> {
        let span = self.previous_span();
        let name = self.consume_identifier("expected shape name")?;
        self.consume(&TokenKind::LeftBrace, "expected '{' after shape name")?;
        let mut fields = vec![];
        while !self.check(&TokenKind::RightBrace) && !self.is_at_end() {
            let field_span = Span {
                line: self.peek().line,
                column: self.peek().column,
            };
            let field_name = self.consume_identifier("expected field name")?;
            self.consume(&TokenKind::Colon, "expected ':' after field name")?;
            let ty = self.type_ref()?;
            fields.push(FieldDef {
                name: field_name,
                ty,
                span: field_span,
            });
            if !self.matches(&[TokenKind::Comma, TokenKind::Semicolon])
                && !self.check(&TokenKind::RightBrace)
            {
                return Err(self.error_here("expected ',' or '}' after shape field"));
            }
        }
        self.consume(&TokenKind::RightBrace, "expected '}' after shape")?;
        Ok(Stmt::Record { name, fields, span })
    }

    fn enum_declaration(&mut self) -> Result<Stmt, NivError> {
        let span = self.previous_span();
        let name = self.consume_identifier("expected choice name")?;
        self.consume(&TokenKind::LeftBrace, "expected '{' after choice name")?;
        let mut variants = vec![];
        while !self.check(&TokenKind::RightBrace) && !self.is_at_end() {
            variants.push(self.consume_identifier("expected variant name")?);
            if !self.matches(&[TokenKind::Comma, TokenKind::Semicolon])
                && !self.check(&TokenKind::RightBrace)
            {
                return Err(self.error_here("expected ',' or '}' after choice variant"));
            }
        }
        self.consume(&TokenKind::RightBrace, "expected '}' after choice")?;
        if variants.is_empty() {
            return Err(NivError::new(
                "choice requires at least one variant",
                span.line,
                span.column,
            ));
        }
        Ok(Stmt::Enum {
            name,
            variants,
            span,
        })
    }

    fn import_declaration(&mut self) -> Result<Stmt, NivError> {
        let span = self.previous_span();
        let path = match self.advance().kind.clone() {
            TokenKind::String(path) => path,
            _ => return Err(self.error_here("expected quoted use path")),
        };
        self.optional_semicolon();
        Ok(Stmt::Import { path, span })
    }

    fn export_declaration(&mut self) -> Result<Stmt, NivError> {
        let span = self.previous_span();
        self.consume(&TokenKind::LeftBrace, "expected '{' after expose")?;
        let mut names = vec![];
        if !self.check(&TokenKind::RightBrace) {
            loop {
                names.push(self.consume_identifier("expected exposed name")?);
                if !self.matches(&[TokenKind::Comma]) {
                    break;
                }
            }
        }
        self.consume(&TokenKind::RightBrace, "expected '}' after exposed names")?;
        self.optional_semicolon();
        if names.is_empty() {
            return Err(NivError::new(
                "expose list cannot be empty",
                span.line,
                span.column,
            ));
        }
        Ok(Stmt::Export { names, span })
    }

    fn type_ref(&mut self) -> Result<TypeRef, NivError> {
        let mut result = if self.matches(&[TokenKind::LeftBracket]) {
            let span = self.previous_span();
            let element = self.type_ref()?;
            self.consume(
                &TokenKind::RightBracket,
                "expected ']' after array element type",
            )?;
            TypeRef::Array(Box::new(element), span)
        } else {
            let span = Span {
                line: self.peek().line,
                column: self.peek().column,
            };
            let name = self.consume_identifier("expected type name")?;
            if name == "Result" && self.matches(&[TokenKind::Less]) {
                let ok = self.type_ref()?;
                self.consume(&TokenKind::Comma, "expected ',' between Result types")?;
                let error = self.type_ref()?;
                self.consume(&TokenKind::Greater, "expected '>' after Result types")?;
                TypeRef::Result(Box::new(ok), Box::new(error), span)
            } else {
                TypeRef::Named(name, span)
            }
        };
        if self.matches(&[TokenKind::Question]) {
            let span = self.previous_span();
            result = TypeRef::Nullable(Box::new(result), span);
        }
        Ok(result)
    }

    fn statement(&mut self) -> Result<Stmt, NivError> {
        if self.matches(&[TokenKind::Print]) {
            return self.print_statement();
        }
        if self.matches(&[TokenKind::Return]) {
            return self.return_statement();
        }
        if self.matches(&[TokenKind::If]) {
            return self.if_statement();
        }
        if self.matches(&[TokenKind::While]) {
            return self.while_statement();
        }
        if self.matches(&[TokenKind::For]) {
            return self.for_statement();
        }
        if self.matches(&[TokenKind::LeftBrace]) {
            let span = self.previous_span();
            return Ok(Stmt::Block(self.block_contents()?, span));
        }
        let expression = self.expression()?;
        self.optional_semicolon();
        Ok(Stmt::Expression(expression))
    }

    fn print_statement(&mut self) -> Result<Stmt, NivError> {
        let span = self.previous_span();
        let value = if self.matches(&[TokenKind::LeftParen]) {
            let expression = self.expression()?;
            self.consume(&TokenKind::RightParen, "expected ')' after show value")?;
            expression
        } else {
            self.expression()?
        };
        self.optional_semicolon();
        Ok(Stmt::Print(value, span))
    }

    fn return_statement(&mut self) -> Result<Stmt, NivError> {
        let span = self.previous_span();
        let value = if self.check(&TokenKind::Semicolon) || self.check(&TokenKind::RightBrace) {
            None
        } else {
            Some(self.expression()?)
        };
        self.optional_semicolon();
        Ok(Stmt::Return(value, span))
    }

    fn if_statement(&mut self) -> Result<Stmt, NivError> {
        let span = self.previous_span();
        let condition = self.control_condition("when")?;
        let then_branch = Box::new(self.statement()?);
        let else_branch = if self.matches(&[TokenKind::Else]) {
            Some(Box::new(self.statement()?))
        } else {
            None
        };
        Ok(Stmt::If {
            condition,
            then_branch,
            else_branch,
            span,
        })
    }

    fn while_statement(&mut self) -> Result<Stmt, NivError> {
        let span = self.previous_span();
        let condition = self.control_condition("repeat")?;
        let body = Box::new(self.statement()?);
        Ok(Stmt::While {
            condition,
            body,
            span,
        })
    }

    fn for_statement(&mut self) -> Result<Stmt, NivError> {
        let span = self.previous_span();
        let parenthesized = self.matches(&[TokenKind::LeftParen]);
        let name = self.consume_identifier("expected iteration binding")?;
        self.consume(&TokenKind::In, "expected 'within' after iteration binding")?;
        let iterable = self.expression()?;
        if parenthesized {
            self.consume(&TokenKind::RightParen, "expected ')' after iterable")?;
        }
        let body = Box::new(self.statement()?);
        Ok(Stmt::For {
            name,
            iterable,
            body,
            span,
        })
    }

    fn control_condition(&mut self, keyword: &str) -> Result<Expr, NivError> {
        if self.matches(&[TokenKind::LeftParen]) {
            let condition = self.expression()?;
            self.consume(
                &TokenKind::RightParen,
                &format!("expected ')' after {keyword} condition"),
            )?;
            Ok(condition)
        } else {
            self.expression()
        }
    }

    fn block_contents(&mut self) -> Result<Vec<Stmt>, NivError> {
        let mut statements = vec![];
        while !self.check(&TokenKind::RightBrace) && !self.is_at_end() {
            statements.push(self.declaration()?);
        }
        self.consume(&TokenKind::RightBrace, "expected '}' after block")?;
        Ok(statements)
    }

    fn expression(&mut self) -> Result<Expr, NivError> {
        self.assignment()
    }

    fn assignment(&mut self) -> Result<Expr, NivError> {
        let expression = self.coalesce()?;
        if self.matches(&[TokenKind::Equal]) {
            let span = self.previous_span();
            let value = self.assignment()?;
            if let Expr::Variable(name, _) = expression {
                return Ok(Expr::Assign(name, Box::new(value), span));
            }
            return Err(NivError::new(
                "invalid assignment target",
                span.line,
                span.column,
            ));
        }
        Ok(expression)
    }

    fn coalesce(&mut self) -> Result<Expr, NivError> {
        let expression = self.or()?;
        if self.matches(&[TokenKind::QuestionQuestion]) {
            let span = self.previous_span();
            return Ok(Expr::Coalesce(
                Box::new(expression),
                Box::new(self.coalesce()?),
                span,
            ));
        }
        Ok(expression)
    }

    fn or(&mut self) -> Result<Expr, NivError> {
        let mut expression = self.and()?;
        while self.matches(&[TokenKind::Or]) {
            let operator = self.previous().kind.clone();
            let span = self.previous_span();
            expression = Expr::Logical(Box::new(expression), operator, Box::new(self.and()?), span);
        }
        Ok(expression)
    }

    fn and(&mut self) -> Result<Expr, NivError> {
        let mut expression = self.equality()?;
        while self.matches(&[TokenKind::And]) {
            let operator = self.previous().kind.clone();
            let span = self.previous_span();
            expression = Expr::Logical(
                Box::new(expression),
                operator,
                Box::new(self.equality()?),
                span,
            );
        }
        Ok(expression)
    }

    fn equality(&mut self) -> Result<Expr, NivError> {
        self.binary(
            Self::comparison,
            &[TokenKind::BangEqual, TokenKind::EqualEqual],
        )
    }
    fn comparison(&mut self) -> Result<Expr, NivError> {
        self.binary(
            Self::term,
            &[
                TokenKind::Greater,
                TokenKind::GreaterEqual,
                TokenKind::Less,
                TokenKind::LessEqual,
            ],
        )
    }
    fn term(&mut self) -> Result<Expr, NivError> {
        self.binary(Self::factor, &[TokenKind::Plus, TokenKind::Minus])
    }
    fn factor(&mut self) -> Result<Expr, NivError> {
        self.binary(
            Self::unary,
            &[TokenKind::Star, TokenKind::Slash, TokenKind::Percent],
        )
    }

    fn binary(
        &mut self,
        next: fn(&mut Self) -> Result<Expr, NivError>,
        kinds: &[TokenKind],
    ) -> Result<Expr, NivError> {
        let mut expression = next(self)?;
        while self.matches(kinds) {
            let operator = self.previous().kind.clone();
            let span = self.previous_span();
            expression = Expr::Binary(Box::new(expression), operator, Box::new(next(self)?), span);
        }
        Ok(expression)
    }

    fn unary(&mut self) -> Result<Expr, NivError> {
        if self.matches(&[TokenKind::Bang, TokenKind::Minus]) {
            let operator = self.previous().kind.clone();
            let span = self.previous_span();
            return Ok(Expr::Unary(operator, Box::new(self.unary()?), span));
        }
        self.call()
    }

    fn call(&mut self) -> Result<Expr, NivError> {
        let mut expression = self.primary()?;
        loop {
            if self.matches(&[TokenKind::LeftParen]) {
                let span = self.previous_span();
                let mut args = vec![];
                if !self.check(&TokenKind::RightParen) {
                    loop {
                        if args.len() >= 255 {
                            return Err(self.error_here("calls may have at most 255 arguments"));
                        }
                        args.push(self.expression()?);
                        if !self.matches(&[TokenKind::Comma]) {
                            break;
                        }
                    }
                }
                self.consume(&TokenKind::RightParen, "expected ')' after arguments")?;
                expression = Expr::Call(Box::new(expression), args, span);
            } else if self.matches(&[TokenKind::LeftBracket]) {
                let span = self.previous_span();
                let index = self.expression()?;
                self.consume(&TokenKind::RightBracket, "expected ']' after index")?;
                expression = Expr::Index(Box::new(expression), Box::new(index), span);
            } else if self.matches(&[TokenKind::Dot]) {
                let span = self.previous_span();
                let name = self.consume_identifier("expected field name after '.'")?;
                expression = Expr::Get(Box::new(expression), name, span);
            } else {
                break;
            }
        }
        Ok(expression)
    }

    fn primary(&mut self) -> Result<Expr, NivError> {
        let token = self.advance().clone();
        let span = Span {
            line: token.line,
            column: token.column,
        };
        match token.kind {
            TokenKind::False => Ok(Expr::Literal(Literal::Bool(false), span)),
            TokenKind::True => Ok(Expr::Literal(Literal::Bool(true), span)),
            TokenKind::Null => Ok(Expr::Literal(Literal::Null, span)),
            TokenKind::Int(value) => Ok(Expr::Literal(Literal::Int(value), span)),
            TokenKind::Float(value) => Ok(Expr::Literal(Literal::Float(value), span)),
            TokenKind::String(value) => Ok(Expr::Literal(Literal::String(value), span)),
            TokenKind::Identifier(name) => Ok(Expr::Variable(name, span)),
            TokenKind::Match => self.match_expression(span),
            TokenKind::LeftBracket => {
                let mut values = vec![];
                if !self.check(&TokenKind::RightBracket) {
                    loop {
                        values.push(self.expression()?);
                        if !self.matches(&[TokenKind::Comma]) {
                            break;
                        }
                    }
                }
                self.consume(&TokenKind::RightBracket, "expected ']' after array")?;
                Ok(Expr::Array(values, span))
            }
            TokenKind::LeftParen => {
                let value = self.expression()?;
                self.consume(&TokenKind::RightParen, "expected ')' after expression")?;
                Ok(value)
            }
            _ => Err(NivError::new("expected expression", span.line, span.column)),
        }
    }

    fn match_expression(&mut self, span: Span) -> Result<Expr, NivError> {
        let subject = if self.matches(&[TokenKind::LeftParen]) {
            let value = self.expression()?;
            self.consume(&TokenKind::RightParen, "expected ')' after choose value")?;
            value
        } else {
            self.expression()?
        };
        self.consume(&TokenKind::LeftBrace, "expected '{' before choose arms")?;
        let mut arms = vec![];
        while !self.check(&TokenKind::RightBrace) && !self.is_at_end() {
            let arm_span = Span {
                line: self.peek().line,
                column: self.peek().column,
            };
            let variant = self.consume_identifier("expected variant name")?;
            let binding = if self.matches(&[TokenKind::LeftParen]) {
                let binding = self.consume_identifier("expected payload binding")?;
                self.consume(&TokenKind::RightParen, "expected ')' after payload binding")?;
                Some(binding)
            } else {
                None
            };
            self.consume(&TokenKind::FatArrow, "expected '=>' after choice arm")?;
            let value = self.expression()?;
            arms.push(MatchArm {
                variant,
                binding,
                value,
                span: arm_span,
            });
            if !self.matches(&[TokenKind::Comma, TokenKind::Semicolon])
                && !self.check(&TokenKind::RightBrace)
            {
                return Err(self.error_here("expected ',' or '}' after choose arm"));
            }
        }
        self.consume(&TokenKind::RightBrace, "expected '}' after choose")?;
        Ok(Expr::Match(Box::new(subject), arms, span))
    }

    fn matches(&mut self, kinds: &[TokenKind]) -> bool {
        if kinds.iter().any(|kind| self.check(kind)) {
            self.advance();
            true
        } else {
            false
        }
    }
    fn check(&self, kind: &TokenKind) -> bool {
        same_variant(&self.peek().kind, kind)
    }
    fn consume(&mut self, kind: &TokenKind, message: &str) -> Result<(), NivError> {
        if self.check(kind) {
            self.advance();
            Ok(())
        } else {
            Err(self.error_here(message))
        }
    }
    fn consume_identifier(&mut self, message: &str) -> Result<String, NivError> {
        match self.peek().kind.clone() {
            TokenKind::Identifier(name) => {
                self.advance();
                Ok(name)
            }
            _ => Err(self.error_here(message)),
        }
    }
    fn optional_semicolon(&mut self) {
        self.matches(&[TokenKind::Semicolon]);
    }
    fn is_at_end(&self) -> bool {
        matches!(self.peek().kind, TokenKind::Eof)
    }
    fn peek(&self) -> &Token {
        &self.tokens[self.current]
    }
    fn previous(&self) -> &Token {
        &self.tokens[self.current - 1]
    }
    fn advance(&mut self) -> &Token {
        if !self.is_at_end() {
            self.current += 1;
        }
        self.previous()
    }
    fn previous_span(&self) -> Span {
        Span {
            line: self.previous().line,
            column: self.previous().column,
        }
    }
    fn error_here(&self, message: &str) -> NivError {
        NivError::new(message, self.peek().line, self.peek().column)
    }
    fn synchronize(&mut self) {
        if !self.is_at_end() {
            self.advance();
        }
        while !self.is_at_end() {
            if matches!(self.previous().kind, TokenKind::Semicolon) {
                return;
            }
            if matches!(
                self.peek().kind,
                TokenKind::Let
                    | TokenKind::Var
                    | TokenKind::Fun
                    | TokenKind::If
                    | TokenKind::While
                    | TokenKind::For
                    | TokenKind::Return
                    | TokenKind::Print
                    | TokenKind::Record
                    | TokenKind::Enum
                    | TokenKind::Import
                    | TokenKind::Export
            ) {
                return;
            }
            self.advance();
        }
    }
}

fn same_variant(left: &TokenKind, right: &TokenKind) -> bool {
    std::mem::discriminant(left) == std::mem::discriminant(right)
}
