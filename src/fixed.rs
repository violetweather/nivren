#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FixedKind {
    I8,
    I16,
    I32,
    U8,
    U16,
    U32,
    U64,
}

impl FixedKind {
    pub const fn name(self) -> &'static str {
        match self {
            Self::I8 => "I8",
            Self::I16 => "I16",
            Self::I32 => "I32",
            Self::U8 => "U8",
            Self::U16 => "U16",
            Self::U32 => "U32",
            Self::U64 => "U64",
        }
    }

    pub const fn minimum(self) -> i128 {
        match self {
            Self::I8 => i8::MIN as i128,
            Self::I16 => i16::MIN as i128,
            Self::I32 => i32::MIN as i128,
            Self::U8 | Self::U16 | Self::U32 | Self::U64 => 0,
        }
    }

    pub const fn maximum(self) -> i128 {
        match self {
            Self::I8 => i8::MAX as i128,
            Self::I16 => i16::MAX as i128,
            Self::I32 => i32::MAX as i128,
            Self::U8 => u8::MAX as i128,
            Self::U16 => u16::MAX as i128,
            Self::U32 => u32::MAX as i128,
            Self::U64 => u64::MAX as i128,
        }
    }

    pub const fn signed(self) -> bool {
        matches!(self, Self::I8 | Self::I16 | Self::I32)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FixedInt {
    pub kind: FixedKind,
    pub value: i128,
}

impl FixedInt {
    pub fn new(kind: FixedKind, value: i128) -> Result<Self, String> {
        if value < kind.minimum() || value > kind.maximum() {
            return Err(format!("value is outside the {} range", kind.name()));
        }
        Ok(Self { kind, value })
    }
}
