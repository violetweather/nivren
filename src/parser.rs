use crate::ast::{
    CapabilityNeed, Expr, FieldDef, Literal, MatchArm, Param, Span, Stmt, TypeParam, TypeRef,
};
use crate::error::NivError;
use crate::lexer::{Token, TokenKind};
use std::collections::{BTreeSet, HashMap};

const MAX_PARSE_DEPTH: usize = 128;

pub fn parse(tokens: Vec<Token>) -> Result<Vec<Stmt>, Vec<NivError>> {
    Parser {
        tokens,
        current: 0,
        errors: vec![],
        depth: 0,
        callables: HashMap::new(),
    }
    .program()
}

struct Parser {
    tokens: Vec<Token>,
    current: usize,
    errors: Vec<NivError>,
    depth: usize,
    callables: HashMap<String, Vec<String>>,
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
            return self.change_declaration();
        }
        if self.matches(&[TokenKind::Fun]) {
            return self.function();
        }
        if self.matches(&[TokenKind::Record]) {
            return self.record();
        }
        if self.check_identifier_value("type")
            && self
                .tokens
                .get(self.current + 1)
                .is_some_and(|token| matches!(token.kind, TokenKind::Identifier(_)))
        {
            self.advance();
            return self.nominal_type();
        }
        if self.matches(&[TokenKind::Enum]) {
            return self.enum_declaration();
        }
        if self.matches(&[TokenKind::Protocol]) {
            return self.protocol_declaration();
        }
        if self.matches(&[TokenKind::Adopt]) {
            return self.adoption_declaration();
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
        let edition_four = self.check(&TokenKind::Is) || self.check(&TokenKind::Set);
        let annotation = if self.matches(&[TokenKind::Colon, TokenKind::Is]) {
            Some(self.type_ref()?)
        } else {
            None
        };
        if edition_four {
            if !self.matches(&[TokenKind::Set]) {
                self.consume(&TokenKind::Set, "this binding states its intent with 'set'")?;
            }
        } else {
            self.consume(&TokenKind::Equal, "expected '=' after binding name")?;
        }
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

    fn change_declaration(&mut self) -> Result<Stmt, NivError> {
        let span = self.previous_span();
        let name = self.consume_identifier("expected a name after 'change'")?;
        if self.matches(&[TokenKind::To]) {
            let value = self.expression()?;
            self.optional_semicolon();
            return Ok(Stmt::Expression(Expr::Assign(name, Box::new(value), span)));
        }
        let edition_four = self.check(&TokenKind::Is) || self.check(&TokenKind::Set);
        let annotation = if self.matches(&[TokenKind::Colon, TokenKind::Is]) {
            Some(self.type_ref()?)
        } else {
            None
        };
        if edition_four {
            if !self.matches(&[TokenKind::Set]) {
                self.consume(
                    &TokenKind::Set,
                    "a mutable binding uses 'set' for its initial value",
                )?;
            }
        } else {
            self.consume(&TokenKind::Equal, "expected '=' after binding name")?;
        }
        let initializer = self.expression()?;
        self.optional_semicolon();
        Ok(Stmt::Let {
            name,
            mutable: true,
            annotation,
            initializer,
            span,
        })
    }

    fn function(&mut self) -> Result<Stmt, NivError> {
        let span = self.previous_span();
        let name = self.consume_identifier("expected function name")?;
        let type_params = self.type_parameters()?;
        let mut params = vec![];
        if self.matches(&[TokenKind::Takes]) {
            self.consume(&TokenKind::LeftBrace, "expected '{' after 'takes'")?;
            while !self.check(&TokenKind::RightBrace) && !self.is_at_end() {
                let param_span = Span {
                    line: self.peek().line,
                    column: self.peek().column,
                };
                let param_name = self.consume_identifier("expected an input name")?;
                self.consume(&TokenKind::Is, "an input states its type with 'is'")?;
                params.push(Param {
                    name: param_name,
                    ty: Some(self.type_ref()?),
                    span: param_span,
                });
                self.matches(&[TokenKind::Comma, TokenKind::Semicolon]);
            }
            self.consume(&TokenKind::RightBrace, "expected '}' after inputs")?;
        } else if self.check(&TokenKind::LeftParen) {
            self.consume(
                &TokenKind::LeftParen,
                "expected 'takes' or '(' after function name",
            )?;
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
        }
        let return_type = if self.matches(&[TokenKind::Arrow]) {
            let value = self.type_ref()?;
            if self.matches(&[TokenKind::Or]) {
                let problem = self.type_ref()?;
                let span = value_span(&value);
                Some(TypeRef::Result(Box::new(value), Box::new(problem), span))
            } else {
                Some(value)
            }
        } else {
            None
        };
        let mut needs = vec![];
        let mut capability_needs = vec![];
        if self.matches(&[TokenKind::Needs]) {
            loop {
                let need_span = Span {
                    line: self.peek().line,
                    column: self.peek().column,
                };
                let capability = self.consume_identifier("expected capability after needs")?;
                if needs.contains(&capability) {
                    return Err(self.error_here("duplicate capability in needs list"));
                }
                needs.push(capability.clone());
                let boundary = if self.matches(&[TokenKind::In]) {
                    match self.advance().kind.clone() {
                        TokenKind::String(boundary) => Some(boundary),
                        _ => return Err(self.error_here("a scoped need expects a quoted boundary")),
                    }
                } else {
                    None
                };
                validate_capability_need(&capability, boundary.as_deref(), need_span)?;
                capability_needs.push(CapabilityNeed {
                    capability,
                    boundary,
                    span: need_span,
                });
                if !self.matches(&[TokenKind::Comma]) {
                    break;
                }
            }
        }
        self.callables.insert(
            name.clone(),
            params
                .iter()
                .map(|parameter| parameter.name.clone())
                .collect(),
        );
        self.consume(&TokenKind::LeftBrace, "expected '{' before function body")?;
        let body = self.block_contents()?;
        Ok(Stmt::Function {
            name,
            type_params,
            params,
            return_type,
            needs,
            capability_needs,
            body,
            span,
        })
    }

    fn type_parameters(&mut self) -> Result<Vec<TypeParam>, NivError> {
        let mut type_params = vec![];
        if self.matches(&[TokenKind::Less]) {
            loop {
                if type_params.len() >= 255 {
                    return Err(
                        self.error_here("declarations may have at most 255 type parameters")
                    );
                }
                let parameter_span = Span {
                    line: self.peek().line,
                    column: self.peek().column,
                };
                let parameter = self.consume_identifier("expected generic type parameter")?;
                if type_params
                    .iter()
                    .any(|existing: &TypeParam| existing.name == parameter)
                {
                    return Err(self.error_here("duplicate generic type parameter"));
                }
                let constraint = if self.matches(&[TokenKind::Colon, TokenKind::Is]) {
                    Some(self.consume_identifier("expected protocol constraint")?)
                } else {
                    None
                };
                type_params.push(TypeParam {
                    name: parameter,
                    constraint,
                    span: parameter_span,
                });
                if !self.matches(&[TokenKind::Comma]) {
                    break;
                }
            }
            self.consume(
                &TokenKind::Greater,
                "expected '>' after generic type parameters",
            )?;
        }
        Ok(type_params)
    }

    fn record(&mut self) -> Result<Stmt, NivError> {
        let span = self.previous_span();
        let name = self.consume_identifier("expected shape name")?;
        let type_params = self.type_parameters()?;
        let edition_four = self.matches(&[TokenKind::Holds]);
        self.consume(&TokenKind::LeftBrace, "expected 'holds {' after shape name")?;
        let mut fields = vec![];
        while !self.check(&TokenKind::RightBrace) && !self.is_at_end() {
            let field_span = Span {
                line: self.peek().line,
                column: self.peek().column,
            };
            let field_name = self.consume_identifier("expected field name")?;
            if !self.matches(&[TokenKind::Colon, TokenKind::Is]) {
                return Err(self.error_here("a shape field states its type with 'is'"));
            }
            let ty = self.type_ref()?;
            fields.push(FieldDef {
                name: field_name,
                ty,
                span: field_span,
            });
            if !edition_four
                && !self.matches(&[TokenKind::Comma, TokenKind::Semicolon])
                && !self.check(&TokenKind::RightBrace)
            {
                return Err(self.error_here("expected ',' or '}' after shape field"));
            }
        }
        self.consume(&TokenKind::RightBrace, "expected '}' after shape")?;
        let derives = self.derive_list()?;
        self.callables.insert(
            name.clone(),
            fields.iter().map(|field| field.name.clone()).collect(),
        );
        for method in crate::derive_methods::METHODS
            .iter()
            .filter(|method| derives.iter().any(|derive| derive == method.derive))
        {
            self.callables.insert(
                format!("{name}.{}", method.name),
                method.labels.iter().map(ToString::to_string).collect(),
            );
        }
        Ok(Stmt::Record {
            name,
            type_params,
            fields,
            derives,
            span,
        })
    }

    fn nominal_type(&mut self) -> Result<Stmt, NivError> {
        let span = self.previous_span();
        let name = self.consume_identifier("expected nominal type name")?;
        if !self.check_identifier_value("from") {
            return Err(self.error_here("a nominal type names its representation with 'from'"));
        }
        self.advance();
        let representation = self.type_ref()?;
        self.optional_semicolon();
        self.callables.insert(name.clone(), vec!["value".into()]);
        Ok(Stmt::Record {
            name,
            type_params: vec![],
            fields: vec![FieldDef {
                name: "value".into(),
                ty: representation,
                span,
            }],
            derives: vec![],
            span,
        })
    }

    fn derive_list(&mut self) -> Result<Vec<String>, NivError> {
        if !self.matches(&[TokenKind::With]) {
            return Ok(vec![]);
        }
        let mut derives = vec![];
        loop {
            let derive = self.consume_identifier("expected a derive name after 'with'")?;
            const BUILT_INS: &[&str] = &[
                "Json",
                "Compare",
                "Display",
                "Key",
                "Validate",
                "Binary",
                "DatabaseRow",
                "Arguments",
            ];
            if !BUILT_INS.contains(&derive.as_str()) {
                return Err(self.error_here(&format!(
                    "unknown derive '{derive}'; Edition 4 derives are {}",
                    BUILT_INS.join(", ")
                )));
            }
            if derives.contains(&derive) {
                return Err(self.error_here(&format!("derive '{derive}' appears more than once")));
            }
            derives.push(derive);
            if !self.matches(&[TokenKind::Comma]) {
                break;
            }
        }
        Ok(derives)
    }

    fn enum_declaration(&mut self) -> Result<Stmt, NivError> {
        let span = self.previous_span();
        let name = self.consume_identifier("expected choice name")?;
        let type_params = self.type_parameters()?;
        let edition_four = self.matches(&[TokenKind::Holds]);
        self.consume(
            &TokenKind::LeftBrace,
            "expected 'holds {' after choice name",
        )?;
        let mut variants = vec![];
        while !self.check(&TokenKind::RightBrace) && !self.is_at_end() {
            let variant_span = Span {
                line: self.peek().line,
                column: self.peek().column,
            };
            let uses_case = self.matches(&[TokenKind::Case]);
            let variant_name = self.consume_identifier("expected case name")?;
            let payload = if self.matches(&[TokenKind::Carries]) {
                Some(self.type_ref()?)
            } else if self.matches(&[TokenKind::LeftParen]) {
                let payload = self.type_ref()?;
                self.consume(
                    &TokenKind::RightParen,
                    "expected ')' after choice variant payload type",
                )?;
                Some(payload)
            } else {
                None
            };
            variants.push(crate::ast::VariantDef {
                name: variant_name,
                payload,
                span: variant_span,
            });
            if !edition_four
                && !uses_case
                && !self.matches(&[TokenKind::Comma, TokenKind::Semicolon])
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
            type_params,
            variants,
            span,
        })
    }

    fn protocol_declaration(&mut self) -> Result<Stmt, NivError> {
        let span = self.previous_span();
        let name = self.consume_identifier("expected protocol name")?;
        let mut members = vec![];
        if self.matches(&[TokenKind::LeftBrace]) {
            while !self.check(&TokenKind::RightBrace) && !self.is_at_end() {
                self.consume(&TokenKind::Fun, "expected 'define' before protocol member")?;
                let member_span = self.previous_span();
                let member_name = self.consume_identifier("expected protocol member name")?;
                let mut params = vec![];
                if self.matches(&[TokenKind::Takes]) {
                    self.consume(&TokenKind::LeftBrace, "expected '{' after 'takes'")?;
                    while !self.check(&TokenKind::RightBrace) && !self.is_at_end() {
                        let parameter_span = Span {
                            line: self.peek().line,
                            column: self.peek().column,
                        };
                        let parameter_name =
                            self.consume_identifier("expected protocol parameter name")?;
                        self.consume(&TokenKind::Is, "a protocol input states its type with 'is'")?;
                        params.push(Param {
                            name: parameter_name,
                            ty: Some(self.type_ref()?),
                            span: parameter_span,
                        });
                        self.matches(&[TokenKind::Comma, TokenKind::Semicolon]);
                    }
                    self.consume(&TokenKind::RightBrace, "expected '}' after protocol inputs")?;
                } else {
                    self.consume(
                        &TokenKind::LeftParen,
                        "expected 'takes' or '(' after protocol member name",
                    )?;
                    if !self.check(&TokenKind::RightParen) {
                        loop {
                            let parameter_span = Span {
                                line: self.peek().line,
                                column: self.peek().column,
                            };
                            let parameter_name =
                                self.consume_identifier("expected protocol parameter name")?;
                            self.consume(
                                &TokenKind::Colon,
                                "protocol parameters require an explicit type",
                            )?;
                            params.push(Param {
                                name: parameter_name,
                                ty: Some(self.type_ref()?),
                                span: parameter_span,
                            });
                            if !self.matches(&[TokenKind::Comma]) {
                                break;
                            }
                        }
                    }
                    self.consume(
                        &TokenKind::RightParen,
                        "expected ')' after protocol parameters",
                    )?;
                }
                self.consume(&TokenKind::Arrow, "protocol members require a 'gives' type")?;
                let value_type = self.type_ref()?;
                let return_type = if self.matches(&[TokenKind::Or]) {
                    let problem_type = self.type_ref()?;
                    TypeRef::Result(Box::new(value_type), Box::new(problem_type), member_span)
                } else {
                    value_type
                };
                let mut needs = vec![];
                if self.matches(&[TokenKind::Needs]) {
                    loop {
                        let capability =
                            self.consume_identifier("expected capability after needs")?;
                        if needs.contains(&capability) {
                            return Err(self.error_here("duplicate capability in needs list"));
                        }
                        needs.push(capability);
                        if !self.matches(&[TokenKind::Comma]) {
                            break;
                        }
                    }
                }
                members.push(crate::ast::ProtocolMember {
                    name: member_name,
                    params,
                    return_type,
                    needs,
                    span: member_span,
                });
                self.optional_semicolon();
            }
            self.consume(
                &TokenKind::RightBrace,
                "expected '}' after protocol members",
            )?;
        }
        self.optional_semicolon();
        Ok(Stmt::Protocol {
            name,
            members,
            span,
        })
    }

    fn adoption_declaration(&mut self) -> Result<Stmt, NivError> {
        let span = self.previous_span();
        let protocol = self.consume_identifier("expected protocol name after adopt")?;
        self.consume(&TokenKind::ForType, "expected 'for' after protocol name")?;
        let ty = self.type_ref()?;
        let mut members = vec![];
        if self.matches(&[TokenKind::LeftBrace]) {
            while !self.check(&TokenKind::RightBrace) && !self.is_at_end() {
                let member_span = Span {
                    line: self.peek().line,
                    column: self.peek().column,
                };
                let member = self.consume_identifier("expected protocol member name")?;
                if !self.matches(&[TokenKind::Set, TokenKind::Equal]) {
                    return Err(self.error_here("a protocol adoption maps a member with 'set'"));
                }
                let implementation =
                    self.consume_identifier("expected implementation function name")?;
                members.push(crate::ast::AdoptionMember {
                    member,
                    implementation,
                    span: member_span,
                });
                if !self.matches(&[TokenKind::Comma, TokenKind::Semicolon])
                    && !self.check(&TokenKind::RightBrace)
                {
                    return Err(self.error_here("expected ',' or '}' after protocol mapping"));
                }
            }
            self.consume(
                &TokenKind::RightBrace,
                "expected '}' after protocol adoption",
            )?;
        }
        self.optional_semicolon();
        Ok(Stmt::Adoption {
            protocol,
            ty,
            members,
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
        self.enter_nested_syntax()?;
        let result = self.type_ref_inner();
        self.depth -= 1;
        result
    }

    fn type_ref_inner(&mut self) -> Result<TypeRef, NivError> {
        let mut result = if self.matches(&[TokenKind::Maybe]) {
            let span = self.previous_span();
            TypeRef::Nullable(Box::new(self.type_ref()?), span)
        } else if self.matches(&[TokenKind::LeftBracket]) {
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
            if self.matches(&[TokenKind::Less]) {
                let mut arguments = vec![];
                loop {
                    arguments.push(self.type_ref()?);
                    if !self.matches(&[TokenKind::Comma]) {
                        break;
                    }
                }
                self.consume(&TokenKind::Greater, "expected '>' after type arguments")?;
                if name == "Result" && arguments.len() == 2 {
                    TypeRef::Result(
                        Box::new(arguments.remove(0)),
                        Box::new(arguments.remove(0)),
                        span,
                    )
                } else {
                    TypeRef::Applied(name, arguments, span)
                }
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
        self.enter_nested_syntax()?;
        let result = self.statement_inner();
        self.depth -= 1;
        result
    }

    fn statement_inner(&mut self) -> Result<Stmt, NivError> {
        if self.matches(&[TokenKind::Prepare]) {
            return self.prepare_statement();
        }
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
        if self.matches(&[TokenKind::Using]) {
            return self.using_statement();
        }
        if self.matches(&[TokenKind::LeftBrace]) {
            let span = self.previous_span();
            return Ok(Stmt::Block(self.block_contents()?, span));
        }
        if let Some(statement) = self.loop_exit_statement() {
            return Ok(statement);
        }
        let expression = self.expression()?;
        self.optional_semicolon();
        Ok(Stmt::Expression(expression))
    }

    /// `stop` and `skip` are contextual statement keywords: they end or
    /// advance the nearest loop only when the word stands alone, so member
    /// access such as `std.iter.skip` keeps its ordinary meaning.
    fn loop_exit_statement(&mut self) -> Option<Stmt> {
        let TokenKind::Identifier(word) = &self.peek().kind else {
            return None;
        };
        let stop = match word.as_str() {
            "stop" => true,
            "skip" => false,
            _ => return None,
        };
        if expression_continues(&self.tokens[self.current + 1].kind) {
            return None;
        }
        self.advance();
        let span = self.previous_span();
        self.optional_semicolon();
        Some(if stop {
            Stmt::Stop(span)
        } else {
            Stmt::Skip(span)
        })
    }

    fn prepare_statement(&mut self) -> Result<Stmt, NivError> {
        let span = self.previous_span();
        let name = self.consume_identifier("expected a plan name after 'prepare'")?;
        self.consume(&TokenKind::As, "a prepared plan states its type with 'as'")?;
        let plan_type = self.consume_identifier("expected a plan type after 'as'")?;
        self.consume(
            &TokenKind::With,
            "a prepared plan provides values with 'with'",
        )?;
        let (labels, arguments) = self.intent_arguments()?;
        self.validate_labels(&plan_type, &labels, span)?;
        self.optional_semicolon();
        Ok(Stmt::Prepare {
            name,
            plan_type: plan_type.clone(),
            initializer: Expr::Call(
                Box::new(Expr::Variable(plan_type, span)),
                arguments,
                Some(labels),
                span,
            ),
            span,
        })
    }

    fn intent_arguments(&mut self) -> Result<(Vec<String>, Vec<Expr>), NivError> {
        self.consume(&TokenKind::LeftBrace, "expected '{' before labeled values")?;
        let mut arguments = vec![];
        let mut names = BTreeSet::new();
        let mut labels = vec![];
        while !self.check(&TokenKind::RightBrace) && !self.is_at_end() {
            let name = self.consume_identifier("expected a labeled value name")?;
            if !names.insert(name.clone()) {
                return Err(
                    self.error_here(&format!("labeled value '{name}' appears more than once"))
                );
            }
            self.consume(&TokenKind::Set, "a labeled value uses 'set'")?;
            labels.push(name);
            arguments.push(self.expression()?);
            self.matches(&[TokenKind::Comma, TokenKind::Semicolon]);
        }
        self.consume(&TokenKind::RightBrace, "expected '}' after labeled values")?;
        Ok((labels, arguments))
    }

    fn validate_labels(
        &self,
        callable: &str,
        labels: &[String],
        span: Span,
    ) -> Result<(), NivError> {
        let expected = self
            .callables
            .get(callable)
            .map(Vec::as_slice)
            .or_else(|| crate::call_labels::get(callable));
        let Some(expected) = expected else {
            return Ok(());
        };
        if labels == expected || (expected.len() == labels.len() + 1 && labels == &expected[1..]) {
            return Ok(());
        }
        Err(NivError::new(
            format!(
                "{callable} expects labeled values [{}] in canonical order; received [{}]",
                expected.join(", "),
                labels.join(", ")
            ),
            span.line,
            span.column,
        ))
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
        let carries = self.matches(&[TokenKind::Carries]);
        let binding = if carries {
            Some(self.consume_identifier("expected a binding name after 'carries'")?)
        } else {
            None
        };
        let then_branch = Box::new(self.statement()?);
        let else_branch = if self.matches(&[TokenKind::Else]) {
            Some(Box::new(self.statement()?))
        } else {
            None
        };
        Ok(match binding {
            Some(binding) => Stmt::IfCarries {
                subject: condition,
                binding,
                then_branch,
                else_branch,
                span,
            },
            None => Stmt::If {
                condition,
                then_branch,
                else_branch,
                span,
            },
        })
    }

    fn while_statement(&mut self) -> Result<Stmt, NivError> {
        let span = self.previous_span();
        self.matches(&[TokenKind::WhileClause]);
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

    fn using_statement(&mut self) -> Result<Stmt, NivError> {
        let span = self.previous_span();
        let name = self.consume_identifier("expected resource name after using")?;
        if !self.matches(&[TokenKind::Set, TokenKind::Equal]) {
            return Err(self.error_here("a scoped resource uses 'set'"));
        }
        let resource = self.expression()?;
        let body = Box::new(self.statement()?);
        Ok(Stmt::Using {
            name,
            resource,
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
        self.enter_nested_syntax()?;
        let result = self.assignment();
        self.depth -= 1;
        result
    }

    fn assignment(&mut self) -> Result<Expr, NivError> {
        self.enter_nested_syntax()?;
        let result = self.assignment_inner();
        self.depth -= 1;
        result
    }

    fn assignment_inner(&mut self) -> Result<Expr, NivError> {
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
        self.enter_nested_syntax()?;
        let result = self.coalesce_inner();
        self.depth -= 1;
        result
    }

    fn coalesce_inner(&mut self) -> Result<Expr, NivError> {
        let expression = self.pipeline()?;
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

    fn pipeline(&mut self) -> Result<Expr, NivError> {
        let mut expression = self.or()?;
        while self.matches(&[TokenKind::Through]) {
            let span = self.previous_span();
            let stage = self.call()?;
            if !matches!(
                stage,
                Expr::Call(_, _, _, _) | Expr::Variable(_, _) | Expr::Get(_, _, _)
            ) {
                return Err(NivError::new(
                    "through expects a function or function call",
                    span.line,
                    span.column,
                ));
            }
            expression = Expr::Through(Box::new(expression), Box::new(stage), span);
        }
        Ok(expression)
    }

    fn or(&mut self) -> Result<Expr, NivError> {
        let mut expression = self.and()?;
        while self.matches(&[TokenKind::Or]) {
            if self.matches(&[TokenKind::Return]) {
                let span = self.previous_span();
                expression = Expr::Propagate(Box::new(expression), span);
                break;
            }
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
        self.enter_nested_syntax()?;
        let result = self.unary_inner();
        self.depth -= 1;
        result
    }

    fn unary_inner(&mut self) -> Result<Expr, NivError> {
        if self.matches(&[TokenKind::Perform]) {
            let span = self.previous_span();
            return Ok(Expr::Perform(Box::new(self.unary()?), span));
        }
        for (kind, operation) in [
            (TokenKind::Start, "spawn"),
            (TokenKind::Wait, "await"),
            (TokenKind::Together, "all"),
            (TokenKind::Race, "race"),
        ] {
            if self.matches(&[kind]) {
                let span = self.previous_span();
                let argument = self.unary()?;
                return Ok(task_call(operation, argument, span));
            }
        }
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
            if self.matches(&[TokenKind::With]) {
                let span = self.previous_span();
                let (labels, args) = self.intent_arguments()?;
                if let Some(name) = expression_path(&expression) {
                    self.validate_labels(&name, &labels, span)?;
                }
                expression = Expr::Call(Box::new(expression), args, Some(labels), span);
            } else if self.matches(&[TokenKind::LeftParen]) {
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
                expression = Expr::Call(Box::new(expression), args, None, span);
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
            let edition_four = self.matches(&[TokenKind::Case]);
            let variant = self.consume_identifier("expected case name")?;
            let binding = if self.matches(&[TokenKind::Carries]) {
                Some(self.consume_identifier("expected payload binding after 'carries'")?)
            } else if self.matches(&[TokenKind::LeftParen]) {
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
            if !edition_four
                && !self.matches(&[TokenKind::Comma, TokenKind::Semicolon])
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
            TokenKind::Set => {
                self.advance();
                Ok("set".into())
            }
            TokenKind::From => {
                self.advance();
                Ok("from".into())
            }
            TokenKind::As => {
                self.advance();
                Ok("as".into())
            }
            TokenKind::With => {
                self.advance();
                Ok("with".into())
            }
            TokenKind::Maybe => {
                self.advance();
                Ok("maybe".into())
            }
            TokenKind::Start => {
                self.advance();
                Ok("start".into())
            }
            _ => Err(self.error_here(message)),
        }
    }
    fn check_identifier_value(&self, expected: &str) -> bool {
        matches!(&self.peek().kind, TokenKind::Identifier(value) if value == expected)
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
    fn enter_nested_syntax(&mut self) -> Result<(), NivError> {
        if self.depth >= MAX_PARSE_DEPTH {
            return Err(self.error_here("syntax nesting exceeds the supported limit"));
        }
        self.depth += 1;
        Ok(())
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

fn value_span(value: &TypeRef) -> Span {
    match value {
        TypeRef::Named(_, span)
        | TypeRef::Applied(_, _, span)
        | TypeRef::Array(_, span)
        | TypeRef::Nullable(_, span)
        | TypeRef::Result(_, _, span) => *span,
    }
}

fn expression_path(expression: &Expr) -> Option<String> {
    match expression {
        Expr::Variable(name, _) => Some(name.clone()),
        Expr::Get(parent, name, _) => {
            let mut path = expression_path(parent)?;
            path.push('.');
            path.push_str(name);
            Some(path)
        }
        _ => None,
    }
}

fn validate_capability_need(
    capability: &str,
    boundary: Option<&str>,
    span: Span,
) -> Result<(), NivError> {
    const CAPABILITIES: &[&str] = &[
        "FileRead",
        "FileWrite",
        "Environment",
        "Time",
        "Process",
        "Network",
        "Task",
        "Channel",
        "Log",
        "Native",
        "Random",
    ];
    if !CAPABILITIES.contains(&capability) {
        return Err(NivError::new(
            format!(
                "unknown capability '{capability}'; expected one of {}",
                CAPABILITIES.join(", ")
            ),
            span.line,
            span.column,
        ));
    }
    let Some(boundary) = boundary else {
        return Ok(());
    };
    if boundary.is_empty() || boundary.len() > 1024 || boundary.chars().any(char::is_control) {
        return Err(NivError::new(
            "a capability boundary must be non-empty, at most 1024 bytes, and contain no control characters",
            span.line,
            span.column,
        ));
    }
    if capability == "Network"
        && (boundary.contains("://")
            || boundary.contains('/')
            || boundary.chars().any(char::is_whitespace))
    {
        return Err(NivError::new(
            "a Network boundary names a host such as \"api.example.com\", without a URL scheme or path",
            span.line,
            span.column,
        ));
    }
    Ok(())
}

fn task_call(operation: &str, argument: Expr, span: Span) -> Expr {
    let standard = Expr::Variable("std".into(), span);
    let task = Expr::Get(Box::new(standard), "tasks".into(), span);
    let function = Expr::Get(Box::new(task), operation.into(), span);
    Expr::Call(Box::new(function), vec![argument], None, span)
}

fn same_variant(left: &TokenKind, right: &TokenKind) -> bool {
    std::mem::discriminant(left) == std::mem::discriminant(right)
}

/// Reports whether a token after an identifier keeps that identifier inside
/// an ordinary expression, which disqualifies a contextual `stop`/`skip`.
fn expression_continues(kind: &TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::LeftParen
            | TokenKind::LeftBracket
            | TokenKind::Dot
            | TokenKind::Plus
            | TokenKind::Minus
            | TokenKind::Star
            | TokenKind::Slash
            | TokenKind::Percent
            | TokenKind::EqualEqual
            | TokenKind::BangEqual
            | TokenKind::Less
            | TokenKind::LessEqual
            | TokenKind::Greater
            | TokenKind::GreaterEqual
            | TokenKind::And
            | TokenKind::Or
            | TokenKind::Question
            | TokenKind::QuestionQuestion
            | TokenKind::Equal
            | TokenKind::FatArrow
            | TokenKind::Through
            | TokenKind::With
            | TokenKind::Set
            | TokenKind::To
            | TokenKind::Is
            | TokenKind::Colon
            | TokenKind::Arrow
    )
}
