use crate::ast::{Literal, Pattern, Span};
use crate::bytecode::{BYTECODE_VERSION, BytecodeArm, Chunk, Instruction, Op, verify};
use crate::error::NivError;
use crate::lexer::TokenKind;

const MAGIC: &[u8; 4] = b"NIVB";
const MAX_ITEMS: usize = 1_000_000;
const MAX_DEPTH: usize = 256;

pub fn encode(chunk: &Chunk) -> Result<Vec<u8>, NivError> {
    verify(chunk)?;
    let mut writer = Writer {
        bytes: MAGIC.to_vec(),
    };
    writer.chunk(chunk)?;
    Ok(writer.bytes)
}

pub fn decode(bytes: &[u8]) -> Result<Chunk, NivError> {
    if !bytes.starts_with(MAGIC) {
        return Err(bundle_error("invalid bundle magic"));
    }
    let mut reader = Reader {
        bytes,
        at: MAGIC.len(),
    };
    let chunk = reader.chunk(0)?;
    if reader.at != bytes.len() {
        return Err(bundle_error("trailing data after bytecode bundle"));
    }
    verify(&chunk)?;
    Ok(chunk)
}

struct Writer {
    bytes: Vec<u8>,
}

impl Writer {
    fn chunk(&mut self, chunk: &Chunk) -> Result<(), NivError> {
        self.u16(chunk.version);
        self.len(chunk.code.len())?;
        for instruction in &chunk.code {
            self.instruction(instruction)?;
        }
        Ok(())
    }

    fn instruction(&mut self, instruction: &Instruction) -> Result<(), NivError> {
        self.span(instruction.span)?;
        match &instruction.op {
            Op::Constant(value) => {
                self.u8(0);
                self.literal(value)?;
            }
            Op::Load(name) => {
                self.u8(1);
                self.string(name)?;
            }
            Op::Store(name) => {
                self.u8(2);
                self.string(name)?;
            }
            Op::Define { name, mutable } => {
                self.u8(3);
                self.string(name)?;
                self.u8(u8::from(*mutable));
            }
            Op::Pop => self.u8(4),
            Op::Unary(operator) => {
                self.u8(5);
                self.operator(operator)?;
            }
            Op::Binary(operator) => {
                self.u8(6);
                self.operator(operator)?;
            }
            Op::Jump(target) => {
                self.u8(7);
                self.len(*target)?;
            }
            Op::JumpIfFalse(target) => {
                self.u8(8);
                self.len(*target)?;
            }
            Op::Call(arity) => {
                self.u8(9);
                self.len(*arity)?;
            }
            Op::MakeArray(length) => {
                self.u8(10);
                self.len(*length)?;
            }
            Op::Index => self.u8(11),
            Op::Coalesce(target) => {
                self.u8(12);
                self.len(*target)?;
            }
            Op::Get(name) => {
                self.u8(13);
                self.string(name)?;
            }
            Op::Print => self.u8(14),
            Op::EnterScope => self.u8(15),
            Op::ExitScope => self.u8(16),
            Op::MakeFunction { name, params, body } => {
                self.u8(17);
                self.string(name)?;
                self.strings(params)?;
                self.chunk(body)?;
            }
            Op::Return => self.u8(18),
            Op::DefineRecord {
                name,
                fields,
                derives,
            } => {
                self.u8(19);
                self.string(name)?;
                self.len(fields.len())?;
                for (field, schema) in fields {
                    self.string(field)?;
                    self.string(schema)?;
                }
                self.strings(derives)?;
            }
            Op::DefineEnum {
                name,
                variants,
                payload_variants,
            } => {
                self.u8(20);
                self.string(name)?;
                self.strings(variants)?;
                self.strings(payload_variants)?;
            }
            Op::Match(arms) => {
                self.u8(21);
                self.len(arms.len())?;
                for arm in arms {
                    self.arm(arm)?;
                }
            }
            Op::DefineModule {
                name,
                body,
                exports,
            } => {
                self.u8(22);
                self.string(name)?;
                self.chunk(body)?;
                self.strings(exports)?;
            }
            Op::Iterate {
                name,
                pattern,
                body,
            } => {
                self.u8(23);
                self.string(name)?;
                match pattern {
                    Some(pattern) => {
                        self.u8(1);
                        self.pattern(pattern)?;
                    }
                    None => self.u8(0),
                }
                self.chunk(body)?;
            }
            Op::Using { name, body } => {
                self.u8(24);
                self.string(name)?;
                self.chunk(body)?;
            }
            Op::Propagate => self.u8(25),
            Op::DefineProtocol { name, members } => {
                self.u8(26);
                self.string(name)?;
                self.strings(members)?;
            }
            Op::AdoptProtocol {
                protocol,
                type_name,
                mappings,
            } => {
                self.u8(27);
                self.string(protocol)?;
                self.string(type_name)?;
                self.len(mappings.len())?;
                for (member, implementation) in mappings {
                    self.string(member)?;
                    self.string(implementation)?;
                }
            }
            Op::Prepare(plan_type) => {
                self.u8(28);
                self.string(plan_type)?;
            }
            Op::Perform => self.u8(29),
            Op::PerformCall(arity) => {
                self.u8(30);
                self.len(*arity)?;
            }
            Op::Repeat { condition, body } => {
                self.u8(31);
                self.chunk(condition)?;
                self.chunk(body)?;
            }
            Op::LoopExit { skip } => {
                self.u8(32);
                self.u8(u8::from(*skip));
            }
            Op::IfCarries {
                patterns,
                then_branch,
                else_branch,
            } => {
                self.u8(33);
                self.len(patterns.len())?;
                for pattern in patterns {
                    self.pattern(pattern)?;
                }
                self.chunk(then_branch)?;
                match else_branch {
                    Some(branch) => {
                        self.u8(1);
                        self.chunk(branch)?;
                    }
                    None => self.u8(0),
                }
            }
            Op::MakeText(length) => {
                self.u8(34);
                self.len(*length)?;
            }
            Op::DefinePattern { pattern } => {
                self.u8(35);
                self.pattern(pattern)?;
            }
            Op::Sample { title, body, shows } => {
                self.u8(36);
                self.string(title)?;
                self.chunk(body)?;
                match shows {
                    Some(expected) => {
                        self.u8(1);
                        self.string(expected)?;
                    }
                    None => self.u8(0),
                }
            }
        }
        Ok(())
    }

    fn arm(&mut self, arm: &BytecodeArm) -> Result<(), NivError> {
        self.len(arm.patterns.len())?;
        for pattern in &arm.patterns {
            self.pattern(pattern)?;
        }
        if let Some(guard) = &arm.guard {
            self.u8(1);
            self.chunk(guard)?;
        } else {
            self.u8(0);
        }
        self.span(arm.span)?;
        self.chunk(&arm.body)
    }

    fn pattern(&mut self, pattern: &Pattern) -> Result<(), NivError> {
        match pattern {
            Pattern::Any(span) => {
                self.u8(0);
                self.span(*span)
            }
            Pattern::Literal(literal, span) => {
                self.u8(1);
                self.span(*span)?;
                self.literal(literal)
            }
            Pattern::Name(name, span) => {
                self.u8(2);
                self.span(*span)?;
                self.string(name)
            }
            Pattern::Binding(name, span) => {
                self.u8(3);
                self.span(*span)?;
                self.string(name)
            }
            Pattern::Carries(name, inner, span) => {
                self.u8(4);
                self.span(*span)?;
                self.string(name)?;
                self.pattern(inner)
            }
            Pattern::Shape(name, fields, span) => {
                self.u8(5);
                self.span(*span)?;
                self.string(name)?;
                self.len(fields.len())?;
                for (field, sub_pattern) in fields {
                    self.string(field)?;
                    self.pattern(sub_pattern)?;
                }
                Ok(())
            }
        }
    }

    fn literal(&mut self, value: &Literal) -> Result<(), NivError> {
        match value {
            Literal::Int(value) => {
                self.u8(0);
                self.bytes.extend_from_slice(&value.to_le_bytes());
            }
            Literal::Float(value) => {
                self.u8(1);
                self.bytes.extend_from_slice(&value.to_bits().to_le_bytes());
            }
            Literal::String(value) => {
                self.u8(2);
                self.string(value)?;
            }
            Literal::Bool(value) => {
                self.u8(3);
                self.u8(u8::from(*value));
            }
            Literal::Null => self.u8(4),
        }
        Ok(())
    }

    fn operator(&mut self, operator: &TokenKind) -> Result<(), NivError> {
        let tag = match operator {
            TokenKind::Plus => 0,
            TokenKind::Minus => 1,
            TokenKind::Star => 2,
            TokenKind::Slash => 3,
            TokenKind::Percent => 4,
            TokenKind::Bang => 5,
            TokenKind::EqualEqual => 6,
            TokenKind::BangEqual => 7,
            TokenKind::Greater => 8,
            TokenKind::GreaterEqual => 9,
            TokenKind::Less => 10,
            TokenKind::LessEqual => 11,
            _ => return Err(bundle_error("invalid bytecode operator")),
        };
        self.u8(tag);
        Ok(())
    }

    fn strings(&mut self, values: &[String]) -> Result<(), NivError> {
        self.len(values.len())?;
        for value in values {
            self.string(value)?;
        }
        Ok(())
    }
    fn string(&mut self, value: &str) -> Result<(), NivError> {
        self.len(value.len())?;
        self.bytes.extend_from_slice(value.as_bytes());
        Ok(())
    }
    fn span(&mut self, span: Span) -> Result<(), NivError> {
        self.len(span.line)?;
        self.len(span.column)
    }
    fn len(&mut self, value: usize) -> Result<(), NivError> {
        let value =
            u32::try_from(value).map_err(|_| bundle_error("bundle value exceeds format limit"))?;
        self.bytes.extend_from_slice(&value.to_le_bytes());
        Ok(())
    }
    fn u16(&mut self, value: u16) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }
    fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn chunk(&mut self, depth: usize) -> Result<Chunk, NivError> {
        if depth > MAX_DEPTH {
            return Err(bundle_error("bundle nesting limit exceeded"));
        }
        let version = self.u16()?;
        if version != BYTECODE_VERSION {
            return Err(bundle_error(format!(
                "unsupported bytecode version {version}"
            )));
        }
        let count = self.count()?;
        let mut code = Vec::with_capacity(count);
        for _ in 0..count {
            code.push(self.instruction(depth)?);
        }
        Ok(Chunk { version, code })
    }

    fn instruction(&mut self, depth: usize) -> Result<Instruction, NivError> {
        let span = self.span()?;
        let op = match self.u8()? {
            0 => Op::Constant(self.literal()?),
            1 => Op::Load(self.string()?),
            2 => Op::Store(self.string()?),
            3 => Op::Define {
                name: self.string()?,
                mutable: self.boolean()?,
            },
            4 => Op::Pop,
            5 => Op::Unary(self.operator()?),
            6 => Op::Binary(self.operator()?),
            7 => Op::Jump(self.count()?),
            8 => Op::JumpIfFalse(self.count()?),
            9 => Op::Call(self.count()?),
            10 => Op::MakeArray(self.count()?),
            11 => Op::Index,
            12 => Op::Coalesce(self.count()?),
            13 => Op::Get(self.string()?),
            14 => Op::Print,
            15 => Op::EnterScope,
            16 => Op::ExitScope,
            17 => Op::MakeFunction {
                name: self.string()?,
                params: self.strings()?,
                body: self.chunk(depth + 1)?,
            },
            18 => Op::Return,
            19 => Op::DefineRecord {
                name: self.string()?,
                fields: {
                    let count = self.count()?;
                    let mut fields = Vec::with_capacity(count);
                    for _ in 0..count {
                        fields.push((self.string()?, self.string()?));
                    }
                    fields
                },
                derives: self.strings()?,
            },
            20 => Op::DefineEnum {
                name: self.string()?,
                variants: self.strings()?,
                payload_variants: self.strings()?,
            },
            21 => {
                let count = self.count()?;
                let mut arms = Vec::with_capacity(count);
                for _ in 0..count {
                    arms.push(self.arm(depth + 1)?);
                }
                Op::Match(arms)
            }
            22 => Op::DefineModule {
                name: self.string()?,
                body: self.chunk(depth + 1)?,
                exports: self.strings()?,
            },
            23 => Op::Iterate {
                name: self.string()?,
                pattern: match self.u8()? {
                    0 => None,
                    1 => Some(self.pattern(depth + 1)?),
                    _ => return Err(bundle_error("invalid optional pattern")),
                },
                body: self.chunk(depth + 1)?,
            },
            24 => Op::Using {
                name: self.string()?,
                body: self.chunk(depth + 1)?,
            },
            25 => Op::Propagate,
            26 => Op::DefineProtocol {
                name: self.string()?,
                members: self.strings()?,
            },
            27 => Op::AdoptProtocol {
                protocol: self.string()?,
                type_name: self.string()?,
                mappings: {
                    let count = self.count()?;
                    let mut mappings = Vec::with_capacity(count);
                    for _ in 0..count {
                        mappings.push((self.string()?, self.string()?));
                    }
                    mappings
                },
            },
            28 => Op::Prepare(self.string()?),
            29 => Op::Perform,
            30 => Op::PerformCall(self.count()?),
            31 => Op::Repeat {
                condition: self.chunk(depth + 1)?,
                body: self.chunk(depth + 1)?,
            },
            32 => Op::LoopExit {
                skip: self.u8()? != 0,
            },
            33 => Op::IfCarries {
                patterns: {
                    let count = self.count()?;
                    if count == 0 {
                        return Err(bundle_error("'when … carries' needs at least one pattern"));
                    }
                    let mut patterns = Vec::with_capacity(count);
                    for _ in 0..count {
                        patterns.push(self.pattern(depth + 1)?);
                    }
                    patterns
                },
                then_branch: self.chunk(depth + 1)?,
                else_branch: if self.u8()? != 0 {
                    Some(self.chunk(depth + 1)?)
                } else {
                    None
                },
            },
            34 => Op::MakeText(self.count()?),
            35 => Op::DefinePattern {
                pattern: self.pattern(depth + 1)?,
            },
            36 => Op::Sample {
                title: self.string()?,
                body: self.chunk(depth + 1)?,
                shows: if self.u8()? != 0 {
                    Some(self.string()?)
                } else {
                    None
                },
            },
            _ => return Err(bundle_error("unknown bytecode instruction")),
        };
        Ok(Instruction { op, span })
    }

    fn arm(&mut self, depth: usize) -> Result<BytecodeArm, NivError> {
        let pattern_count = self.count()?;
        if pattern_count == 0 {
            return Err(bundle_error("a choose arm needs at least one pattern"));
        }
        let mut patterns = Vec::with_capacity(pattern_count);
        for _ in 0..pattern_count {
            patterns.push(self.pattern(depth)?);
        }
        let guard = match self.u8()? {
            0 => None,
            1 => Some(self.chunk(depth)?),
            _ => return Err(bundle_error("invalid optional guard")),
        };
        let span = self.span()?;
        let body = self.chunk(depth)?;
        Ok(BytecodeArm {
            patterns,
            guard,
            body,
            span,
        })
    }

    fn pattern(&mut self, depth: usize) -> Result<Pattern, NivError> {
        if depth > MAX_DEPTH {
            return Err(bundle_error("pattern nesting is too deep"));
        }
        Ok(match self.u8()? {
            0 => Pattern::Any(self.span()?),
            1 => {
                let span = self.span()?;
                Pattern::Literal(self.literal()?, span)
            }
            2 => {
                let span = self.span()?;
                Pattern::Name(self.string()?, span)
            }
            3 => {
                let span = self.span()?;
                Pattern::Binding(self.string()?, span)
            }
            4 => {
                let span = self.span()?;
                let name = self.string()?;
                Pattern::Carries(name, Box::new(self.pattern(depth + 1)?), span)
            }
            5 => {
                let span = self.span()?;
                let name = self.string()?;
                let count = self.count()?;
                let mut fields = Vec::with_capacity(count);
                for _ in 0..count {
                    let field = self.string()?;
                    fields.push((field, self.pattern(depth + 1)?));
                }
                Pattern::Shape(name, fields, span)
            }
            _ => return Err(bundle_error("unknown pattern kind")),
        })
    }

    fn literal(&mut self) -> Result<Literal, NivError> {
        Ok(match self.u8()? {
            0 => Literal::Int(i64::from_le_bytes(self.array()?)),
            1 => Literal::Float(f64::from_bits(u64::from_le_bytes(self.array()?))),
            2 => Literal::String(self.string()?),
            3 => Literal::Bool(self.boolean()?),
            4 => Literal::Null,
            _ => return Err(bundle_error("unknown bytecode literal")),
        })
    }
    fn operator(&mut self) -> Result<TokenKind, NivError> {
        Ok(match self.u8()? {
            0 => TokenKind::Plus,
            1 => TokenKind::Minus,
            2 => TokenKind::Star,
            3 => TokenKind::Slash,
            4 => TokenKind::Percent,
            5 => TokenKind::Bang,
            6 => TokenKind::EqualEqual,
            7 => TokenKind::BangEqual,
            8 => TokenKind::Greater,
            9 => TokenKind::GreaterEqual,
            10 => TokenKind::Less,
            11 => TokenKind::LessEqual,
            _ => return Err(bundle_error("unknown bytecode operator")),
        })
    }
    fn strings(&mut self) -> Result<Vec<String>, NivError> {
        let count = self.count()?;
        let mut values = Vec::with_capacity(count);
        for _ in 0..count {
            values.push(self.string()?);
        }
        Ok(values)
    }
    fn string(&mut self) -> Result<String, NivError> {
        let length = self.count()?;
        let bytes = self.take(length)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| bundle_error("bundle contains invalid UTF-8"))
    }
    fn span(&mut self) -> Result<Span, NivError> {
        Ok(Span {
            line: self.count()?,
            column: self.count()?,
        })
    }
    fn boolean(&mut self) -> Result<bool, NivError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(bundle_error("invalid boolean")),
        }
    }
    fn count(&mut self) -> Result<usize, NivError> {
        let value = u32::from_le_bytes(self.array()?) as usize;
        if value > MAX_ITEMS {
            Err(bundle_error("bundle allocation limit exceeded"))
        } else {
            Ok(value)
        }
    }
    fn u16(&mut self) -> Result<u16, NivError> {
        Ok(u16::from_le_bytes(self.array()?))
    }
    fn u8(&mut self) -> Result<u8, NivError> {
        Ok(self.take(1)?[0])
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], NivError> {
        self.take(N)?
            .try_into()
            .map_err(|_| bundle_error("truncated bytecode bundle"))
    }
    fn take(&mut self, length: usize) -> Result<&[u8], NivError> {
        let end = self
            .at
            .checked_add(length)
            .ok_or_else(|| bundle_error("bundle length overflow"))?;
        let value = self
            .bytes
            .get(self.at..end)
            .ok_or_else(|| bundle_error("truncated bytecode bundle"))?;
        self.at = end;
        Ok(value)
    }
}

fn bundle_error(message: impl Into<String>) -> NivError {
    NivError::new(message, 1, 1)
}
